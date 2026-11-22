//! Paper: an exchange that is none, for tests. It reports accounts and is
//! traded on as an exchange's integration is ([`AccountSource`],
//! [`TradingVenue`]), from a book it keeps in memory, and it has nothing a
//! venue may leave out: no stream of accounts, no leverage setting, no
//! margin report for trading, no price source, nothing to sign and nothing
//! kept between runs. Its instruments are named as no other exchange names
//! its own (`BTC-PERP`).
//!
//! It is what shows that the layers above a venue know no exchange: the
//! widgets that trade are run over it as they are over Hyperliquid
//! (`the Trade Panel's and the Positions' tests`), and what they do there
//! asks Hyperliquid nothing. A widget or a layer that came to count on
//! something Hyperliquid's integration does fails those tests.
//!
//! What it does with an order: a limit order rests until cancelled; a
//! market order that only reduces fills at the mark for no more than is
//! held and takes that off the position; any other market order is taken
//! with nothing more said ([`Placed::Accepted`]). Leverage and exits it
//! refuses in words.

use std::{cell::RefCell, rc::Rc, time::Duration};

use alloy::signers::Signer;

use super::{
    CancelRequest, Doing, Exits, Following, Hears, Instrument, Instruments, OpenOrder, OpenOrders,
    OrderKind, OrderRequest, Placed, Side, TradingVenue,
};
use crate::data::{
    net::{ApiError, Queries, QuerySpec, StatusBoard},
    portfolio::{
        AccountSource, AddressError, Balances, Direction, Leverage, MarginAccount, Performance,
        Position, evm_address,
    },
    wallet::WalletSigner,
};

/// Its id, as accounts are saved under it and widgets pick it.
pub(crate) const VENUE: &str = "paper";
/// What everything it trades is marked at.
const MARK: f64 = 110.0;
/// How often what it reports is asked for again while watched; what an
/// order changes is asked for at once.
const REFRESH: Duration = Duration::from_secs(60);

/// What it keeps.
struct Book {
    /// The account trading is enabled for, if any.
    enabled: Option<String>,
    /// The last order id it gave.
    last: u64,
    resting: Vec<OpenOrder>,
    /// How much of `BTC-PERP` every account holds, long.
    held: f64,
}

/// The exchange. Cloning is cheap: every clone is the same one.
#[derive(Clone)]
pub(crate) struct Paper {
    queries: Queries,
    book: Rc<RefCell<Book>>,
}

fn refused<T>(why: impl AsRef<str>) -> Result<T, ApiError> {
    Err(ApiError::invalid_request(why))
}

impl Paper {
    /// One whose every account holds 2 `BTC-PERP` long, entered at 100.
    pub(crate) fn new(status: StatusBoard) -> Self {
        Self {
            queries: Queries::new(status, "paper.queries", "Paper requests"),
            book: Rc::new(RefCell::new(Book {
                enabled: None,
                last: 0,
                resting: Vec::new(),
                held: 2.0,
            })),
        }
    }

    fn orders_key(address: &str) -> String {
        format!("paper orders {address}")
    }

    fn margin_key(address: &str) -> String {
        format!("paper margin {address}")
    }

    /// Asks again for what an order of `address`'s changed.
    fn changed(&self, address: &str) {
        self.queries.refresh(&Self::orders_key(address));
        self.queries.refresh(&Self::margin_key(address));
    }

    /// What became of `order`, placed for `address`.
    fn take(&self, address: &str, order: &OrderRequest) -> Result<Placed, ApiError> {
        let mut book = self.book.borrow_mut();
        if book.enabled.as_deref() != Some(address) {
            return refused("Enable trading on Paper for this account first.");
        }
        if order.instrument != "BTC-PERP" {
            return refused(format!("Paper lists no {}.", order.instrument));
        }
        if (order.size * 10.0).fract().abs() > 1e-9 {
            return refused("A size of BTC-PERP has 1 decimal place at most.");
        }
        book.last += 1;
        let id = book.last.to_string();
        match order.kind {
            OrderKind::Limit { price, .. } => {
                book.resting.insert(
                    0,
                    OpenOrder {
                        id: id.clone(),
                        instrument: order.instrument.clone(),
                        side: order.side,
                        price: Some(price),
                        size: Some(order.size),
                        reduce_only: order.reduce_only,
                        kind: "Limit".to_owned(),
                        trigger_price: None,
                        exits: Exits::default(),
                        whole_position: false,
                    },
                );
                Ok(Placed::Resting { id })
            }
            OrderKind::Market { .. } if order.reduce_only => {
                if order.side != Side::Sell || book.held <= 0.0 {
                    return refused("There is nothing for this order to reduce.");
                }
                let size = order.size.min(book.held);
                book.held -= size;
                Ok(Placed::Filled {
                    id,
                    size,
                    price: MARK,
                })
            }
            OrderKind::Market { .. } => Ok(Placed::Accepted),
        }
    }
}

impl TradingVenue for Paper {
    fn venue(&self) -> &'static str {
        VENUE
    }

    fn name(&self) -> &'static str {
        "Paper"
    }

    fn enabling_note(&self) -> &'static str {
        "Paper trades nothing real."
    }

    fn queries(&self) -> &Queries {
        &self.queries
    }

    fn instruments_query(&self) -> QuerySpec<Instruments> {
        let listed = |symbol: &str, tradable: bool| Instrument {
            symbol: symbol.to_owned(),
            size_decimals: 1,
            max_leverage: None,
            tradable,
            isolated_only: false,
        };
        QuerySpec::new("paper instruments", move || async move {
            Ok(Instruments(vec![
                listed("BTC-PERP", true),
                listed("OLD-PERP", false),
            ]))
        })
        .refresh_every(REFRESH)
    }

    fn open_orders_query(&self, address: &str) -> QuerySpec<OpenOrders> {
        let book = self.book.clone();
        QuerySpec::new(Self::orders_key(address), move || {
            let resting = book.borrow().resting.clone();
            async move { Ok(OpenOrders(resting)) }
        })
        .refresh_every(REFRESH)
    }

    fn margin_query(&self, _address: &str) -> Option<QuerySpec<MarginAccount>> {
        None
    }

    fn leverage_query(
        &self,
        _address: &str,
        _instrument: &str,
    ) -> Option<QuerySpec<Option<Leverage>>> {
        None
    }

    fn nearest_price(&self, _instrument: &Instrument, price: f64) -> Option<f64> {
        let whole = price.round();
        (whole.is_finite() && whole > 0.0).then_some(whole)
    }

    fn follow(&self, _address: &str, _hears: Hears) -> Option<Following> {
        None
    }

    fn enable(&self, signer: WalletSigner) -> Doing<()> {
        // Nothing to sign: the account is allowed as it is.
        self.book.borrow_mut().enabled = Some(format!("{:#x}", signer.address()));
        Box::pin(async { Ok(()) })
    }

    fn kept(&self, _address: &str) -> bool {
        false
    }

    fn resume(&self, _address: &str) -> Doing<bool> {
        Box::pin(async { Ok(false) })
    }

    fn release(&self) {
        self.book.borrow_mut().enabled = None;
    }

    fn disable(&self, _address: &str) {
        self.book.borrow_mut().enabled = None;
    }

    fn place(&self, address: &str, order: OrderRequest) -> Doing<Placed> {
        let made = self.take(address, &order);
        self.changed(address);
        Box::pin(async move { made })
    }

    fn set_leverage(&self, _address: &str, _instrument: &str, _leverage: Leverage) -> Doing<()> {
        Box::pin(async { refused("Paper has no leverage setting.") })
    }

    fn protect(&self, _address: &str, _instrument: &str, _exits: Exits) -> Doing<()> {
        Box::pin(async { refused("Paper places no exits.") })
    }

    fn cancel(&self, address: &str, cancel: CancelRequest) -> Doing<()> {
        let made = {
            let mut book = self.book.borrow_mut();
            let before = book.resting.len();
            book.resting.retain(|order| order.id != cancel.id);
            if book.resting.len() < before {
                Ok(())
            } else {
                refused("Paper has no such order.")
            }
        };
        self.changed(address);
        Box::pin(async move { made })
    }
}

impl AccountSource for Paper {
    fn venue(&self) -> &'static str {
        VENUE
    }

    fn name(&self) -> &'static str {
        "Paper"
    }

    fn address_hint(&self) -> &'static str {
        "0x… account address"
    }

    fn parse_address(&self, input: &str) -> Result<String, AddressError> {
        evm_address(input).ok_or(AddressError("Enter the account's address."))
    }

    fn queries(&self) -> &Queries {
        &self.queries
    }

    fn margin_query(&self, address: &str) -> Option<QuerySpec<MarginAccount>> {
        let book = self.book.clone();
        let spec = QuerySpec::new(Self::margin_key(address), move || {
            let held = book.borrow().held;
            let position = Position {
                instrument: "BTC-PERP".to_owned(),
                direction: Direction::Long,
                size: held,
                entry_price: Some(100.0),
                mark_price: Some(MARK),
                value: Some(held * MARK),
                unrealized_pnl: Some(held * (MARK - 100.0)),
                return_on_equity: None,
                liquidation_price: None,
                margin_used: None,
                leverage: None,
            };
            let account = MarginAccount {
                positions: Some(position)
                    .filter(|position| position.size > 0.0)
                    .into_iter()
                    .collect(),
                ..MarginAccount::default()
            };
            async move { Ok(account) }
        });
        Some(spec.refresh_every(REFRESH))
    }

    fn balances_query(&self, _address: &str) -> Option<QuerySpec<Balances>> {
        None
    }

    fn performance_query(&self, _address: &str) -> Option<QuerySpec<Performance>> {
        None
    }
}
