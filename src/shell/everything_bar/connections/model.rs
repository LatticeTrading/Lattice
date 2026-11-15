//! What the connection panel says, worked out from the sources as they
//! stand ([`SourceDetails`]): each source's heading, each feed's row, the
//! counts in the header and the report. No views here, so it is tested as
//! plain values. The wording and the rules are the old app's
//! (`ConnectionStatusPopover`'s `feedSeverity`, `feedState` and
//! `consumersLabel`, and `ConnectionStatusPolicy`).

use std::{collections::HashMap, rc::Rc, time::Duration};

use serde_json::json;

use crate::{
    data::net::{
        Consumer, Failure, Feed, FeedState, Health, Held, Link, LinkState, SourceDetails,
        TopicPhase,
    },
    exchanges, format,
    theme::{self, Role},
};

/// Rows a page of the panel shows (the old `topicPageSize`).
pub(super) const PAGE_SIZE: usize = 50;
/// Failed fetches in a row after which a request is an error rather than a
/// warning (the old `repeatedFailureCount`).
const REPEATED_FAILURES: u32 = 3;
/// How many of its refresh intervals old a request's value is before it
/// reads as stale.
const STALE_INTERVALS: f64 = 3.0;

/// The last page's number, from 0, for `rows` rows.
pub(super) fn last_page(rows: usize) -> usize {
    rows.saturating_sub(1) / PAGE_SIZE
}

/// How something is doing, as the panel colors it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Severity {
    /// Nothing is asked of it.
    #[default]
    Neutral,
    Healthy,
    Warning,
    Error,
}

impl Severity {
    pub fn of(health: Health) -> Self {
        match health {
            Health::Idle => Self::Neutral,
            Health::Healthy => Self::Healthy,
            Health::Pending | Health::Degraded => Self::Warning,
            Health::Failing => Self::Error,
        }
    }

    /// The color it is drawn in.
    pub fn role(self) -> Role {
        match self {
            Self::Neutral => theme::TEXT_FAINT,
            Self::Healthy => theme::SUCCESS,
            Self::Warning => theme::WARNING,
            Self::Error => theme::DANGER,
        }
    }

    /// What the panel's header calls it.
    pub fn title(self) -> &'static str {
        match self {
            Self::Neutral => "Idle",
            Self::Healthy => "Good",
            Self::Warning => "Degraded",
            Self::Error => "Error",
        }
    }

    /// Its name for assistive tech (`Connection status: healthy`).
    pub fn word(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Healthy => "healthy",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// One source's heading and what it says under it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SourceView {
    pub id: &'static str,
    pub name: &'static str,
    /// `WS` or `HTTP`.
    pub protocol: &'static str,
    /// A socket's last keepalive round trip, in milliseconds.
    pub latency_ms: Option<u64>,
    pub severity: Severity,
    /// `Connected`, `Reconnecting`, `Up to date`, ...
    pub status: &'static str,
    /// Whether anything is asked of it.
    pub active: bool,
    /// What is wrong with it, while something is: lines in the warning
    /// color under its heading.
    pub notices: Vec<String>,
    /// Quiet lines after its feeds.
    pub notes: Vec<String>,
}

/// The part of a feed's row that stays while the feed does: what it is and
/// who wants it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Row {
    /// Its source's id and its key there.
    pub id: Rc<str>,
    /// Which source it is under, by position.
    pub source: usize,
    /// The exchange id, empty when it is no one exchange's.
    pub exchange: String,
    pub symbol: String,
    pub description: String,
    pub consumers: String,
}

/// The part of a feed's row that moves: how it is doing.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct RowState {
    pub severity: Severity,
    pub label: &'static str,
    /// When its data last came, in Unix milliseconds.
    pub activity_ms: Option<f64>,
    pub failure: Option<Failure>,
    pub note: Option<String>,
}

impl RowState {
    /// What its tooltip says: its failure, else its note, else nothing.
    pub fn hint(&self) -> String {
        match (&self.failure, &self.note) {
            (Some(failure), _) => failure_line(failure),
            (None, Some(note)) => note.clone(),
            (None, None) => String::new(),
        }
    }
}

/// Everything the panel shows, as of one moment.
pub(super) struct Snapshot {
    /// When it was taken, in Unix milliseconds.
    pub captured_ms: f64,
    /// The worst source's.
    pub severity: Severity,
    pub sources: Vec<SourceView>,
    /// Every feed, in the order the panel lists them: by source, exchange,
    /// symbol and description.
    pub rows: Rc<Vec<Row>>,
    pub states: HashMap<Rc<str>, RowState>,
    /// What it was made from, for the report.
    details: Vec<SourceDetails>,
}

impl Snapshot {
    /// The panel's content for `details`. `names` is what each widget is
    /// called, `now_ms` the wall clock, and `time` writes a Unix time in
    /// milliseconds as the bar's clock does.
    pub fn of(
        details: Vec<SourceDetails>,
        names: &HashMap<Consumer, &'static str>,
        now_ms: f64,
        time: &dyn Fn(f64) -> String,
    ) -> Self {
        let sources: Vec<SourceView> = details
            .iter()
            .map(|source| source_view(source, time))
            .collect();
        let mut rows = Vec::new();
        let mut states = HashMap::new();
        for (index, source) in details.iter().enumerate() {
            let available = available(source);
            for feed in &source.feeds {
                let id: Rc<str> = Rc::from(format!("{}:{}", source.status.id, feed.key));
                rows.push(Row {
                    id: id.clone(),
                    source: index,
                    exchange: feed.subject.exchange.clone(),
                    symbol: feed.subject.name.clone(),
                    description: description(feed),
                    consumers: consumers_label(&feed.held, names),
                });
                states.insert(id, row_state(feed, available, now_ms));
            }
        }
        rows.sort_by(|a, b| {
            (a.source, &a.exchange, &a.symbol, &a.description, &a.id).cmp(&(
                b.source,
                &b.exchange,
                &b.symbol,
                &b.description,
                &b.id,
            ))
        });
        Self {
            captured_ms: now_ms,
            severity: sources
                .iter()
                .map(|source| source.severity)
                .max()
                .unwrap_or_default(),
            sources,
            rows: Rc::new(rows),
            states,
            details,
        }
    }

    /// How many sources something is asked of.
    pub fn active_sources(&self) -> usize {
        self.sources.iter().filter(|source| source.active).count()
    }

    /// How many feeds are doing well.
    pub fn online(&self) -> usize {
        self.states
            .values()
            .filter(|state| state.severity == Severity::Healthy)
            .count()
    }

    /// What Copy report puts on the clipboard: the sources and feeds with
    /// something wrong, who wants them, and their timings. Every feed is
    /// looked at, whatever page or folded source it is on.
    pub fn report(&self) -> String {
        let mut report = vec![
            "Lattice connection issues".to_owned(),
            format!("Captured: {}", format::iso_time(Some(self.captured_ms))),
        ];
        for (index, (source, view)) in self.details.iter().zip(&self.sources).enumerate() {
            let mut lines = Vec::new();
            if view.active && view.severity > Severity::Healthy {
                if let Some(link) = &source.link {
                    lines.push(format!("Connection: {}", link_word(link.state)));
                    lines.extend(view.notices.iter().map(|notice| format!("  {notice}")));
                    lines.push(format!("  {}", link_timing(link, source)));
                } else {
                    lines.extend(view.notices.iter().cloned());
                }
            }
            lines.extend(view.notes.iter().cloned());
            for (row, feed) in self.rows_of(index, source) {
                let Some(state) = self.states.get(&row.id) else {
                    continue;
                };
                if state.severity <= Severity::Healthy {
                    continue;
                }
                let exchange = match row.exchange.as_str() {
                    "" => String::new(),
                    id => format!("{} · ", exchanges::display_name(id)),
                };
                lines.push(format!("{exchange}{} · {}", row.symbol, state.label));
                lines.push(format!("  Components: {}", row.consumers));
                lines.push(format!("  Feed: {}", row.description));
                if let Some(failure) = &state.failure {
                    lines.push(format!(
                        "  Error: {} (atMs: {})",
                        failure_line(failure),
                        failure.at_ms
                    ));
                }
                if let Some(note) = &state.note {
                    lines.push(format!("  {note}"));
                }
                lines.push(format!("  {}", feed_timing(feed)));
            }
            if lines.is_empty() {
                continue;
            }
            report.push(String::new());
            report.push(format!("{} {} · {}", view.name, view.protocol, view.status));
            report.extend(lines);
        }
        if report.len() == 2 {
            report.push(String::new());
            report.push("No current connection issues.".to_owned());
        }
        report.join("\n")
    }

    /// Source `index`'s rows in list order, each with the feed it is of.
    fn rows_of<'a>(
        &'a self,
        index: usize,
        source: &'a SourceDetails,
    ) -> impl Iterator<Item = (&'a Row, &'a Feed)> {
        let prefix = source.status.id.len() + 1;
        self.rows
            .iter()
            .filter(move |row| row.source == index)
            .filter_map(move |row| {
                let key = row.id.get(prefix..)?;
                let feed = source.feeds.iter().find(|feed| feed.key == key)?;
                Some((row, feed))
            })
    }
}

/// Whether a source can deliver right now: a socket that is open, has
/// answered and misses no keepalive; a query hub always.
fn available(source: &SourceDetails) -> bool {
    source.link.as_ref().is_none_or(|link| {
        link.state == LinkState::Open && link.responsive && link.missed_keepalives == 0
    })
}

fn source_view(source: &SourceDetails, time: &dyn Fn(f64) -> String) -> SourceView {
    let status = &source.status;
    let active = status.demand > 0;
    let severity = if active {
        Severity::of(status.health)
    } else {
        Severity::Neutral
    };
    let link = source.link.as_ref();
    let socket = link.is_some();
    let retrying = link.is_some_and(|link| link.state == LinkState::Retrying);
    let pending = status.health == Health::Pending;
    let word = match severity {
        Severity::Neutral => "Idle",
        Severity::Error => "Needs attention",
        _ if retrying => "Reconnecting",
        Severity::Healthy if socket => "Connected",
        Severity::Healthy => "Up to date",
        Severity::Warning if pending && socket => "Connecting",
        Severity::Warning if pending => "Loading",
        Severity::Warning => "Degraded",
    };

    let mut notices = Vec::new();
    let troubled = active && severity > Severity::Healthy;
    if troubled && let Some(link) = link {
        if link.missed_keepalives > 0 {
            notices.push(format!(
                "No response to {} consecutive keepalive {}.",
                link.missed_keepalives,
                plural(link.missed_keepalives as usize, "check", "checks")
            ));
        } else if !link.responsive {
            notices.push(format!("Waiting for a response from {}.", status.label));
        }
        if let Some(at) = link.retry_at_ms {
            notices.push(format!("Retry at {} · attempt {}", time(at), link.attempt));
        }
    }
    // A first connection or a first fetch in flight has failed at nothing:
    // a failure kept from before is not what it is waiting on.
    if troubled
        && !pending
        && let Some(failure) = &status.last_failure
    {
        notices.push(failure_line(failure));
    }

    let mut notes = Vec::new();
    if active && status.unrouted.frames > 0 {
        let frames = status.unrouted.frames;
        let mut note = format!(
            "{frames} {} matched no subscription",
            plural(frames as usize, "frame", "frames")
        );
        if !status.unrouted.targets.is_empty() {
            note.push_str(&format!(": {}", status.unrouted.targets.join(", ")));
        }
        notes.push(note);
    }

    SourceView {
        id: status.id,
        name: status.label,
        protocol: source.transport.label(),
        latency_ms: link
            .filter(|_| active)
            .and_then(|link| link.latency)
            .map(|latency| latency.as_millis().min(u128::from(u64::MAX)) as u64),
        severity,
        status: word,
        active,
        notices,
        notes,
    }
}

/// How a feed is doing. `available` is whether its source can deliver.
fn row_state(feed: &Feed, available: bool, now_ms: f64) -> RowState {
    match &feed.state {
        FeedState::Topic { phase, alias } => {
            let (severity, label, note) = match phase {
                TopicPhase::Failed => (Severity::Error, "Feed error", None),
                _ if !available => (Severity::Warning, "Waiting for source", None),
                _ if feed.failure.is_some() => (Severity::Warning, "Warning", None),
                TopicPhase::Live => (Severity::Healthy, "Receiving", None),
                TopicPhase::Subscribed => match alias {
                    Some(alias) => (
                        Severity::Warning,
                        "Unconfirmed route",
                        Some(format!("Acknowledged, but its data arrives as {alias}.")),
                    ),
                    None => (Severity::Healthy, "Subscribed", None),
                },
                TopicPhase::Connecting | TopicPhase::Subscribing => {
                    (Severity::Warning, "Awaiting data", None)
                }
            };
            RowState {
                severity,
                label,
                activity_ms: feed.last_data_ms,
                failure: feed.failure.clone(),
                note,
            }
        }
        FeedState::Request {
            loaded,
            fetching,
            failures,
            every,
        } => {
            let stale = every.zip(feed.last_data_ms).is_some_and(|(every, at)| {
                now_ms - at > every.as_secs_f64() * 1_000.0 * STALE_INTERVALS
            });
            let (severity, label) = if *failures >= REPEATED_FAILURES {
                (Severity::Error, "Request failed")
            } else if *failures > 0 {
                (Severity::Warning, "Request failed")
            } else if !loaded {
                (Severity::Warning, "Loading")
            } else if stale {
                (Severity::Warning, "Stale data")
            } else if *fetching {
                (Severity::Healthy, "Refreshing")
            } else {
                (Severity::Healthy, "Up to date")
            };
            RowState {
                severity,
                label,
                activity_ms: feed.last_data_ms,
                failure: feed.failure.clone().filter(|_| *failures > 0),
                note: None,
            }
        }
    }
}

/// What a feed asks for, and how often for a polled one.
fn description(feed: &Feed) -> String {
    let cadence = match &feed.state {
        FeedState::Request {
            every: Some(every), ..
        } => format!("every {}", span(*every)),
        _ => String::new(),
    };
    [feed.subject.detail.clone(), cadence]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A refresh interval in a word: `5s`, `1m`, `90s`.
fn span(every: Duration) -> String {
    let seconds = every.as_secs();
    if seconds == 0 {
        format!("{}ms", every.as_millis())
    } else if seconds.is_multiple_of(60) {
        format!("{}m", seconds / 60)
    } else {
        format!("{seconds}s")
    }
}

/// Who wants a feed, by name: each widget kind once with how many of it
/// (`Order Book ×2 · Chart`), then how many nobody named.
pub(super) fn consumers_label(held: &Held, names: &HashMap<Consumer, &'static str>) -> String {
    let mut labels: Vec<(&str, usize)> = Vec::new();
    for consumer in &held.consumers {
        let label = match consumer {
            Consumer::Part(name) => name,
            // A widget on its way out of the workspace, or of a kind this
            // build does not know.
            Consumer::Widget(_) => names.get(consumer).copied().unwrap_or("Widget"),
        };
        match labels.iter_mut().find(|(seen, _)| *seen == label) {
            Some((_, count)) => *count += 1,
            None => labels.push((label, 1)),
        }
    }
    let mut parts: Vec<String> = labels
        .into_iter()
        .map(|(label, count)| {
            if count > 1 {
                format!("{label} ×{count}")
            } else {
                label.to_owned()
            }
        })
        .collect();
    if held.unattributed > 0 {
        parts.push(format!("{} unattributed", held.unattributed));
    }
    if parts.is_empty() {
        "No retained widgets".to_owned()
    } else {
        parts.join(" · ")
    }
}

/// A failure as the panel writes it: `CODE · message`.
fn failure_line(failure: &Failure) -> String {
    format!("{} · {}", failure.code, failure.message)
}

fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}

fn link_word(state: LinkState) -> &'static str {
    match state {
        LinkState::Idle => "not connected",
        LinkState::Connecting => "connecting",
        LinkState::Open => "open",
        LinkState::Retrying => "retrying",
    }
}

/// A socket's timings, for the report.
fn link_timing(link: &Link, source: &SourceDetails) -> String {
    json!({
        "responsive": link.responsive,
        "missedKeepalives": link.missed_keepalives,
        "retryAtMs": link.retry_at_ms,
        "attempt": link.attempt,
        "latencyMs": link.latency.map(|latency| latency.as_millis() as u64),
        "unroutedFrames": source.status.unrouted.frames,
    })
    .to_string()
}

/// A feed's timings, for the report.
fn feed_timing(feed: &Feed) -> String {
    match &feed.state {
        FeedState::Topic { phase, alias } => json!({
            "phase": format!("{phase:?}"),
            "lastDataAtMs": feed.last_data_ms,
            "arrivingAs": alias,
        }),
        FeedState::Request {
            loaded,
            fetching,
            failures,
            every,
        } => json!({
            "loaded": loaded,
            "fetching": fetching,
            "consecutiveFailures": failures,
            "lastSuccessAtMs": feed.last_data_ms,
            "everyMs": every.map(|every| every.as_millis() as u64),
        }),
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::net::{SourceStatus, Subject, Transport, Unrouted};

    fn clock(ms: f64) -> String {
        format!("t{ms}")
    }

    fn topic(symbol: &str, phase: TopicPhase, widgets: &[u64]) -> Feed {
        Feed {
            key: format!("orderbook:bybit:{symbol}:{{}}"),
            subject: Subject {
                exchange: "bybit".to_owned(),
                name: symbol.to_owned(),
                detail: "Order book · 50 levels".to_owned(),
            },
            state: FeedState::Topic { phase, alias: None },
            held: Held {
                consumers: widgets.iter().map(|id| Consumer::Widget(*id)).collect(),
                unattributed: 0,
            },
            last_data_ms: (phase == TopicPhase::Live).then_some(9_000.0),
            failure: None,
        }
    }

    fn request(name: &str, failures: u32, loaded: bool, at: Option<f64>) -> Feed {
        Feed {
            key: format!("ferris:{name}"),
            subject: Subject {
                exchange: String::new(),
                name: name.to_owned(),
                detail: "Market statistics".to_owned(),
            },
            state: FeedState::Request {
                loaded,
                fetching: false,
                failures,
                every: Some(Duration::from_secs(5)),
            },
            held: Held {
                consumers: vec![Consumer::Part("Market directory")],
                unattributed: 1,
            },
            last_data_ms: at,
            failure: (failures > 0).then(|| Failure {
                at_ms: 1.0,
                code: "NETWORK_ERROR".to_owned(),
                message: "offline".to_owned(),
            }),
        }
    }

    fn socket(health: Health, link: Link, feeds: Vec<Feed>) -> SourceDetails {
        SourceDetails {
            status: SourceStatus {
                id: "ferris.socket",
                label: "Ferris realtime",
                health,
                demand: feeds.len(),
                summary: String::new(),
                last_failure: Some(Failure {
                    at_ms: 2.0,
                    code: "SOCKET_CLOSED".to_owned(),
                    message: "closed with code 1006".to_owned(),
                }),
                unrouted: Unrouted::default(),
            },
            transport: Transport::Socket,
            link: Some(link),
            feeds,
        }
    }

    fn hub(health: Health, feeds: Vec<Feed>) -> SourceDetails {
        SourceDetails {
            status: SourceStatus {
                id: "ferris.queries",
                label: "Ferris requests",
                health,
                demand: feeds.len(),
                summary: String::new(),
                last_failure: None,
                unrouted: Unrouted::default(),
            },
            transport: Transport::Http,
            link: None,
            feeds,
        }
    }

    fn open() -> Link {
        Link {
            state: LinkState::Open,
            responsive: true,
            missed_keepalives: 0,
            latency: Some(Duration::from_millis(42)),
            retry_at_ms: None,
            attempt: 0,
        }
    }

    fn names() -> HashMap<Consumer, &'static str> {
        HashMap::from([
            (Consumer::Widget(1), "Order Book"),
            (Consumer::Widget(2), "Order Book"),
            (Consumer::Widget(3), "Chart"),
        ])
    }

    #[test]
    fn rows_say_what_each_feed_is_who_wants_it_and_how_it_is_doing() {
        let details = vec![
            socket(
                Health::Healthy,
                open(),
                vec![
                    topic("ETH/USDT", TopicPhase::Subscribed, &[3]),
                    topic("BTC/USDT", TopicPhase::Live, &[1, 2, 3, 9]),
                ],
            ),
            hub(
                Health::Degraded,
                vec![
                    request("stats", 1, true, Some(9_500.0)),
                    request("fresh", 0, true, Some(9_500.0)),
                    request("old", 0, true, Some(-10_000.0)),
                    request("first", 0, false, None),
                ],
            ),
        ];
        let snapshot = Snapshot::of(details, &names(), 10_000.0, &clock);

        // By source, then exchange, symbol and description.
        let symbols: Vec<&str> = snapshot
            .rows
            .iter()
            .map(|row| row.symbol.as_str())
            .collect();
        assert_eq!(
            symbols,
            ["BTC/USDT", "ETH/USDT", "first", "fresh", "old", "stats"]
        );
        let row = |symbol: &str| {
            let row = snapshot
                .rows
                .iter()
                .find(|row| row.symbol == symbol)
                .unwrap();
            (row, &snapshot.states[&row.id])
        };

        let (btc, state) = row("BTC/USDT");
        assert_eq!(btc.description, "Order book · 50 levels");
        assert_eq!(btc.consumers, "Order Book ×2 · Chart · Widget");
        assert_eq!(
            (state.severity, state.label),
            (Severity::Healthy, "Receiving")
        );
        assert_eq!(state.activity_ms, Some(9_000.0));
        assert_eq!(row("ETH/USDT").1.label, "Subscribed");

        let (stats, state) = row("stats");
        assert_eq!(stats.description, "Market statistics · every 5s");
        assert_eq!(stats.consumers, "Market directory · 1 unattributed");
        assert_eq!(
            (state.severity, state.label),
            (Severity::Warning, "Request failed")
        );
        assert_eq!(state.hint(), "NETWORK_ERROR · offline");
        assert_eq!(row("fresh").1.label, "Up to date");
        assert_eq!(row("old").1.label, "Stale data");
        assert_eq!(row("first").1.label, "Loading");

        assert_eq!(snapshot.severity, Severity::Warning);
        assert_eq!((snapshot.active_sources(), snapshot.online()), (2, 3));
        let heads: Vec<_> = snapshot
            .sources
            .iter()
            .map(|source| {
                (
                    source.name,
                    source.protocol,
                    source.status,
                    source.latency_ms,
                )
            })
            .collect();
        assert_eq!(
            heads,
            [
                ("Ferris realtime", "WS", "Connected", Some(42)),
                ("Ferris requests", "HTTP", "Degraded", None),
            ]
        );
        // A healthy socket says nothing of a failure it recovered from.
        assert!(snapshot.sources[0].notices.is_empty());
    }

    #[test]
    fn a_socket_that_cannot_deliver_says_why_and_its_topics_wait_for_it() {
        let retrying = Link {
            state: LinkState::Retrying,
            responsive: false,
            missed_keepalives: 0,
            latency: None,
            retry_at_ms: Some(12_000.0),
            attempt: 2,
        };
        let details = vec![socket(
            Health::Degraded,
            retrying,
            vec![
                topic("BTC/USDT", TopicPhase::Connecting, &[1]),
                topic("ETH/USDT", TopicPhase::Failed, &[]),
            ],
        )];
        let snapshot = Snapshot::of(details, &names(), 10_000.0, &clock);
        let source = &snapshot.sources[0];
        assert_eq!(
            (source.severity, source.status),
            (Severity::Warning, "Reconnecting")
        );
        assert_eq!(
            source.notices,
            [
                "Waiting for a response from Ferris realtime.",
                "Retry at t12000 · attempt 2",
                "SOCKET_CLOSED · closed with code 1006",
            ]
        );
        let labels: Vec<_> = snapshot
            .rows
            .iter()
            .map(|row| (snapshot.states[&row.id].label, row.consumers.as_str()))
            .collect();
        assert_eq!(
            labels,
            [
                ("Waiting for source", "Order Book"),
                ("Feed error", "No retained widgets"),
            ]
        );
        assert_eq!(snapshot.online(), 0);

        let report = snapshot.report();
        assert!(
            report.starts_with("Lattice connection issues\nCaptured: 1970-01-01T00:00:10.000Z\n")
        );
        assert!(report.contains("\nFerris realtime WS · Reconnecting\nConnection: retrying\n"));
        assert!(report.contains("\nBybit · BTC/USDT · Waiting for source\n  Components: Order Book\n  Feed: Order book · 50 levels\n"));
        assert!(report.contains("\nBybit · ETH/USDT · Feed error\n"));
    }

    #[test]
    fn nothing_wanted_is_idle_and_reports_no_issues() {
        let mut idle = socket(Health::Idle, open(), Vec::new());
        idle.link.as_mut().unwrap().state = LinkState::Idle;
        let snapshot = Snapshot::of(
            vec![idle, hub(Health::Idle, Vec::new())],
            &names(),
            0.0,
            &clock,
        );
        assert_eq!(snapshot.severity, Severity::Neutral);
        assert_eq!((snapshot.active_sources(), snapshot.online()), (0, 0));
        assert!(
            snapshot
                .sources
                .iter()
                .all(|source| source.status == "Idle")
        );
        assert_eq!(snapshot.sources[0].latency_ms, None);
        assert_eq!(
            [0, 1, 50, 51, 101].map(last_page),
            [0, 0, 0, 1, 2],
            "fifty rows a page"
        );
        assert!(
            snapshot
                .report()
                .ends_with("\n\nNo current connection issues.")
        );
    }
}
