//! Headless layout/semantics checks. Native hit testing and GPU pixels still
//! require the Windows smoke checklist in docs/initial-shell.md.

use gpui_kit::{
    AppContext, Bounds, Role, TestAppContext, WindowBounds, px, size, test::TestWindowExt,
};

use crate::{app, shell::Shell, theme};

#[gpui_kit::test]
fn shell_keeps_chrome_reachable_and_placeholders_disabled(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(cx).expect("load bundled fonts");
    });
    let (handle, shell) = cx.update(|cx| {
        gpui_kit::open_window(app::window_options(cx), cx, |window, cx| {
            cx.new(|cx| Shell::new(window, cx))
        })
        .expect("open shell")
    });

    let mut full_size_widget_width = None;
    for width in [1920., 1600., 1280., 768., 640.] {
        cx.simulate_window_resize(handle, size(px(width), px(800.)));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let bar = window.find("everything-bar");
            assert_eq!(bar.role(), Some(Role::Toolbar));
            assert_eq!(bar.bounds().left(), px(8.));
            assert_eq!(bar.bounds().top(), px(8.));
            assert_eq!(bar.bounds().size.height, px(40.));
            assert_eq!(bar.bounds().size.width, px(width - 16.));

            for id in [
                "layout",
                "page-1",
                "add-page",
                "new-widget",
                "connections",
                "settings",
            ] {
                assert_eq!(window.find(id).disabled(), Some(true), "{id}");
            }
            assert_eq!(window.find("clock").label(), Some("Clock unavailable"));
            assert_eq!(window.find("symbol-search").role(), Some(Role::TextInput));
            let search = window.find("symbol-search");
            assert!(search.bounds().size.width > px(20.));
            assert_eq!(search.bounds().size.height, px(38.));
            assert_eq!(search.placeholder(), Some("Search symbols..."));
            assert_eq!(search.value(), Some(""));

            let page = window.find("page-1").bounds();
            let add_page = window.find("add-page").bounds();
            assert_eq!(add_page.size, page.size);
            assert_eq!(add_page.left(), page.right());
            assert_eq!(
                add_page.size.width,
                px(if width <= 768. { 28. } else { 32. })
            );

            let new_widget = window.find("new-widget").bounds();
            assert!(new_widget.size.width >= px(100.) && new_widget.size.width <= px(140.));
            assert_eq!(new_widget.size.height, px(38.));
            if width > 768. {
                let expected_width = *full_size_widget_width.get_or_insert(new_widget.size.width);
                assert_eq!(
                    new_widget.size.width, expected_width,
                    "button grew at {width}px"
                );
            }
            if width >= 1600. {
                // Surplus width belongs to the gaps, not either terminal button.
                assert!(search.bounds().left() > add_page.right());
                assert!(window.find("clock").bounds().left() > new_widget.right());
            } else {
                assert_eq!(search.bounds().left(), add_page.right() - px(1.));
                assert_eq!(window.find("clock").bounds().left(), new_widget.right());
            }
            // Kit 0.9 reports no disabled flag on the projected input owner;
            // check its actual input state and pointer/focus behavior instead.
            assert!(shell.read(cx).search.read(cx).presentation().is_disabled());
            window.click("symbol-search", cx);
            assert_eq!(window.find("symbol-search").focused(), Some(false));
            let mut previous_right = bar.bounds().left();
            for id in [
                "layout",
                "page-1",
                "add-page",
                "symbol-search",
                "new-widget",
                "clock",
                "connections",
                "settings",
                "minimize",
                "maximize",
                "close",
            ] {
                let element = window.find(id);
                let bounds = element.bounds();
                assert!(element.visible(), "{id} at {width}");
                // Only search may overlap the preceding control's 1px border.
                let shared_border = if id == "symbol-search" {
                    px(1.)
                } else {
                    px(0.)
                };
                assert!(
                    bounds.left() >= previous_right - shared_border,
                    "{id} overlaps at {width}"
                );
                assert!(
                    bounds.right() <= bar.bounds().right(),
                    "{id} clipped at {width}"
                );
                previous_right = bounds.right();
            }
            for (id, label) in [
                ("minimize", "Minimize"),
                ("maximize", "Maximize"),
                ("close", "Close"),
            ] {
                assert_eq!(window.find(id).role(), Some(Role::Button));
                assert_eq!(window.find(id).label(), Some(label));
                assert_ne!(window.find(id).disabled(), Some(true));
            }
            window.click("layout", cx);
            window.click("add-page", cx);
            window.click("new-widget", cx);
            window.click("settings", cx);
            assert_eq!(cx.windows().len(), 1);
            assert!(window.try_find("page-2").is_none());
        })
        .expect("inspect shell");
    }
}

#[gpui_kit::test]
fn initial_window_contract(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let options = app::window_options(cx);
        assert_eq!(options.titlebar.unwrap().title.as_deref(), Some("Lattice"));
        assert_eq!(options.window_min_size, Some(size(px(640.), px(360.))));
        assert!(options.is_resizable);
        let Some(WindowBounds::Windowed(Bounds {
            size: initial_size, ..
        })) = options.window_bounds
        else {
            panic!("initial window should be windowed");
        };
        assert_eq!(initial_size, size(px(1280.), px(800.)));
    });
}
