//! The old app's compact number field: a whole number within a range,
//! typed or stepped with Up and Down.

use std::{ops::RangeInclusive, rc::Rc};

use nsh_core::{
    element::{Input, color},
    keymap::When,
    prelude::*,
};

use crate::{theme, ui};

/// A field for a whole number within `range`, 60pt wide and right
/// aligned, showing `value` (tracked).
///
/// - While something else is typed it shows that, else `value`.
/// - Enter or leaving the field commits what is typed: a number, rounded
///   and clamped into `range` (one above it saves the range's end), goes
///   to `on_commit`; anything else goes back to `value`. The field then
///   shows `value` again.
/// - Up and Down while it has the focus commit the next step above or
///   below what is typed (or `value`, while that is not a number), in
///   steps of `step` from the range's start as an HTML number field counts
///   them, clamped into `range`.
///
/// Chain its label, hint and test id onto it.
pub(crate) fn number_field(
    value: impl Fn() -> u16 + 'static,
    range: RangeInclusive<u16>,
    step: u16,
    on_commit: impl Fn(u16) + 'static,
) -> Input {
    let (min, max) = (*range.start(), *range.end());
    let value = computed(value);
    // What the field shows: `value`, unless something else is being
    // typed.
    let draft = signal(value.get_untracked().to_string());
    effect(move || draft.set(value.get().to_string()));
    let typed = move || read(&draft.get_untracked());
    let save: Rc<dyn Fn(u16)> = Rc::new(move |next: u16| {
        on_commit(next.clamp(min, max));
        draft.set(value.get_untracked().to_string());
    });
    let commit = {
        let save = save.clone();
        move || match typed() {
            Some(next) => save(next),
            None => draft.set(value.get_untracked().to_string()),
        }
    };
    let focused = signal(false);
    effect(move || {
        if !focused.get() {
            return;
        }
        let keys = KeyOptions {
            when: When::Always,
            repeat: true,
            ..KeyOptions::default()
        };
        for (key, up) in [("up", true), ("down", false)] {
            let save = save.clone();
            on_key_with(key, keys, move |_| {
                let from = typed().unwrap_or_else(|| value.get_untracked());
                save(stepped(from, up, min, step));
            });
        }
    });
    let (submit, blur) = (commit.clone(), commit);
    ui::text_field(draft)
        .w(px(60.0))
        .h(px(24.0))
        .flex_none()
        .px(px(6.0))
        .text_size(px(11.0))
        .text_right()
        .bg(theme::SURFACE)
        .border_1()
        .border_color(theme::BORDER)
        .hover(|s| s.border_color(theme::TEXT_FAINT))
        .focus(|s| s.border_color(theme::ACCENT))
        .selection_color(|| {
            color(theme::SURFACE.get()).blend(color(theme::ACCENT.opacity(0.35).get()))
        })
        .keyboard_type(KeyboardType::Number)
        .on_focus(move || focused.set(true))
        .on_submit(move |_: &str| submit())
        .on_blur(move || {
            focused.set(false);
            blur();
        })
}

/// The whole number `text` holds, rounded and saturated into `u16` (one
/// above it reads as `u16::MAX`, one below 0 as 0), so stepping from it
/// never overflows; the caller clamps it into its range.
fn read(text: &str) -> Option<u16> {
    let typed = text.trim().parse::<f64>().ok()?;
    // `as` saturates.
    typed.is_finite().then(|| typed.round() as u16)
}

/// The step after `from` (`up`) or before it, on the grid of `step`s from
/// `min`; below `min` either way is `min`. The caller clamps it.
fn stepped(from: u16, up: bool, min: u16, step: u16) -> u16 {
    let (from, min, step) = (u32::from(from), u32::from(min), u32::from(step.max(1)));
    let next = match from.checked_sub(min) {
        Some(above) if up => min + (above / step + 1) * step,
        Some(above) if above > 0 => min + (above.div_ceil(step) - 1) * step,
        _ => min,
    };
    next.min(u32::from(u16::MAX)) as u16
}
