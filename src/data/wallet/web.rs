//! The web's way to the wallets: the page's own.
//!
//! - Wallets announce themselves (EIP-6963: the page dispatches
//!   `eip6963:requestProvider` and each wallet answers with an
//!   `eip6963:announceProvider` carrying its name and its provider), so two
//!   wallets are two entries instead of a fight over `window.ethereum`.
//!   What announces within [`SETTLE`] is the first list; one that comes
//!   later is another list. A browser where nothing announces but
//!   `window.ethereum` is there has that one wallet ([`INJECTED`]).
//! - A request is the provider's `request({ method, params })`, each in a
//!   task of its own: a signature waiting for the user must not hold up a
//!   read.
//! - The browser's callbacks (an announcement, `accountsChanged`,
//!   `chainChanged`) only queue what they heard and wake the task that
//!   hands it on, so the app sees it inside the engine's run like any other
//!   task wake, as the browser WebSocket does (`net/socket/web.rs`).
//!
//! Values cross between Rust and the page as JSON text (`JSON.parse`,
//! `JSON.stringify`): what a wallet takes and gives is JSON.

use std::{pin::pin, time::Duration};

use js_sys::{Function, JSON, Object, Promise, Reflect};
use nsh_core::prelude::*;
use serde_json::Value;
use tokio::sync::mpsc;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::JsFuture;
use web_sys::{CustomEvent, Window};

use super::{Call, Connector, Event, Events, Link, Reach, Refusal, Session, WalletInfo};
use crate::data::net::{TaskHandle, TaskSet};

/// What a wallet announces itself with, and what asks it to.
const ANNOUNCE: &str = "eip6963:announceProvider";
const REQUEST: &str = "eip6963:requestProvider";
/// How long wallets are given to announce themselves before the app is told
/// which there are.
const SETTLE: Duration = Duration::from_millis(400);
/// The id of the wallet that is `window.ethereum` and announced nothing.
const INJECTED: &str = "injected";

/// The wallets of the page the app runs in.
pub(super) struct PageWallets;

impl Connector for PageWallets {
    fn reach(&self) -> Reach {
        Reach::Page
    }

    fn open(&self, events: Events) -> Result<Session, String> {
        let window = web_sys::window().ok_or("This page has no window to find a wallet in.")?;
        let (link, calls) = Link::new();
        let (said, heard) = mpsc::unbounded_channel();
        let announced: Closure<dyn FnMut(web_sys::Event)> = Closure::new({
            let said = said.clone();
            move |event: web_sys::Event| {
                if let Some(wallet) = announcement(&event) {
                    let _ = said.send(wallet);
                }
            }
        });
        window
            .add_event_listener_with_callback(ANNOUNCE, announced.as_ref().unchecked_ref())
            .map_err(|error| refusal(&error).message)?;
        // The wallets answer inside the dispatch or soon after it.
        web_sys::Event::new(REQUEST)
            .and_then(|request| window.dispatch_event(&request))
            .map_err(|error| refusal(&error).message)?;
        let task = TaskHandle::spawn(run(window.clone(), calls, heard, said, events));
        Ok(Session::new(
            link,
            Running {
                window,
                announced,
                _task: task,
            },
        ))
    }
}

/// The page's wallets while their session lasts.
struct Running {
    window: Window,
    announced: Closure<dyn FnMut(web_sys::Event)>,
    _task: TaskHandle,
}

impl Drop for Running {
    fn drop(&mut self) {
        // Detached first: the closure is freed with this.
        let _ = self
            .window
            .remove_event_listener_with_callback(ANNOUNCE, self.announced.as_ref().unchecked_ref());
    }
}

/// What the browser's callbacks queue for [`run`].
enum Heard {
    Announced {
        id: String,
        name: String,
        provider: JsValue,
    },
    Accounts {
        wallet: String,
        accounts: Value,
    },
    Chain {
        wallet: String,
        chain: Value,
    },
}

/// A wallet of the page: who it is, its provider, and what listens to it.
struct Found {
    info: WalletInfo,
    provider: JsValue,
    _listeners: Option<Listeners>,
}

/// Hands on what the callbacks heard and answers `calls`, until the
/// session drops.
async fn run(
    window: Window,
    mut calls: mpsc::UnboundedReceiver<Call>,
    mut heard: mpsc::UnboundedReceiver<Heard>,
    said: mpsc::UnboundedSender<Heard>,
    events: Events,
) {
    let mut wallets: Vec<Found> = Vec::new();
    let asking = TaskSet::default();
    let mut settled = false;
    let mut settle = pin!(timer(SETTLE));
    let listed = |wallets: &[Found]| {
        Event::Wallets(wallets.iter().map(|wallet| wallet.info.clone()).collect())
    };
    loop {
        tokio::select! {
            biased;
            news = heard.recv() => match news {
                Some(Heard::Announced { id, name, provider }) => {
                    if wallets.iter().any(|wallet| wallet.info.id == id) {
                        continue;
                    }
                    wallets.push(Found {
                        _listeners: Listeners::on(&provider, &id, &said),
                        info: WalletInfo { id, name },
                        provider,
                    });
                    // One that announces itself late is news to the app.
                    if settled {
                        events(listed(&wallets));
                    }
                }
                Some(Heard::Accounts { wallet, accounts }) => {
                    events(Event::Accounts { wallet, accounts });
                }
                Some(Heard::Chain { wallet, chain }) => events(Event::Chain { wallet, chain }),
                None => return,
            },
            call = calls.recv() => {
                let Some(Call { wallet, method, params, reply }) = call else {
                    return;
                };
                let provider = wallets
                    .iter()
                    .find(|found| found.info.id == wallet)
                    .map(|found| found.provider.clone());
                asking.spawn(async move {
                    let answer = match provider {
                        Some(provider) => ask(&provider, &method, &params).await,
                        None => Err(Refusal::new(
                            Refusal::DISCONNECTED,
                            "That wallet is not in this page any more.",
                        )),
                    };
                    // Nobody waiting is nothing to do about.
                    let _ = reply.send(answer);
                });
            },
            () = settle.as_mut(), if !settled => {
                settled = true;
                // A wallet too old to announce itself is still
                // `window.ethereum`.
                if wallets.is_empty()
                    && let Some(provider) = member(&window, "ethereum").filter(JsValue::is_object)
                {
                    wallets.push(Found {
                        _listeners: Listeners::on(&provider, INJECTED, &said),
                        info: WalletInfo {
                            id: INJECTED.to_owned(),
                            name: "Browser wallet".to_owned(),
                        },
                        provider,
                    });
                }
                events(listed(&wallets));
            }
        }
    }
}

/// The wallet an `eip6963:announceProvider` announces.
fn announcement(event: &web_sys::Event) -> Option<Heard> {
    let detail = event.dyn_ref::<CustomEvent>()?.detail();
    let info = member(&detail, "info")?;
    let provider = member(&detail, "provider").filter(JsValue::is_object)?;
    // The name it registered (`rdns`, "io.rabby") is the same on every page
    // load, as the desktop's page needs it; the announcement's own id else.
    let named = |name: &str| {
        member(&info, name)
            .and_then(|value| value.as_string())
            .filter(|value| !value.is_empty())
    };
    Some(Heard::Announced {
        id: named("rdns").or_else(|| named("uuid"))?,
        name: member(&info, "name")
            .and_then(|name| name.as_string())
            .unwrap_or_else(|| "Wallet".to_owned()),
        provider,
    })
}

/// One EIP-1193 request of `provider`, waited for however long its user
/// takes.
async fn ask(provider: &JsValue, method: &str, params: &Value) -> Result<Value, Refusal> {
    let request: Function = member(provider, "request")
        .and_then(|request| request.dyn_into().ok())
        .ok_or_else(|| Refusal::new(Refusal::UNSUPPORTED, "The wallet takes no requests."))?;
    let arguments = Object::new();
    let set = |name: &str, value: &JsValue| {
        Reflect::set(&arguments, &JsValue::from_str(name), value)
            .map(drop)
            .map_err(|error| refusal(&error))
    };
    set("method", &JsValue::from_str(method))?;
    set(
        "params",
        &JSON::parse(&params.to_string()).map_err(|error| refusal(&error))?,
    )?;
    let asked = request
        .call1(provider, &arguments)
        .map_err(|error| refusal(&error))?;
    // Whatever thenable the wallet returns is waited for as a promise.
    let answered = JsFuture::from(Promise::resolve(&asked))
        .await
        .map_err(|error| refusal(&error))?;
    Ok(json(&answered))
}

/// `provider`'s `accountsChanged` and `chainChanged`, queued as they come.
/// Dropping it stops listening.
struct Listeners {
    provider: JsValue,
    accounts: Closure<dyn FnMut(JsValue)>,
    chain: Closure<dyn FnMut(JsValue)>,
}

impl Listeners {
    /// Listens to the wallet `id`; `None` for a provider that tells nothing
    /// (no `on`).
    fn on(provider: &JsValue, id: &str, said: &mpsc::UnboundedSender<Heard>) -> Option<Self> {
        let on: Function = member(provider, "on")?.dyn_into().ok()?;
        let accounts: Closure<dyn FnMut(JsValue)> = Closure::new({
            let (said, wallet) = (said.clone(), id.to_owned());
            move |accounts: JsValue| {
                let _ = said.send(Heard::Accounts {
                    wallet: wallet.clone(),
                    accounts: json(&accounts),
                });
            }
        });
        let chain: Closure<dyn FnMut(JsValue)> = Closure::new({
            let (said, wallet) = (said.clone(), id.to_owned());
            move |chain: JsValue| {
                let _ = said.send(Heard::Chain {
                    wallet: wallet.clone(),
                    chain: json(&chain),
                });
            }
        });
        on.call2(provider, &"accountsChanged".into(), accounts.as_ref())
            .ok()?;
        on.call2(provider, &"chainChanged".into(), chain.as_ref())
            .ok()?;
        Some(Self {
            provider: provider.clone(),
            accounts,
            chain,
        })
    }
}

impl Drop for Listeners {
    fn drop(&mut self) {
        // Detached first: the closures are freed with this.
        let Some(remove) = member(&self.provider, "removeListener")
            .and_then(|remove| remove.dyn_into::<Function>().ok())
        else {
            return;
        };
        let _ = remove.call2(
            &self.provider,
            &"accountsChanged".into(),
            self.accounts.as_ref(),
        );
        let _ = remove.call2(&self.provider, &"chainChanged".into(), self.chain.as_ref());
    }
}

/// `object`'s member `name`, when it has one.
fn member(object: &JsValue, name: &str) -> Option<JsValue> {
    Reflect::get(object, &JsValue::from_str(name))
        .ok()
        .filter(|value| !value.is_undefined() && !value.is_null())
}

/// A value of the page as JSON. What JSON cannot say (`undefined`, a
/// function) is `null`.
fn json(value: &JsValue) -> Value {
    JSON::stringify(value)
        .ok()
        .and_then(|text| text.as_string())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null)
}

/// What a wallet threw or rejected with, as EIP-1193 shapes it: an error
/// with a `code`, a `message` and perhaps `data`.
fn refusal(error: &JsValue) -> Refusal {
    // Whole numbers that are an error code fit an `i64` exactly.
    #[expect(clippy::cast_possible_truncation)]
    let code = member(error, "code")
        .and_then(|code| code.as_f64())
        .filter(|code| code.fract() == 0.0 && code.abs() < 1e15)
        .map_or(Refusal::INTERNAL, |code| code as i64);
    let message = member(error, "message")
        .and_then(|message| message.as_string())
        .or_else(|| error.as_string())
        .unwrap_or_else(|| "The wallet refused the request.".to_owned());
    let data = member(error, "data")
        .map(|data| json(&data))
        .filter(|data| !data.is_null());
    Refusal {
        code,
        message,
        data,
    }
}
