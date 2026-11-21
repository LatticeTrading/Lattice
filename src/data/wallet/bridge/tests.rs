//! The bridge over a real loopback connection, with the test in the page's
//! place: it asks as the page's script does and answers as a wallet would.

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

use nsh_core::element::Root;
use nsh_engine::EngineConfig;
use nsh_testing::{Headless, MemoryStorage, WAIT};

use super::*;
use crate::data::{
    DataConfig, DataServices,
    ferris::FerrisConfig,
    wallet::{Phase, WalletConfig},
};

/// The account the stand-in wallet shares.
const ACCOUNT: &str = "0x5e9ee1089755c3435139848e47e6635505d5a13a";

/// How long [`Impatient`]'s bridge gives a quiet page.
const IMPATIENT: Duration = Duration::from_secs(1);

/// A Connect button over the real bridge, and a line saying how the
/// connection stands. The bridge gives a quiet page far longer than a test
/// takes, so only what the test does ends its session.
#[component]
fn Harness() -> impl IntoElement {
    over_bridge(Duration::from_secs(120))
}

/// [`Harness`] over a bridge that gives a quiet page up after
/// [`IMPATIENT`].
#[component]
fn Impatient() -> impl IntoElement {
    over_bridge(IMPATIENT)
}

fn over_bridge(grace: Duration) -> View {
    let services = DataServices::provide_with(DataConfig {
        ferris: FerrisConfig {
            http_base: "https://ferris.test".to_owned(),
            ws_url: "ws://127.0.0.1:9/v1/ws".to_owned(),
            ..FerrisConfig::production()
        },
        wallet: WalletConfig {
            connector: Some(Rc::new(Bridge { grace })),
        },
        ..DataConfig::default()
    });
    let wallet = services.wallet;
    let stands = {
        let wallet = wallet.clone();
        move || {
            wallet.with(|state| {
                let phase = match &state.phase {
                    Phase::Disconnected => "Disconnected".to_owned(),
                    Phase::Finding => "Finding".to_owned(),
                    Phase::Choosing(wallets) => format!("Choosing of {}", wallets.len()),
                    Phase::Approving(wallet) => format!("Approving {}", wallet.name),
                    Phase::Connected(account) => {
                        format!(
                            "Connected {} on {}",
                            account.address_text(),
                            account.chain_id
                        )
                    }
                };
                match &state.failure {
                    Some(failure) => format!("{phase}: {failure}"),
                    None => phase,
                }
            })
        }
    };
    view()
        .size_full()
        .flex_col()
        .child(
            view()
                .role(AccessibilityRole::Button)
                .label("Connect")
                .on_press(move || wallet.connect())
                .child(text("Connect")),
        )
        .child(text(stands))
}

/// The page's side of a session: where the bridge listens, and its secret.
#[derive(Clone)]
struct Page {
    port: u16,
    token: String,
}

impl Page {
    /// Connects, and reads where the app sent the user's browser.
    fn open(app: &mut Headless) -> Self {
        app.press("Connect").unwrap();
        app.wait_until("the page being opened", WAIT, |screen| {
            !screen.opened_urls().is_empty()
        })
        .unwrap();
        let url = app.screen().opened_urls()[0].clone();
        let rest = url.strip_prefix("http://127.0.0.1:").expect(&url);
        let (port, token) = rest.split_once('/').expect(&url);
        assert_eq!(token.len(), 32, "the secret is 128 bits in hexadecimal");
        Self {
            port: port.parse().unwrap(),
            token: token.to_owned(),
        }
    }

    fn host(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    /// `GET`s `action` under the secret.
    fn get(&self, action: &str) -> (u16, String) {
        send(
            self.port,
            "GET",
            &format!("/{}{action}", self.token),
            &self.host(),
            None,
        )
    }

    /// `POST`s `body` as JSON to `action` under the secret; the status.
    fn post(&self, action: &str, body: &Value) -> u16 {
        let path = format!("/{}{action}", self.token);
        send(
            self.port,
            "POST",
            &path,
            &self.host(),
            Some(("application/json", body)),
        )
        .0
    }

    /// The next requests for the page `page`: the status, and what is asked.
    fn next(&self, page: &str) -> (u16, Value) {
        let (status, body) = self.get(&format!("/next?page={page}"));
        (status, serde_json::from_str(&body).unwrap_or_default())
    }
}

/// One request, as a browser sends it, and its answer: the status and the
/// body.
fn send(
    port: u16,
    method: &str,
    path: &str,
    host: &str,
    body: Option<(&str, &Value)>,
) -> (u16, String) {
    try_send(port, method, path, host, body).expect("the bridge answers")
}

/// [`send`], or nothing where nothing answers (nobody listens, or the
/// session ended before it answered).
fn try_send(
    port: u16,
    method: &str,
    path: &str,
    host: &str,
    body: Option<(&str, &Value)>,
) -> Option<(u16, String)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(40)))
        .unwrap();
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\n");
    let text = body.map(|(kind, body)| {
        let text = body.to_string();
        request.push_str(&format!(
            "Content-Type: {kind}\r\nContent-Length: {}\r\n",
            text.len()
        ));
        text
    });
    request.push_str("\r\n");
    request.push_str(&text.unwrap_or_default());
    stream.write_all(request.as_bytes()).ok()?;
    let mut answer = String::new();
    stream.read_to_string(&mut answer).ok()?;
    let status = answer.get(9..12)?.parse().ok()?;
    let body = answer
        .split_once("\r\n\r\n")
        .map_or_else(String::new, |(_, body)| body.to_owned());
    Some((status, body))
}

fn start() -> Headless {
    start_root(Root::of::<Harness>())
}

fn start_root(root: Root) -> Headless {
    let mut config = EngineConfig::new(root);
    config.storage = Some(MemoryStorage::new([]));
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Disconnected").unwrap();
    app
}

/// Says hello as the page `page` that found one wallet.
fn hello(page: &Page, id: &str) {
    let found = json!({ "page": id, "wallets": [{ "id": "w1", "name": "Test Wallet" }] });
    assert_eq!(page.post("/hello", &found), 204);
}

#[test]
fn the_page_carries_the_apps_requests_to_the_wallet_and_its_answers_back() {
    let mut app = start();
    let page = Page::open(&mut app);
    app.wait_for("Finding").unwrap();

    let (status, html) = page.get("");
    assert_eq!(status, 200);
    assert!(html.contains("eip6963:requestProvider"));

    // One wallet: it is asked for its accounts at once, then for its chain.
    hello(&page, "p1");
    app.wait_for("Approving Test Wallet").unwrap();
    let (status, asked) = page.next("p1");
    assert_eq!(status, 200);
    assert_eq!(
        asked,
        json!([{ "id": 1, "wallet": "w1", "method": "eth_requestAccounts", "params": [] }])
    );
    assert_eq!(
        page.post("/reply", &json!({ "id": 1, "result": [ACCOUNT] })),
        204
    );
    let (_, asked) = page.next("p1");
    assert_eq!(
        asked,
        json!([{ "id": 2, "wallet": "w1", "method": "eth_chainId", "params": [] }])
    );
    assert_eq!(
        page.post("/reply", &json!({ "id": 2, "result": "0xa4b1" })),
        204
    );
    app.wait_for(&format!("Connected {ACCOUNT} on 42161"))
        .unwrap();
    // An answer nobody asked for is no request's.
    assert_eq!(
        page.post("/reply", &json!({ "id": 2, "result": "0x1" })),
        404
    );

    // What the wallet says by itself reaches the app: another chain, and
    // then the user disconnecting the site there.
    let chain = json!({ "wallet": "w1", "name": "chainChanged", "data": "0x1" });
    assert_eq!(page.post("/event", &chain), 204);
    app.wait_for(&format!("Connected {ACCOUNT} on 1")).unwrap();
    let gone = json!({ "wallet": "w1", "name": "accountsChanged", "data": [] });
    // The session ends with it, and may end before it is answered.
    let path = format!("/{}/event", page.token);
    let _ = try_send(
        page.port,
        "POST",
        &path,
        &page.host(),
        Some(("application/json", &gone)),
    );
    app.wait_for("Disconnected: Test Wallet disconnected Lattice.")
        .unwrap();
}

#[test]
fn only_the_page_with_the_secret_is_answered() {
    let mut app = start();
    let page = Page::open(&mut app);
    let under = |token: &str, action: &str| {
        send(
            page.port,
            "GET",
            &format!("/{token}{action}"),
            &page.host(),
            None,
        )
        .0
    };
    // No secret, a wrong one, and a right one's beginning.
    assert_eq!(send(page.port, "GET", "/", &page.host(), None).0, 404);
    assert_eq!(under(&"0".repeat(32), ""), 404);
    assert_eq!(under(&page.token[..31], ""), 404);
    assert_eq!(under(&page.token, "/nothing"), 404);
    assert_eq!(under(&page.token, "/hello"), 405);

    // Another site's name for this machine (DNS rebinding) is refused,
    // secret or not; this machine's own names are taken.
    let path = format!("/{}", page.token);
    assert_eq!(send(page.port, "GET", &path, "evil.example", None).0, 403);
    let wrong_port = format!("127.0.0.1:{}", page.port.wrapping_add(1));
    assert_eq!(send(page.port, "GET", &path, &wrong_port, None).0, 403);
    let local = format!("localhost:{}", page.port);
    assert_eq!(send(page.port, "GET", &path, &local, None).0, 200);

    // A body must be declared JSON (what another site's page cannot send
    // unasked) and be JSON of the right shape.
    let hello = json!({ "page": "p1", "wallets": [] });
    let path = format!("/{}/hello", page.token);
    let plain = Some(("text/plain", &hello));
    assert_eq!(send(page.port, "POST", &path, &page.host(), plain).0, 400);
    assert_eq!(page.post("/hello", &json!({ "wallets": "none" })), 400);
    assert_eq!(page.post("/reply", &json!({ "result": 1 })), 400);
    // Nothing of that made a page or a wallet.
    app.wait_for("Finding").unwrap();
    assert_eq!(page.next("p1").0, 409);
}

#[test]
fn a_reloaded_page_takes_over_and_is_asked_again() {
    let mut app = start();
    let page = Page::open(&mut app);
    hello(&page, "p1");
    app.wait_for("Approving Test Wallet").unwrap();
    let (_, asked) = page.next("p1");
    assert_eq!(asked[0]["method"], "eth_requestAccounts");

    // The tab is reloaded before the wallet answered: the new page is the
    // one asked, and is asked what the old one never answered.
    hello(&page, "p2");
    assert_eq!(page.next("p1").0, 409);
    let (status, again) = page.next("p2");
    assert_eq!(status, 200);
    assert_eq!(again, asked);

    // The user turns it down there: the wallet is offered again.
    let rejected = json!({ "id": 1, "error": { "code": 4001, "message": "User rejected" } });
    assert_eq!(page.post("/reply", &rejected), 204);
    app.wait_for("Choosing of 1: Rejected in the wallet.")
        .unwrap();
}

#[test]
fn a_closed_page_ends_the_session() {
    let mut app = start_root(Root::of::<Impatient>());
    let page = Page::open(&mut app);
    // The page says hello and never asks what there is to do: its tab was
    // closed.
    hello(&page, "p1");
    app.wait_until("the page counting as closed", WAIT, |screen| {
        screen
            .texts()
            .contains(&"Disconnected: The wallet page was closed.")
    })
    .unwrap();
    // Nothing serves the page any more: nobody listens there (which Windows
    // takes two seconds to say, unless told how long to try), or another
    // test's bridge does by now, which knows nothing of this secret.
    let there = SocketAddr::from(([127, 0, 0, 1], page.port));
    if TcpStream::connect_timeout(&there, Duration::from_millis(250)).is_ok() {
        let path = format!("/{}", page.token);
        let served = try_send(page.port, "GET", &path, &page.host(), None);
        assert_ne!(served.map(|(status, _)| status), Some(200));
    }
}

#[test]
fn a_page_waiting_to_be_asked_is_not_a_closed_one() {
    let mut app = start_root(Root::of::<Impatient>());
    let page = Page::open(&mut app);
    hello(&page, "p1");
    assert_eq!(page.next("p1").1[0]["method"], "eth_requestAccounts");
    let accounts = json!({ "id": 1, "result": [ACCOUNT] });
    assert_eq!(page.post("/reply", &accounts), 204);
    assert_eq!(page.next("p1").1[0]["method"], "eth_chainId");
    let chain = json!({ "id": 2, "result": "0xa4b1" });
    assert_eq!(page.post("/reply", &chain), 204);

    // Nothing is asked now, so the page's poll is held, as a tab left open
    // behind the app holds one for as long as the wallet is connected.
    let held = std::thread::spawn({
        let page = page.clone();
        move || page.next("p1").0
    });
    let connected = format!("Connected {ACCOUNT} on 42161");
    app.wait_for(&connected).unwrap();
    // Longer than a quiet page is given, and the session stands.
    app.wait(IMPATIENT * 2 + Duration::from_millis(500))
        .unwrap();
    assert!(app.shows(&connected));

    // Another tab takes over: the held poll ends, and is told so.
    hello(&page, "p2");
    assert_eq!(held.join().unwrap(), 409);
}
