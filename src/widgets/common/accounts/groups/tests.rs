//! The address groups' manager, opened from an account widget's rows.

use nsh_core::prelude::*;
use nsh_testing::{Headless, MemoryStorage, WAIT};
use serde_json::{Value, json};

use crate::widgets::{
    WidgetKind,
    common::accounts::testing::{TRADER, USER, start_storing, workspace_of},
};

/// Where the groups are saved.
const KEY: &str = "lattice.address-groups.v1";
const MANAGER: &str = "id:address-groups";
const FIELD: &str = "id:address-group-address";

/// A workspace of one Positions widget watching no account.
#[component]
fn Unset() -> impl IntoElement {
    workspace_of(WidgetKind::Positions, None)
}

/// Waits until the manager's footer says `said` of the group shown.
fn comes_to(app: &mut Headless, said: &str) {
    app.wait_until(said, WAIT, |screen| screen.shows(said))
        .unwrap();
}

/// The groups as `storage` holds them.
fn saved(storage: &MemoryStorage) -> Value {
    serde_json::from_str(&storage.entries()[KEY]).unwrap()
}

#[test]
fn a_group_is_made_filled_and_given_to_a_widget_whole() {
    let (mut app, storage) = start_storing(nsh_testing::windowed_root::<Unset>(), []);
    app.wait_for("id:positions-no-account").unwrap();

    // "Groups", beside Add, opens the manager: no group yet, none to use.
    app.press("id:positions-groups").unwrap();
    app.wait_for(MANAGER).unwrap();
    assert!(app.shows("No groups yet. Name one below."));
    assert!(app.shows("No group to use."));
    assert!(!app.screen().enabled("id:address-group-use").unwrap());
    assert!(!app.screen().enabled("id:address-group-replace").unwrap());

    // A group is its name; made, it is the one shown.
    app.type_text("id:address-group-name", " Desk \n").unwrap();
    app.wait_for("label:Group Desk").unwrap();
    assert_eq!(
        app.screen().input_text("id:address-group-name").unwrap(),
        Some("")
    );
    assert_eq!(
        app.screen().input_text("id:address-group-rename").unwrap(),
        Some("Desk")
    );
    comes_to(&mut app, "Desk: 0 addresses, 0 accounts");

    // What is no address of the exchange checked is not added.
    app.type_text(FIELD, "vitalik.eth\n").unwrap();
    app.wait_for("id:address-group-problem").unwrap();
    assert!(app.shows("Desk: 0 addresses, 0 accounts"));
    app.clear(FIELD).unwrap();

    // An address is watched on the exchange the rows had picked, at first.
    app.type_text(
        FIELD,
        &format!("{}\n", USER.to_uppercase().replace("0X", "0x")),
    )
    .unwrap();
    comes_to(&mut app, "Desk: 1 address, 1 account");
    assert!(!app.screen().matches("id:address-group-problem").unwrap());
    assert!(app.shows(USER));
    assert_eq!(app.screen().input_text(FIELD).unwrap(), Some(""));

    // Its exchanges are a checklist, which stays open while they are
    // checked: on Lighter too, it is two accounts.
    app.press("id:address-group-exchanges-0").unwrap();
    app.wait_for("id:address-group-exchanges-0-menu").unwrap();
    app.press("id:address-group-exchanges-0-lighterxyz")
        .unwrap();
    comes_to(&mut app, "Desk: 1 address, 2 accounts");
    assert!(
        app.screen()
            .matches("id:address-group-exchanges-0-menu")
            .unwrap()
    );
    app.press_key("Escape").unwrap();
    app.wait_for_gone("id:address-group-exchanges-0-menu", WAIT)
        .unwrap();
    assert!(app.screen().matches(MANAGER).unwrap());

    // The next is watched on Lighter alone: the last exchange checked
    // cannot be unchecked, so Lighter is checked first.
    app.press("id:address-group-new-exchanges").unwrap();
    app.wait_for("id:address-group-new-exchanges-menu").unwrap();
    assert!(
        !app.screen()
            .enabled("id:address-group-new-exchanges-hyperliquid")
            .unwrap()
    );
    app.press("id:address-group-new-exchanges-lighterxyz")
        .unwrap();
    app.press("id:address-group-new-exchanges-hyperliquid")
        .unwrap();
    app.press_key("Escape").unwrap();
    app.wait_for_gone("id:address-group-new-exchanges-menu", WAIT)
        .unwrap();
    app.type_text(FIELD, &format!("{TRADER}\n")).unwrap();
    comes_to(&mut app, "Desk: 2 addresses, 3 accounts");

    // Every change was saved as it was made.
    assert_eq!(
        saved(&storage),
        json!({ "groups": [{ "name": "Desk", "addresses": [
            { "address": USER, "venues": ["hyperliquid", "lighterxyz"] },
            { "address": TRADER, "venues": ["lighterxyz"] },
        ] }] })
    );

    // Used, the group's accounts are the widget's, and the manager closes.
    app.press("id:address-group-use").unwrap();
    app.wait_for_gone(MANAGER, WAIT).unwrap();
    app.wait_for(
        "label:Change accounts: Hyperliquid 0x0123…4567, Lighter 0x0123…4567, \
         Lighter 0xe117…b9a8",
    )
    .unwrap();
    assert!(!app.screen().matches("id:positions-no-account").unwrap());

    // The settings' rows open it too, on the groups as they were left.
    app.press("Widget settings").unwrap();
    app.wait_for("id:positions-settings").unwrap();
    app.press("id:positions-settings >> id:positions-groups")
        .unwrap();
    app.wait_for(MANAGER).unwrap();
    comes_to(&mut app, "Desk: 2 addresses, 3 accounts");

    // × takes an address out; what is left replaces the widget's accounts.
    app.press("id:address-group-take-0").unwrap();
    comes_to(&mut app, "Desk: 1 address, 1 account");
    app.press("id:address-group-replace").unwrap();
    app.wait_for_gone(MANAGER, WAIT).unwrap();
    app.wait_for("label:Change account: Lighter 0xe117…b9a8")
        .unwrap();
    assert!(app.screen().matches("id:positions-settings").unwrap());
}

/// One watching an account already.
#[component]
fn Watching() -> impl IntoElement {
    let saved = json!({ "account": { "venue": "hyperliquid", "address": USER } });
    workspace_of(WidgetKind::Positions, Some(saved))
}

#[test]
fn saved_groups_are_shown_renamed_and_removed_and_take_several_addresses_at_once() {
    let groups = json!({ "groups": [
        { "name": "Desk", "addresses": [{ "address": USER, "venues": ["hyperliquid"] }] },
        { "name": "Funds", "addresses": [] },
    ] });
    let (mut app, storage) = start_storing(
        nsh_testing::windowed_root::<Watching>(),
        [(KEY.to_owned(), groups.to_string())],
    );
    app.wait_for("label:Change account: Hyperliquid 0x0123…4567")
        .unwrap();
    app.press("Widget settings").unwrap();
    app.wait_for("id:positions-settings").unwrap();
    app.press("id:positions-settings >> id:positions-groups")
        .unwrap();
    app.wait_for(MANAGER).unwrap();

    // The groups saved are there, the first of them shown; a line shows
    // its group.
    comes_to(&mut app, "Desk: 1 address, 1 account");
    app.press("id:address-group-1").unwrap();
    comes_to(&mut app, "Funds: 0 addresses, 0 accounts");
    // One of a name there is shows the one there is.
    app.type_text("id:address-group-name", "desk\n").unwrap();
    comes_to(&mut app, "Desk: 1 address, 1 account");
    assert!(!app.screen().matches("label:Group desk").unwrap());

    // Several addresses at once, each once; none while one is no address.
    app.type_text(FIELD, &format!("{TRADER}, nobody\n"))
        .unwrap();
    app.wait_for("id:address-group-problem").unwrap();
    assert!(app.shows("Desk: 1 address, 1 account"));
    app.clear(FIELD).unwrap();
    app.type_text(FIELD, &format!("{USER}, {TRADER} {TRADER}\n"))
        .unwrap();
    comes_to(&mut app, "Desk: 2 addresses, 2 accounts");
    assert!(!app.screen().matches("id:address-group-problem").unwrap());

    // A name another group has is not taken; another is.
    app.press("id:address-group-1").unwrap();
    comes_to(&mut app, "Funds: 0 addresses, 0 accounts");
    app.clear("id:address-group-rename").unwrap();
    app.type_text("id:address-group-rename", "Desk\n").unwrap();
    app.wait_for("id:address-group-problem").unwrap();
    app.clear("id:address-group-rename").unwrap();
    app.type_text("id:address-group-rename", "Whales\n")
        .unwrap();
    app.wait_for("label:Group Whales").unwrap();
    comes_to(&mut app, "Whales: 0 addresses, 0 accounts");
    assert!(!app.screen().matches("id:address-group-problem").unwrap());
    assert_eq!(saved(&storage)["groups"][1]["name"], "Whales");

    // An empty group goes with its ×; one that holds addresses asks again.
    app.press("id:address-group-remove-1").unwrap();
    app.wait_for_gone("label:Group Whales", WAIT).unwrap();
    comes_to(&mut app, "Desk: 2 addresses, 2 accounts");
    app.press("id:address-group-remove-0").unwrap();
    app.wait_for("id:address-group-confirm-0").unwrap();
    assert!(app.screen().matches("label:Group Desk").unwrap());
    app.press("id:address-group-confirm-0").unwrap();
    app.wait_until("no group", WAIT, |screen| {
        screen.shows("No groups yet. Name one below.")
    })
    .unwrap();
    assert_eq!(saved(&storage), json!({ "groups": [] }));

    // Its close button closes it, and the widget is as it was.
    app.press("label:Close Address groups").unwrap();
    app.wait_for_gone(MANAGER, WAIT).unwrap();
    assert!(
        app.screen()
            .matches("label:Change account: Hyperliquid 0x0123…4567")
            .unwrap()
    );
}
