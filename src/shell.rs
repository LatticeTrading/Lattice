//! Application chrome. The workspace is deliberately empty in this first slice.

mod everything_bar;

#[cfg(test)]
mod tests;

use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Window,
    component::input::InputState, div,
};

use crate::theme;
use everything_bar::{BAR_INSET, EverythingBar};

pub struct Shell {
    search: Entity<InputState>,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            search: cx.new(|cx| InputState::new(window, cx).placeholder("Search symbols...")),
        }
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .p(BAR_INSET)
            .bg(theme::color(theme::WINDOW_BACKGROUND))
            .font_family(theme::FONT_FAMILY)
            .text_color(theme::color(theme::TEXT_PRIMARY))
            .child(EverythingBar::new(&self.search))
    }
}
