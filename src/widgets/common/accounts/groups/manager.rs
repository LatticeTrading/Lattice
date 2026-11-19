//! The address groups' manager ([`group_manager`]): a modal in the middle
//! of the window, which an account widget's rows open ("Groups").
//!
//! - On the left, narrow: the groups, a line each (its name, how many
//!   addresses it holds, and × which removes it, asking again when it holds
//!   any), the one shown lit; under them the name of a group to make, with
//!   Add (or Enter).
//! - On the right, the group shown: its name in a field (Enter renames it),
//!   then a line for each of its addresses: the address, a button with the
//!   logos of the exchanges it is watched on, which opens their checklist
//!   ([`exchange_button`]), and × which takes the address out. Under them
//!   the address of one to add, the exchanges to watch it on, and Add (or
//!   Enter). Several addresses apart by spaces or commas are added at once,
//!   or none of them while one is no address of an exchange checked.
//! - Under both, why what was typed or checked cannot be, while it cannot.
//! - At the bottom, what the group shown comes to (its addresses, and the
//!   accounts they are on their exchanges), and what to do with it for the
//!   widget: add its accounts to those the widget shows, or show them
//!   instead. Either closes the manager.
//!
//! Every change is saved at once ([`AddressGroups`]). Nothing is asked of
//! an exchange to check an address.

use std::rc::Rc;

use nsh_core::{LineHeight, prelude::*};

use super::{
    exchanges::{Venues, exchange_button, name_of},
    model::{GroupAddress, NAME_LENGTH},
    store::AddressGroups,
};
use crate::{
    data::{
        Portfolio,
        portfolio::{Account, short_address},
    },
    theme,
    ui::{self, Tone},
};

/// The widest the manager is, and its groups' column.
const PANEL_WIDTH: f32 = 640.0;
const NAMES_WIDTH: f32 = 176.0;
/// The tallest its two lists are.
const LISTS_HEIGHT: f32 = 340.0;
/// The height of a group's line and of an address's.
const LINE_HEIGHT: f32 = 28.0;
/// How wide the Add button after an address is, as the rows' own.
const ADD_WIDTH: f32 = 60.0;
const TITLE: &str = "Address groups";

/// What a widget does with a group's accounts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Use {
    /// Shows them too.
    Add,
    /// Shows them instead of its own.
    Replace,
}

/// What the manager's parts share.
#[derive(Clone)]
struct Manager {
    portfolio: Portfolio,
    groups: AddressGroups,
    /// The exchanges that report accounts.
    offer: Venues,
    /// The name of the group shown.
    selected: Signal<Option<String>>,
    /// The exchanges the next address added is watched on.
    checked: Signal<Vec<String>>,
    /// Why what was typed or checked cannot be.
    problem: Signal<Option<String>>,
    /// The group whose × asks again.
    confirming: Signal<Option<String>>,
}

/// The manager of `groups`, over `portfolio`'s exchanges, of which `picked`
/// (the one the widget's rows have picked) is checked for a new address at
/// first. Using a group calls `on_use` with its accounts; that, Escape, the
/// close button and a press beside the manager call `on_close`. Leaving it
/// out of the view is the caller's job.
pub(crate) fn group_manager(
    portfolio: &Portfolio,
    groups: AddressGroups,
    picked: &str,
    on_use: impl Fn(Vec<Account>, Use) + 'static,
    on_close: impl Fn() + Clone + 'static,
) -> View {
    let offer: Venues = portfolio
        .sources()
        .iter()
        .map(|source| (source.venue(), source.name()))
        .collect();
    let first = offer
        .iter()
        .find(|(venue, _)| *venue == picked)
        .or_else(|| offer.first())
        .map(|(venue, _)| (*venue).to_owned());
    let shown =
        groups.with_untracked(|groups| groups.all().first().map(|group| group.name.clone()));
    let manager = Manager {
        portfolio: portfolio.clone(),
        groups,
        offer,
        selected: signal(shown),
        checked: signal(first.into_iter().collect()),
        problem: signal(None),
        confirming: signal(None),
    };

    let pane = {
        let manager = manager.clone();
        dynamic(move || {
            Some(match manager.selected.get() {
                Some(name) => manager.pane(name),
                None => view().flex_1().min_w(px(0.0)).child(note(
                    "Name a group on the left, then add the addresses it holds and the \
                     exchanges to watch each on.",
                )),
            })
        })
    };
    let lists = view()
        .h(move || px((window().height.0 - 180.0).clamp(160.0, LISTS_HEIGHT)))
        .flex_row()
        .child(manager.names())
        .child(pane);
    let body = view()
        .test_id("address-groups")
        .flex_col()
        .text_size(px(11.0))
        .text_color(theme::TEXT)
        .child(lists)
        .child(manager.said())
        .child(manager.footer(Rc::new(on_use), Rc::new(on_close.clone())));
    let width = px((window().width.0 - 16.0).clamp(320.0, PANEL_WIDTH));
    ui::modal_panel(width, TITLE, None, on_close, body)
}

/// A few words where a list has nothing to list.
fn note(said: &'static str) -> View {
    view()
        .p(px(12.0))
        .text_color(theme::TEXT_MUTED)
        .child(text(said).line_height(LineHeight::Multiple(1.5)))
}

/// `count` of something: `1 address`, `3 addresses`.
fn counted(count: usize, one: &str, several: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { several })
}

/// The addresses in `typed`: what is between its spaces, commas and
/// semicolons.
fn typed_addresses(typed: &str) -> Vec<&str> {
    typed
        .split(|character: char| character.is_whitespace() || matches!(character, ',' | ';'))
        .filter(|address| !address.is_empty())
        .collect()
}

impl Manager {
    /// The addresses `typed` names, each as the first of `venues`' sources
    /// writes it and once; why not, when one of them is no address of one
    /// of `venues`.
    fn parsed(&self, typed: &str, venues: &[String]) -> Result<Vec<String>, String> {
        let typed = typed_addresses(typed);
        let mut addresses: Vec<String> = Vec::new();
        for input in &typed {
            let mut written = None;
            for venue in venues {
                match self.portfolio.account(venue, input) {
                    Ok(account) => {
                        written.get_or_insert(account.address);
                    }
                    Err(error) => {
                        // Which address, of several, and which exchange.
                        let exchange = name_of(&self.offer, venue);
                        let whose = match (typed.len() > 1, venues.len() > 1) {
                            (true, true) => format!("{} on {exchange}: ", short_address(input)),
                            (true, false) => format!("{}: ", short_address(input)),
                            (false, true) => format!("{exchange}: "),
                            (false, false) => String::new(),
                        };
                        return Err(format!("{whose}{}", error.message()));
                    }
                }
            }
            if let Some(address) = written
                && !addresses.contains(&address)
            {
                addresses.push(address);
            }
        }
        Ok(addresses)
    }

    /// The groups' column: a line each, over the name of one to make.
    fn names(&self) -> View {
        let Self {
            groups,
            selected,
            problem,
            confirming,
            ..
        } = self.clone();
        let names = computed(move || {
            groups.with(|groups| {
                groups
                    .all()
                    .iter()
                    .map(|group| group.name.clone())
                    .collect::<Vec<_>>()
            })
        });
        let list = {
            let manager = self.clone();
            dynamic(move || {
                let names = names.get();
                Some(if names.is_empty() {
                    note("No groups yet. Name one below.")
                } else {
                    let lines = names
                        .into_iter()
                        .enumerate()
                        .map(|(at, name)| manager.group_line(name, at));
                    view()
                        .w_full()
                        .flex_col()
                        .role(AccessibilityRole::List)
                        .label("Groups")
                        .children(lines.collect::<Vec<_>>())
                })
            })
        };

        let draft = signal(String::new());
        let make: Rc<dyn Fn()> = Rc::new(move || {
            let typed = draft.get_untracked();
            batch(|| {
                let mut made = None;
                groups.update(|groups| made = groups.add(&typed));
                // One of that name is shown instead of made again.
                if let Some(name) = made {
                    draft.set(String::new());
                    selected.set(Some(name));
                    confirming.set(None);
                    problem.set(None);
                }
            });
        });
        let (submit, press) = (make.clone(), make);
        let field = ui::text_field(draft)
            .flex_1()
            .placeholder("New group")
            .label("New group name")
            .test_id("address-group-name")
            .max_length(NAME_LENGTH as u32)
            .on_submit(move |_: &str| submit());
        let add = ui::button("Add", Tone::Normal)
            .flex_none()
            .border_l_1()
            .border_color(theme::BORDER)
            .label("Add group")
            .test_id("address-group-add")
            .on_press(move || press());

        view()
            .w(px(NAMES_WIDTH))
            .h_full()
            .flex_none()
            .flex_col()
            .border_r_1()
            .border_color(theme::BORDER)
            .child(
                scroll()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(view().flex_col().child(list)),
            )
            .child(
                view()
                    .h(ui::ROW_HEIGHT)
                    .w_full()
                    .flex_none()
                    .flex_row()
                    .border_t_1()
                    .border_color(theme::BORDER)
                    .child(field)
                    .child(add),
            )
    }

    /// The line of the group `name`, the `at`th: its name and how many
    /// addresses it holds, which shows it, then × which removes it (asking
    /// again when it holds any).
    fn group_line(&self, name: String, at: usize) -> View {
        let Self {
            groups,
            selected,
            problem,
            confirming,
            ..
        } = self.clone();
        let is = |which: Signal<Option<String>>| {
            let name = name.clone();
            computed(move || which.with(|which| which.as_deref() == Some(name.as_str())))
        };
        let (shown, asked) = (is(selected), is(confirming));
        let held = {
            let name = name.clone();
            computed(move || {
                groups.with(|groups| groups.named(&name).map_or(0, |group| group.addresses.len()))
            })
        };

        let show = {
            let name = name.clone();
            move || {
                batch(|| {
                    selected.set(Some(name.clone()));
                    confirming.set(None);
                    problem.set(None);
                });
            }
        };
        let pick = view()
            .h_full()
            .flex_1()
            .min_w(px(0.0))
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .px(px(10.0))
            .role(AccessibilityRole::Button)
            .label(format!("Group {name}"))
            .test_id(&format!("address-group-{at}"))
            .accessibility_state(move || AccessibilityState {
                selected: shown.get(),
                ..Default::default()
            })
            .on_press(show)
            .child(text(name.clone()).flex_1().min_w(px(0.0)).truncate())
            .child(
                text(move || held.get().to_string())
                    .flex_none()
                    .tabular_nums()
                    .text_color(theme::TEXT_FAINT),
            );

        let remove: Rc<dyn Fn()> = {
            let name = name.clone();
            Rc::new(move || {
                batch(|| {
                    let mut next = None;
                    groups.update(|groups| next = groups.remove(&name));
                    if selected.with_untracked(|shown| shown.as_deref() == Some(name.as_str())) {
                        selected.set(next);
                    }
                    confirming.set(None);
                    problem.set(None);
                });
            })
        };
        let ask = {
            let (name, remove) = (name.clone(), remove.clone());
            ui::button("×", Tone::Quiet)
                .flex_none()
                .border_l_1()
                .border_color(theme::BORDER)
                .label(format!("Remove group {name}"))
                .test_id(&format!("address-group-remove-{at}"))
                .style_when(move || asked.get(), |style| style.hidden())
                .on_press(move || {
                    // Nothing is lost with an empty one.
                    if held.get_untracked() == 0 {
                        remove();
                    } else {
                        confirming.set(Some(name.clone()));
                    }
                })
        };
        let confirm = ui::button("Remove?", Tone::Danger)
            .flex_none()
            .border_l_1()
            .border_color(theme::BORDER)
            .label(format!("Remove group {name} and its addresses"))
            .test_id(&format!("address-group-confirm-{at}"))
            .style_when(move || !asked.get(), |style| style.hidden())
            .on_press(move || remove());

        view()
            .h(px(LINE_HEIGHT))
            .w_full()
            .flex_row()
            .items_center()
            .border_b_1()
            .border_color(theme::BORDER)
            .text_color(theme::TEXT)
            .hover(|style| style.bg(theme::SURFACE))
            .style_when(
                move || shown.get(),
                |style| style.bg(theme::SURFACE).text_color(theme::ACCENT),
            )
            .child(pick)
            .child(ask)
            .child(confirm)
    }

    /// The group `name`: its name, its addresses, and the one to add.
    fn pane(&self, name: String) -> View {
        let Self {
            portfolio,
            groups,
            offer,
            selected,
            checked,
            problem,
            ..
        } = self.clone();

        let title = signal(name.clone());
        let rename = {
            let name = name.clone();
            move |typed: &str| {
                if typed.trim() == name {
                    return;
                }
                batch(|| {
                    let mut renamed = None;
                    groups.update(|groups| renamed = groups.rename(&name, typed));
                    match renamed {
                        Some(renamed) => {
                            problem.set(None);
                            selected.set(Some(renamed));
                        }
                        None => {
                            title.set(name.clone());
                            let why = if typed.trim().is_empty() {
                                "A group needs a name."
                            } else {
                                "Another group has that name."
                            };
                            problem.set(Some(why.to_owned()));
                        }
                    }
                });
            }
        };
        let heading = ui::text_field(title)
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(theme::BORDER)
            .label("Group name")
            .placeholder("Group name")
            .test_id("address-group-rename")
            .max_length(NAME_LENGTH as u32)
            .on_submit(rename);

        let addresses = {
            let name = name.clone();
            computed(move || {
                groups.with(|groups| {
                    groups.named(&name).map_or_else(Vec::new, |group| {
                        group
                            .addresses
                            .iter()
                            .map(|listed| listed.address.clone())
                            .collect::<Vec<_>>()
                    })
                })
            })
        };
        let list = {
            let (manager, name) = (self.clone(), name.clone());
            dynamic(move || {
                let addresses = addresses.get();
                Some(if addresses.is_empty() {
                    note(
                        "No addresses yet. Add the ones this group holds below, each with \
                         the exchanges to watch it on.",
                    )
                } else {
                    let lines = addresses
                        .into_iter()
                        .enumerate()
                        .map(|(at, address)| manager.address_line(&name, address, at));
                    view()
                        .test_id("address-group-addresses")
                        .w_full()
                        .flex_col()
                        .children(lines.collect::<Vec<_>>())
                })
            })
        };

        // The address to add, as it is typed.
        let draft = signal(String::new());
        let commit: Rc<dyn Fn()> = {
            let manager = self.clone();
            Rc::new(move || {
                let venues = checked.get_untracked();
                match manager.parsed(&draft.get_untracked(), &venues) {
                    Ok(addresses) => batch(|| {
                        problem.set(None);
                        draft.set(String::new());
                        groups.update(|groups| {
                            for address in addresses {
                                let venues = venues.clone();
                                groups.put(&name, GroupAddress { address, venues });
                            }
                        });
                    }),
                    Err(why) => problem.set(Some(why)),
                }
            })
        };
        let (submit, press) = (commit.clone(), commit);
        let field = ui::text_field(draft)
            .flex_1()
            .placeholder(move || {
                checked
                    .with(|checked| checked.first().and_then(|venue| portfolio.source(venue)))
                    .map_or("Account address", |source| source.address_hint())
                    .to_owned()
            })
            .label("Address to add")
            .test_id("address-group-address")
            .on_submit(move |_: &str| submit());
        let exchanges = exchange_button(
            &offer,
            move || checked.get(),
            move |venue, on| {
                problem.set(None);
                checked.update(|checked| {
                    let at = checked.iter().position(|watched| watched == venue);
                    match (on, at) {
                        (true, None) => checked.push(venue.to_owned()),
                        (false, Some(at)) if checked.len() > 1 => {
                            checked.remove(at);
                        }
                        _ => {}
                    }
                });
            },
            "Exchanges to watch".to_owned(),
            "address-group-new-exchanges",
        )
        .border_l_1()
        .border_color(theme::BORDER);
        let add = ui::button("Add", Tone::Normal)
            .w(px(ADD_WIDTH))
            .flex_none()
            .justify_center()
            .border_l_1()
            .border_color(theme::BORDER)
            .label("Add address")
            .test_id("address-group-address-add")
            .on_press(move || press());

        view()
            .h_full()
            .flex_1()
            .min_w(px(0.0))
            .flex_col()
            .child(heading)
            .child(
                scroll()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(view().flex_col().child(list)),
            )
            .child(
                view()
                    .h(ui::ROW_HEIGHT)
                    .w_full()
                    .flex_none()
                    .flex_row()
                    .border_t_1()
                    .border_color(theme::BORDER)
                    .child(field)
                    .child(exchanges)
                    .child(add),
            )
    }

    /// The line of `address`, the `at`th of the group `group`: the address,
    /// the exchanges it is watched on (which opens their checklist), and ×
    /// which takes it out of the group.
    fn address_line(&self, group: &str, address: String, at: usize) -> View {
        let Self {
            portfolio,
            groups,
            offer,
            problem,
            ..
        } = self.clone();
        let short = short_address(&address);
        let watched = {
            let (group, address) = (group.to_owned(), address.clone());
            move || groups.with(|groups| groups.venues_of(&group, &address))
        };
        let toggle = {
            let (group, address, offer) = (group.to_owned(), address.clone(), offer.clone());
            move |venue: &'static str, on: bool| {
                // What one exchange calls an address another may not.
                if on && let Err(error) = portfolio.account(venue, &address) {
                    let exchange = name_of(&offer, venue);
                    problem.set(Some(format!("{exchange}: {}", error.message())));
                    return;
                }
                problem.set(None);
                groups.update(|groups| groups.watch(&group, &address, venue, on));
            }
        };
        let exchanges = exchange_button(
            &offer,
            watched,
            toggle,
            format!("Exchanges of {short}"),
            &format!("address-group-exchanges-{at}"),
        )
        .border_l_1()
        .border_color(theme::BORDER);
        let remove = {
            let (group, address) = (group.to_owned(), address.clone());
            ui::button("×", Tone::Quiet)
                .flex_none()
                .border_l_1()
                .border_color(theme::BORDER)
                .label(format!("Remove {short}"))
                .test_id(&format!("address-group-take-{at}"))
                .on_press(move || {
                    problem.set(None);
                    groups.update(|groups| groups.take(&group, &address));
                })
        };
        let whole = address.clone();
        view()
            .h(px(LINE_HEIGHT))
            .w_full()
            .flex_row()
            .items_center()
            .pl(px(10.0))
            .border_b_1()
            .border_color(theme::BORDER)
            .child(
                text(address)
                    .flex_1()
                    .min_w(px(0.0))
                    .pr(px(8.0))
                    .truncate()
                    .tabular_nums()
                    .text_color(theme::TEXT)
                    .tooltip(ui::hint(move || whole.clone())),
            )
            .child(exchanges)
            .child(remove)
    }

    /// Why what was typed or checked cannot be, as a warning; hidden while
    /// there is nothing to say.
    fn said(&self) -> View {
        let problem = self.problem;
        view()
            .flex_none()
            .px(px(10.0))
            .py(px(5.0))
            .border_t_1()
            .border_l(px(3.0))
            .border_color(theme::BORDER)
            .border_l_color(theme::WARNING)
            .bg(theme::WARNING.opacity(0.1))
            .text_color(theme::WARNING)
            .role(AccessibilityRole::Text)
            .test_id("address-group-problem")
            .style_when(
                move || problem.with(Option::is_none),
                |style| style.hidden(),
            )
            .child(
                text(move || problem.get().unwrap_or_default())
                    .line_height(LineHeight::Multiple(1.5)),
            )
    }

    /// What the group shown comes to, and the buttons that use it for the
    /// widget.
    fn footer(&self, on_use: Rc<dyn Fn(Vec<Account>, Use)>, on_close: Rc<dyn Fn()>) -> View {
        let Self {
            portfolio,
            groups,
            selected,
            ..
        } = self.clone();
        // The group shown: its name, how many addresses it holds, and the
        // accounts they are.
        let shown = computed(move || {
            let name = selected.get()?;
            groups.with(|groups| {
                let group = groups.named(&name)?;
                Some((
                    group.name.clone(),
                    group.addresses.len(),
                    group.accounts(&portfolio),
                ))
            })
        });
        let summary = move || {
            shown.with(|shown| match shown {
                Some((name, addresses, accounts)) => format!(
                    "{name}: {}, {}",
                    counted(*addresses, "address", "addresses"),
                    counted(accounts.len(), "account", "accounts")
                ),
                None => "No group to use.".to_owned(),
            })
        };
        let unusable = move || {
            shown.with(|shown| {
                shown
                    .as_ref()
                    .is_none_or(|(_, _, accounts)| accounts.is_empty())
            })
        };
        let button = |said: &'static str, named: &'static str, id: &str, how: Use| {
            let (on_use, on_close) = (on_use.clone(), on_close.clone());
            ui::button(said, Tone::Normal)
                .flex_none()
                .border_l_1()
                .border_color(theme::BORDER)
                .label(named)
                .test_id(id)
                .tooltip(ui::hint(move || named.to_owned()))
                .disabled(unusable)
                .on_press(move || {
                    let accounts = shown
                        .with_untracked(|shown| {
                            shown.as_ref().map(|(_, _, accounts)| accounts.clone())
                        })
                        .unwrap_or_default();
                    if accounts.is_empty() {
                        return;
                    }
                    // Closed first: the widget's first accounts take the
                    // rows the manager opened from out of the view.
                    let (on_use, on_close) = (on_use.clone(), on_close.clone());
                    on_close();
                    on_use(accounts, how);
                })
        };
        let replace = button(
            "Replace accounts",
            "Show this group's accounts instead of the widget's",
            "address-group-replace",
            Use::Replace,
        );
        let add = button(
            "Add to widget",
            "Add this group's accounts to the widget's",
            "address-group-use",
            Use::Add,
        )
        .text_color(theme::ACCENT);
        view()
            .h(ui::ROW_HEIGHT)
            .flex_none()
            .flex_row()
            .items_center()
            .border_t_1()
            .border_color(theme::BORDER)
            .child(
                view().flex_1().min_w(px(0.0)).px(px(10.0)).child(
                    text(summary)
                        .truncate()
                        .tabular_nums()
                        .text_color(theme::TEXT_MUTED),
                ),
            )
            .child(replace)
            .child(add)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_addresses_are_apart_by_spaces_commas_and_semicolons() {
        assert_eq!(typed_addresses("  0x01 "), ["0x01"]);
        assert_eq!(
            typed_addresses("0x01, 0x02;0x03\n0x04\t,,0x05"),
            ["0x01", "0x02", "0x03", "0x04", "0x05"]
        );
        assert!(typed_addresses(" ,; ").is_empty());
        assert_eq!(counted(1, "address", "addresses"), "1 address");
        assert_eq!(counted(0, "address", "addresses"), "0 addresses");
    }
}
