//! Application chrome and the first interactive widget workspace.

mod everything_bar;

#[cfg(test)]
mod tests;

use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Window,
    component::input::InputState, div,
};

use crate::layout::Workspace;
use crate::theme;
use everything_bar::{BAR_INSET, EverythingBar};

pub struct Shell {
    search: Entity<InputState>,
    workspace: Entity<Workspace>,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        Self {
            search: cx.new(|cx| InputState::new(window, cx).placeholder("Search symbols...")),
            workspace,
        }
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let workspace = self.workspace.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .p(BAR_INSET)
            .bg(theme::color(theme::WINDOW_BACKGROUND))
            .font_family(theme::FONT_FAMILY)
            .text_color(theme::color(theme::TEXT_PRIMARY))
            .child(EverythingBar::new(&self.search, move |app| {
                workspace.update(app, |workspace, cx| workspace.spawn(cx));
            }))
            .child(self.workspace.clone())
    }
}
