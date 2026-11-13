//! The shared Everything Bar.
//!
//! Desktop uses the platform's normal title bar for window controls. The web
//! build gets no title bar because the browser owns that chrome.

use nsh_base::tooltip;
use nsh_core::{
    desktop::{TitleBarOptions, WindowAttributes},
    element::{ImageSrc, desktop::current_window, input},
    prelude::*,
};

use super::{
    clock::{self, Clock},
    settings,
};
use crate::{
    layout, storage,
    storage::StoredWorkspace,
    theme::{self, ActiveTheme},
    ui::{self, Side, Tone},
};

#[cfg(test)]
mod tests;

pub const BAR_HEIGHT: Points = px(40.0);

const PANEL_WIDTH: Points = px(340.0);

/// A destructive Layout Manager action waiting for the user to confirm it.
#[derive(Clone, PartialEq)]
enum Pending {
    /// Delete (or, for the last page, empty) the page with this id.
    Clear(usize),
    /// Replace every page with an imported, already validated workspace.
    Import(Box<StoredWorkspace>),
}

#[derive(Clone, PartialEq)]
struct Feedback {
    error: bool,
    message: String,
}

/// The Layout Manager's state. It lives as long as the bar, so a pasted or
/// generated code survives closing and reopening the panel.
#[derive(Clone, Copy)]
struct Manager {
    open: Signal<bool>,
    code: Signal<String>,
    pending: Signal<Option<Pending>>,
    feedback: Signal<Option<Feedback>>,
}

impl Manager {
    fn new() -> Self {
        Self {
            open: signal(false),
            code: signal(String::new()),
            pending: signal(None),
            feedback: signal(None),
        }
    }

    fn close(self) {
        self.open.set(false);
        self.pending.set(None);
        self.feedback.set(None);
    }

    fn toggle(self) {
        if self.open.get_untracked() {
            self.close();
        } else {
            self.open.set(true);
        }
    }

    fn report(self, error: bool, message: impl Into<String>) {
        self.feedback.set(Some(Feedback {
            error,
            message: message.into(),
        }));
    }

    fn ask(self, pending: Pending) {
        self.feedback.set(None);
        self.pending.set(Some(pending));
    }

    /// Validates the pasted code, then asks before replacing anything.
    fn request_import(self) {
        match layout::import_code(&self.code.get_untracked()) {
            Ok(imported) => self.ask(Pending::Import(Box::new(imported))),
            Err(error) => {
                self.pending.set(None);
                self.report(true, error);
            }
        }
    }

    fn confirm(self, workspace: Signal<StoredWorkspace>) {
        let Some(pending) = self.pending.get_untracked() else {
            return;
        };
        self.pending.set(None);
        match pending {
            Pending::Clear(id) => {
                let mut state = workspace.get_untracked();
                let Some(number) = page_number(&state, id) else {
                    return;
                };
                layout::clear_page(&mut state, id);
                match persist(&state) {
                    Ok(()) => {
                        workspace.set(state);
                        self.report(false, format!("Layout {number} cleared."));
                    }
                    Err(error) => self.report(true, error),
                }
            }
            Pending::Import(imported) => match persist(&imported) {
                Ok(()) => {
                    workspace.set(*imported);
                    self.report(false, "Layout imported.");
                }
                Err(error) => self.report(true, error),
            },
        }
    }
}

pub fn bar(
    workspace: Signal<StoredWorkspace>,
    active_theme: ActiveTheme,
    clock: Clock,
) -> impl IntoElement {
    let manager = Manager::new();
    let now = clock::ticking_seconds();
    let settings_open = signal(false);
    let bar_ref = element_ref();
    // Keep minimize, maximize/restore, and close in the real desktop title
    // bar; the Everything Bar holds only app controls. Off desktops (the
    // browser owns the chrome on web) this does nothing.
    let mut window_attributes = WindowAttributes::default();
    window_attributes.title = Some("Lattice".to_owned());
    window_attributes.title_bar = Some(TitleBarOptions::default());
    current_window().set(window_attributes);

    let controls = dynamic(move || {
        let saved = workspace.get();
        let compact = window().width.0 <= 768.0;
        let control_width = if compact { 36.0 } else { 40.0 };
        let page_width = if compact { 28.0 } else { 32.0 };

        let wordmark = view()
            .h_full()
            .flex()
            .items_center()
            .flex_shrink_0()
            .px(px(if compact { 12.0 } else { 16.0 }))
            .text_size(px(if compact { 12.0 } else { 14.0 }))
            .font_weight(FontWeight::BOLD)
            .child(text("LATTICE"));

        let layout_button = fitted_segment("Layout")
            .label("Layout manager")
            .on_press(move || manager.toggle());

        let pages = saved
            .pages
            .iter()
            .enumerate()
            .map(|(index, page)| {
                let id = page.id;
                let selected = id == saved.active_page_id;
                segment(&(index + 1).to_string(), page_width)
                    .accessibility_state(move || AccessibilityState {
                        selected,
                        ..Default::default()
                    })
                    .text_color(if selected {
                        theme::ACCENT
                    } else {
                        theme::TEXT_MUTED
                    })
                    .bg(if selected {
                        theme::ACCENT.opacity(0.10)
                    } else {
                        theme::SURFACE
                    })
                    .on_press(move || {
                        workspace.update(|state| {
                            layout::select_page(state, id);
                            let _ = persist(state);
                        });
                    })
            })
            .collect::<Vec<_>>();

        let add_page = segment("+", page_width)
            .border_r_1()
            .label("Add layout page")
            .on_press({
                let workspace = workspace;
                move || {
                    workspace.update(|state| {
                        layout::add_page(state);
                        let _ = persist(state);
                    });
                }
            });

        let left = view()
            .h_full()
            .flex()
            .items_center()
            .flex_shrink_0()
            .child(wordmark)
            .child(layout_button)
            .children(pages)
            .child(add_page);

        let search = input()
            .h(px(38.0))
            .flex_1()
            .min_w(px(0.0))
            .px(px(8.0))
            .placeholder("Search symbols...")
            .readonly(true)
            .disabled(true)
            .text_size(px(if compact { 11.0 } else { 12.0 }))
            .border_l_1()
            .border_color(theme::BORDER)
            .label("Search symbols");

        let new_widget = view()
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(if compact { 12.0 } else { 16.0 }))
            .border_l_1()
            .border_r_1()
            .border_color(theme::BORDER)
            .text_color(theme::TEXT_MUTED)
            .label("New Widget")
            .on_press({
                let workspace = workspace;
                move || {
                    workspace.update(|state| {
                        layout::spawn(state);
                        let _ = persist(state);
                    });
                }
            })
            .child(text("+"))
            .child(text("New Widget").text_size(px(12.0)));

        let zone = move || clock::offset_label(clock.get().offset);
        let clock = view()
            .test_id("clock")
            .h_full()
            .flex()
            .items_center()
            .flex_shrink_0()
            .px(px(if compact { 12.0 } else { 16.0 }))
            .text_size(px(if compact { 11.0 } else { 12.0 }))
            .font_weight(FontWeight::MEDIUM)
            .border_l_1()
            .border_color(theme::BORDER)
            .label(move || format!("Time, {}", zone()))
            .tooltip(
                tooltip()
                    .placement(Placement::Bottom)
                    .px(px(8.0))
                    .py(px(4.0))
                    .bg(theme::PANEL)
                    .border_1()
                    .border_color(theme::BORDER)
                    .text_color(theme::TEXT)
                    .text_size(px(11.0))
                    .child(text(zone)),
            )
            .child(text(move || clock::format_time(now.get(), clock.get())));

        let connection = view()
            .size(px(14.0))
            .pointer_events(PointerEvents::None)
            .child(
                svg(ImageSrc::svg_markup(include_str!(
                    "../../assets/icons/connection-status.svg"
                )))
                .size_full(),
            );
        let connection = icon_segment(control_width)
            .disabled(true)
            .label("Connection status")
            .child(connection);
        let settings = icon_segment(control_width)
            .label("Application settings")
            .on_press(move || settings_open.update(|open| *open = !*open))
            .child(text("⚙"));

        let center = view()
            .w(px(400.0))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .child(search)
            .child(new_widget);

        let right = view()
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .child(clock)
            .child(connection)
            .child(settings);

        let left_slot = view().flex_1().h_full().flex().items_center().child(left);
        let right_slot = view()
            .flex_1()
            .h_full()
            .flex()
            .items_center()
            .justify_end()
            .child(right);

        view()
            .size_full()
            .flex_row()
            .items_center()
            .child(left_slot)
            .child(center)
            .child(right_slot)
    });

    view()
        .element_ref(&bar_ref)
        .test_id("everything-bar")
        .role(AccessibilityRole::Toolbar)
        .label("Everything Bar")
        .h(BAR_HEIGHT)
        .w_full()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .border_1()
        .border_color(theme::BORDER)
        .bg(theme::SURFACE)
        .child(controls)
        // Outside the bar's `dynamic` so the panel, and the field's focus and
        // caret, survive workspace changes such as clearing a page.
        .child(dynamic(move || {
            manager
                .open
                .get()
                .then(|| layout_manager(workspace, manager, &bar_ref))
        }))
        .child(dynamic(move || {
            settings_open.get().then(|| {
                settings::panel(active_theme, clock, &bar_ref, move || {
                    settings_open.set(false)
                })
            })
        }))
}

fn layout_manager(
    workspace: Signal<StoredWorkspace>,
    manager: Manager,
    anchor: &ElementRef,
) -> Overlay {
    let pages = dynamic(move || {
        let saved = workspace.get();
        saved
            .pages
            .iter()
            .enumerate()
            .map(|(index, page)| {
                let (number, id) = (index + 1, page.id);
                let active = id == saved.active_page_id;
                view()
                    .h(px(32.0))
                    .w_full()
                    .flex()
                    .items_center()
                    .pl(px(10.0))
                    .text_size(px(11.0))
                    .text_color(theme::TEXT)
                    .child(text(format!("Layout {number}")))
                    .child(text(if active { " · active" } else { "" }).text_color(theme::ACCENT))
                    .child(view().flex_1())
                    .child(
                        ui::button("Clear", Tone::Quiet)
                            .label(format!("Clear layout {number}"))
                            .on_press(move || manager.ask(Pending::Clear(id))),
                    )
            })
            .collect::<Vec<_>>()
    });

    let generate = ui::button("Generate layout code", Tone::Normal)
        .h(ui::ROW_HEIGHT)
        .w_full()
        .on_press(
            move || match layout::export_code(&workspace.get_untracked()) {
                Ok(code) => {
                    set_clipboard(code.clone());
                    manager.code.set(code);
                    manager.pending.set(None);
                    manager.report(false, "Layout code generated and copied.");
                }
                Err(error) => manager.report(true, error),
            },
        );

    let code_input = ui::text_field(manager.code)
        .test_id("layout-code")
        .flex_1()
        .placeholder("Paste import key...")
        .label("Layout code")
        .on_change(move |_: &str| {
            manager.pending.set(None);
            manager.feedback.set(None);
        })
        .on_submit(move |_: &str| manager.request_import());

    let import = ui::button("Import", Tone::Normal)
        .flex_none()
        .px(px(12.0))
        .border_l_1()
        .border_color(theme::BORDER)
        .label("Import layout")
        .disabled(move || manager.code.get().trim().is_empty())
        .on_press(move || manager.request_import());

    let confirmation = dynamic(move || {
        let pending = manager.pending.get()?;
        let (question, action) = match &pending {
            Pending::Clear(id) => {
                let saved = workspace.get();
                let number = page_number(&saved, *id)?;
                let question = if saved.pages.len() == 1 {
                    format!("Remove every widget from layout {number}? This cannot be undone.")
                } else {
                    format!("Delete layout {number} and its widgets? This cannot be undone.")
                };
                (question, "Delete layout")
            }
            Pending::Import(_) => (
                "Replace all current layouts with this code? This cannot be undone.".to_owned(),
                "Replace layouts",
            ),
        };
        Some(
            view()
                .w_full()
                .flex_col()
                .border_t_1()
                .border_color(theme::BORDER)
                .child(
                    view()
                        .p(px(10.0))
                        .text_size(px(11.0))
                        .role(AccessibilityRole::Alert)
                        .child(text(question)),
                )
                .child(
                    view()
                        .h(ui::ROW_HEIGHT)
                        .flex_row()
                        .border_t_1()
                        .border_color(theme::BORDER)
                        .child(
                            ui::button("Cancel", Tone::Normal)
                                .flex_1()
                                .justify_center()
                                .on_press(move || manager.pending.set(None)),
                        )
                        .child(
                            ui::button(action, Tone::Danger)
                                .flex_1()
                                .justify_center()
                                .border_l_1()
                                .border_color(theme::BORDER)
                                .on_press(move || manager.confirm(workspace)),
                        ),
                ),
        )
    });

    let feedback = dynamic(move || {
        manager.feedback.get().map(|feedback| {
            view()
                .w_full()
                .px(px(10.0))
                .py(px(8.0))
                .border_t_1()
                .border_color(theme::BORDER)
                .text_size(px(11.0))
                .text_color(if feedback.error {
                    theme::DANGER
                } else {
                    theme::SUCCESS
                })
                .role(AccessibilityRole::Alert)
                .child(text(feedback.message))
        })
    });

    let body = view()
        .test_id("layout-manager")
        .max_h(px(520.0))
        .flex_col()
        .child(
            scroll()
                .max_h(px(240.0))
                .pt(px(4.0))
                .pb(px(4.0))
                .role(AccessibilityRole::List)
                .label("Layout pages")
                .child(pages),
        )
        .child(
            view()
                .w_full()
                .border_t_1()
                .border_color(theme::BORDER)
                .child(generate),
        )
        .child(
            view()
                // The field's full height plus the row's top border.
                .h(ui::ROW_HEIGHT + px(1.0))
                .flex_none()
                .flex()
                .items_center()
                .border_t_1()
                .border_color(theme::BORDER)
                .child(code_input)
                .child(import),
        )
        .child(confirmation)
        .child(feedback);

    ui::floating_panel(
        anchor,
        Side::Left,
        PANEL_WIDTH,
        "Layout Manager",
        move || manager.close(),
        body,
    )
}

/// The 1-based number the bar shows for a page.
fn page_number(state: &StoredWorkspace, id: usize) -> Option<usize> {
    state
        .pages
        .iter()
        .position(|page| page.id == id)
        .map(|index| index + 1)
}

fn persist(state: &StoredWorkspace) -> Result<(), String> {
    storage::save(state).map_err(|error| error.to_string())
}

fn segment(label: &str, width: f32) -> View {
    icon_segment(width).child(text(label.to_owned()))
}

fn fitted_segment(label: &str) -> View {
    view()
        .h_full()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .px(px(12.0))
        .border_l_1()
        .border_color(theme::BORDER)
        .text_color(theme::TEXT_MUTED)
        .text_size(px(12.0))
        .child(text(label.to_owned()))
}

fn icon_segment(width: f32) -> View {
    view()
        .h_full()
        .w(px(width))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .px(px(8.0))
        .border_l_1()
        .border_color(theme::BORDER)
        .text_color(theme::TEXT_MUTED)
        .text_size(px(12.0))
}
