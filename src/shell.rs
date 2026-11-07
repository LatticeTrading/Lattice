//! Application chrome and the first interactive widget workspace.

mod everything_bar;

#[cfg(test)]
mod tests;

use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window,
    component::input::InputState, div,
};

use crate::layout::Workspace;
use crate::theme;
use everything_bar::{BAR_INSET, EverythingBar};

pub struct Shell {
    search: Entity<InputState>,
    workspace: Entity<Workspace>,
    _workspace_subscription: Subscription,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        let workspace_subscription = cx.observe(&workspace, |_, _, cx| cx.notify());
        Self {
            search: cx.new(|cx| InputState::new(window, cx).placeholder("Search symbols...")),
            workspace,
            _workspace_subscription: workspace_subscription,
        }
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace = self.workspace.clone();
        let workspace_for_pages = self.workspace.clone();
        let workspace_for_selection = self.workspace.clone();
        let page_ids = self.workspace.read(cx).page_ids();
        let active_page_id = self.workspace.read(cx).active_page_id();
        div()
            .size_full()
            .flex()
            .flex_col()
            .p(BAR_INSET)
            .bg(theme::color(theme::WINDOW_BACKGROUND))
            .font_family(theme::FONT_FAMILY)
            .text_color(theme::color(theme::TEXT_PRIMARY))
            .child(EverythingBar::new(
                &self.search,
                move |app| {
                    workspace.update(app, |workspace, cx| workspace.spawn(cx));
                },
                move |app| {
                    workspace_for_pages.update(app, |workspace, cx| workspace.add_page(cx));
                },
                move |app, page_id| {
                    workspace_for_selection
                        .update(app, |workspace, cx| workspace.select_page(page_id, cx));
                },
                page_ids,
                active_page_id,
            ))
            .child(self.workspace.clone())
    }
}
