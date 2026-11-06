//! Static Everything Bar. Product controls are disabled, not fake implementations.
//! Native window management is the only interaction enabled in this slice.

use std::rc::Rc;

use gpui_kit::{
    App, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Pixels, RenderOnce,
    Role, StatefulInteractiveElement, Styled, TestSupportExt, Window, WindowControlArea,
    component::{
        Disableable, Icon, IconName, Sizable,
        button::{Button, ButtonVariants},
        input::{Input, InputState},
    },
    div,
    prelude::FluentBuilder,
    px,
};

use crate::{
    assets,
    theme::{self, color},
};

pub const BAR_INSET: Pixels = px(8.);
pub const BAR_HEIGHT: Pixels = px(40.);

#[derive(IntoElement)]
pub struct EverythingBar {
    search: Entity<InputState>,
    on_new_widget: Rc<dyn Fn(&mut App)>,
}

impl EverythingBar {
    pub fn new(search: &Entity<InputState>, on_new_widget: impl Fn(&mut App) + 'static) -> Self {
        Self {
            search: search.clone(),
            on_new_widget: Rc::new(on_new_widget),
        }
    }
}

/// Local chrome styling, not an application-wide button abstraction. Kit owns
/// disabled semantics, focus, keyboard activation and accessible button roles.
fn segment(id: &'static str) -> Button {
    Button::new(id)
        .ghost()
        .rounded(px(0.))
        .h_full()
        .px(px(12.))
        .border_l_1()
        .border_color(color(theme::BORDER))
        .text_color(color(theme::TEXT_SECONDARY))
}

fn placeholder(id: &'static str, label: &'static str) -> Button {
    segment(id)
        .accessibility_label(label)
        .disabled(true)
        .child(div().text_size(px(12.)).child(label))
}

fn icon_button(
    id: &'static str,
    label: &'static str,
    icon: impl Into<Icon>,
    width: Pixels,
) -> Button {
    segment(id)
        .accessibility_label(label)
        .tooltip(label)
        .w(width)
        .px_0()
        .child(Icon::new(icon).with_size(px(12.)))
}

impl RenderOnce for EverythingBar {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let compact = window.viewport_size().width <= px(768.);
        let control_width = px(if compact { 36. } else { 40. });
        let page_width = px(if compact { 28. } else { 32. });

        let left = div()
            .flex()
            .h_full()
            .flex_shrink_0()
            .child(
                div()
                    .id("wordmark")
                    .window_control_area(WindowControlArea::Drag)
                    .flex()
                    .items_center()
                    .h_full()
                    .px(px(if compact { 12. } else { 16. }))
                    .text_size(px(if compact { 12. } else { 14. }))
                    .font_weight(FontWeight::BOLD)
                    .child("LATTICE"),
            )
            .child(placeholder("layout", "Layout"))
            // One blank initial page; no sample layouts or extra numbered pages.
            .child(
                placeholder("page-1", "1")
                    .accessibility_label("Layout 1")
                    .w(page_width)
                    .px_0()
                    .text_color(color(theme::ACCENT))
                    .bg(color(theme::ACCENT).opacity(0.1)),
            )
            .child(
                placeholder("add-page", "+")
                    .accessibility_label("Add layout page")
                    .w(page_width)
                    .px_0()
                    // Close the segment even when space separates it from search.
                    // Share this edge with search's left border when adjacent.
                    .border_r_1()
                    .mr(-px(1.)),
            );

        let center = div()
            .flex()
            .flex_1()
            .min_w_0()
            .max_w(px(800.))
            .h_full()
            .child(
                Input::new(&self.search)
                    .id("symbol-search")
                    .aria_label("Search symbols")
                    .disabled(true)
                    .appearance(false)
                    .small()
                    // Input::h configures multiline editors; Styled::h sets the
                    // actual single-line control frame to the bar's inner height.
                    .map(|input| Styled::h(input, BAR_HEIGHT - px(2.)))
                    .flex_1()
                    .min_w_0()
                    .px(px(8.))
                    .border_l_1()
                    .border_color(color(theme::BORDER))
                    .text_size(px(if compact { 11. } else { 12. }))
                    .prefix(Icon::new(IconName::Search).with_size(px(14.))),
            )
            .child(
                segment("new-widget")
                    .accessibility_label("New Widget")
                    .on_click({
                        let on_new_widget = self.on_new_widget.clone();
                        move |_, _, cx| on_new_widget(cx)
                    })
                    .flex_none()
                    .px(px(if compact { 12. } else { 16. }))
                    // Keep a visible end before the flexible gap. The negative
                    // margin shares the clock group's left border when adjacent.
                    .border_r_1()
                    .mr(-px(1.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(Icon::new(IconName::Plus).with_size(px(14.)))
                            .child(div().text_size(px(12.)).child("New Widget")),
                    ),
            );

        let right = div()
            .flex()
            .h_full()
            .flex_shrink_0()
            .border_l_1()
            .border_color(color(theme::BORDER))
            .child(
                div()
                    .id("clock")
                    .aria_label("Clock unavailable")
                    .test_support()
                    .window_control_area(WindowControlArea::Drag)
                    .flex()
                    .h_full()
                    .items_center()
                    .px(px(if compact { 12. } else { 16. }))
                    .text_size(px(if compact { 11. } else { 12. }))
                    .font_weight(FontWeight::MEDIUM)
                    // No fabricated timestamp and no timer before clock support.
                    .child("--:--:--"),
            )
            .child(
                icon_button(
                    "connections",
                    "Connection status",
                    Icon::default().path(assets::CONNECTION_STATUS),
                    control_width,
                )
                .text_color(color(theme::TEXT_TERTIARY))
                .disabled(true),
            )
            .child(
                icon_button(
                    "settings",
                    "Application settings",
                    IconName::Settings,
                    control_width,
                )
                .disabled(true),
            )
            .child(
                icon_button(
                    "minimize",
                    "Minimize",
                    IconName::WindowMinimize,
                    control_width,
                )
                .when(cfg!(target_os = "windows"), |button| {
                    button.window_control_area(WindowControlArea::Min)
                })
                .on_click(|_, window, _| window.minimize_window()),
            )
            .child(
                icon_button(
                    "maximize",
                    if window.is_maximized() {
                        "Restore"
                    } else {
                        "Maximize"
                    },
                    if window.is_maximized() {
                        IconName::WindowRestore
                    } else {
                        IconName::WindowMaximize
                    },
                    control_width,
                )
                .when(cfg!(target_os = "windows"), |button| {
                    button.window_control_area(WindowControlArea::Max)
                })
                .on_click(|_, window, _| window.zoom_window()),
            )
            .child(
                icon_button("close", "Close", IconName::WindowClose, control_width)
                    .when(cfg!(target_os = "windows"), |button| {
                        button.window_control_area(WindowControlArea::Close)
                    })
                    .on_click(|_, window, _| window.remove_window()),
            );

        div()
            .id("everything-bar")
            .role(Role::Toolbar)
            .aria_label("Everything Bar")
            .test_support()
            .flex()
            .justify_between()
            .w_full()
            .h(BAR_HEIGHT)
            .flex_shrink_0()
            .border_1()
            .border_color(color(theme::BORDER))
            .bg(color(theme::BAR_BACKGROUND))
            .child(left)
            .child(center)
            .child(right)
    }
}
