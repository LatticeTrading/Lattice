//! The rows' vertical scrolling, which the table drives itself instead of
//! the platform, so the rows the list shows are always bound.
//!
//! A scroll the platform makes natively (its scrollbar, the wheel) moves
//! the list's offset on the main thread at once, and the engine binds the
//! rows coming into view later: scrolled further ahead of it than the rows
//! kept bound around the viewport, the list shows rows not bound yet. A
//! non-animated `scroll_to_offset` from the app moves the core's window of
//! rows and binds the new ones in the commit that sends the scroll, so the
//! offset never gets ahead of its rows. So the list does not scroll by
//! itself: the wheel over it, the scroll keys while the pointer is over the
//! table, the table's own scrollbar and the table's jumps to the top all
//! set one offset ([`RowScroll`]), which goes to the list at most once an
//! engine turn. A fast drag sends its latest position, never a queue of old
//! ones: the rows and the thumb may trail the pointer, but never show
//! blank. Sideways scrolling stays native (the table's horizontal scroll
//! view and its pinned symbol column).

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use nsh_core::{
    AccessibilityActionDef, AccessibilityValue, element::Spawned, keymap::When, prelude::*,
    scroll::KEY_LINE,
};

use super::ROW_HEIGHT;
use crate::theme;

/// A wheel line in points, as the platform scrolls one (a notch is the
/// system's lines per notch, three by default).
const WHEEL_LINE: f32 = 16.0;
/// The share of the viewport a page scrolls (the rest stays in view), as
/// the platform's page keys do.
const PAGE: f32 = 0.875;
/// The scrollbar's width.
const BAR: f32 = 10.0;
/// The thumb's inset from the track's sides.
const THUMB_INSET: f32 = 2.0;
/// The shortest thumb, however many rows there are.
const MIN_THUMB: f32 = 24.0;
/// How long the track is held before it pages again, and how often then.
const REPEAT_DELAY: Duration = Duration::from_millis(300);
const REPEAT_EVERY: Duration = Duration::from_millis(50);

/// Where a scroll key moves the rows.
#[derive(Clone, Copy)]
enum Step {
    By(f32),
    Pages(f32),
    Start,
    End,
}

/// The scroll keys: those the platform's keyboard scrolling takes along a
/// list (Left and Right stay its own, for the table's sideways scroll).
const KEYS: [(&str, Step); 10] = [
    ("up", Step::By(-KEY_LINE.0)),
    ("down", Step::By(KEY_LINE.0)),
    ("pageup", Step::Pages(-1.0)),
    ("pagedown", Step::Pages(1.0)),
    ("space", Step::Pages(1.0)),
    ("shift+space", Step::Pages(-1.0)),
    ("home", Step::Start),
    ("end", Step::End),
    ("ctrl+home", Step::Start),
    ("ctrl+end", Step::End),
];

/// The rows' vertical offset: the one place it is kept, the latest asked
/// for, within the rows. Every scroll goes through [`RowScroll::to`].
#[derive(Clone, Copy)]
pub(super) struct RowScroll {
    /// Where the rows are scrolled to, `[0, max]`.
    target: Signal<f32>,
    /// The rows' viewport height, as laid out.
    viewport: Signal<f32>,
    /// How many rows there are.
    count: Memo<usize>,
    /// The furthest offset: the rows' height past the viewport.
    max: Memo<f32>,
}

impl RowScroll {
    /// The offset of `list`, a list of `count` rows [`ROW_HEIGHT`] tall
    /// (no header, footer, padding or separators), which sends it to the
    /// list once a turn while it changes.
    pub fn new(list: ElementRef, count: impl Fn() -> usize + 'static) -> Self {
        let target = signal(0.0_f32);
        let viewport = signal(0.0_f32);
        let count = computed(count);
        let max = computed(move || (count.get() as f32 * ROW_HEIGHT - viewport.get()).max(0.0));
        // Fewer rows (a narrower search) or a taller viewport: still within
        // the rows.
        effect(move || {
            let max = max.get();
            if target.get_untracked() > max {
                target.set(max);
            }
        });
        // The writes of a turn send one scroll, of where the last left the
        // offset: a task runs once the turn's events are handled, before
        // its commit. Spawned under the table, not this effect, which runs
        // again (and would end it) on the next write.
        let owner = Owner::current();
        let pending = Rc::new(Cell::new(false));
        let first = Cell::new(true);
        effect(move || {
            target.with(|_| ());
            if first.replace(false) || pending.replace(true) {
                return;
            }
            let pending = pending.clone();
            owner.run(|| {
                spawn(async move {
                    pending.set(false);
                    list.scroll_to_offset(px(target.get_untracked()), false);
                })
            });
        });
        Self {
            target,
            viewport,
            count,
            max,
        }
    }

    /// Scrolls the rows to `offset` (within them).
    pub fn to(self, offset: f32) {
        let max = self.max.get_untracked();
        self.target.set(offset.clamp(0.0, max));
    }

    /// Scrolls the rows by `delta` (positive: towards the end).
    pub fn by(self, delta: f32) {
        self.to(self.target.get_untracked() + delta);
    }

    /// Scrolls the rows by `pages` viewports, less a little, so the rows at
    /// the edge stay in view.
    pub fn page(self, pages: f32) {
        let page = (self.viewport.get_untracked() * PAGE).max(KEY_LINE.0);
        self.by(pages * page);
    }

    /// The rows' viewport is `height` tall.
    pub fn measure(self, height: f32) {
        self.viewport.set(height);
    }

    fn step(self, step: Step) {
        match step {
            Step::By(delta) => self.by(delta),
            Step::Pages(pages) => self.page(pages),
            Step::Start => self.to(0.0),
            Step::End => self.to(f32::MAX),
        }
    }

    /// A wheel or touchpad over the rows. A sideways one (Shift + wheel,
    /// a tilted wheel, a touchpad swiped sideways) and Control + wheel are
    /// left to the platform: the table scrolls sideways natively, and
    /// zooms.
    pub fn wheel(self, event: &WheelEvent) {
        if event.modifiers.ctrl || (event.modifiers.shift && !event.delta.precise()) {
            return;
        }
        let delta = event.delta.points(px(WHEEL_LINE));
        if delta.x.0.abs() > delta.y.0.abs() {
            return;
        }
        self.by(delta.y.0);
    }

    /// Binds the scroll keys while `active` (the pointer is over the table,
    /// as the platform's keyboard scrolling follows it) and no text input
    /// has the focus. A bound key is the table's even at an edge: the
    /// platform would give it to the sideways scroll instead.
    pub fn bind_keys(self, active: impl Fn() -> bool + 'static) {
        effect(move || {
            if !active() {
                return;
            }
            let options = KeyOptions {
                when: When::NotEditing,
                repeat: true,
                ..KeyOptions::default()
            };
            for (keys, step) in KEYS {
                on_key_with(keys, options, move |_| self.step(step));
            }
        });
    }

    /// The scrollbar, the rows' height: dragging its thumb scrolls the
    /// rows, and pressing the track pages towards the pointer (again while
    /// held). Hidden while every row shows; the caller places it.
    pub fn scrollbar(self) -> View {
        let Self {
            target,
            viewport,
            count,
            max,
        } = self;
        let thumb = computed(move || {
            let view = viewport.get();
            let content = count.get() as f32 * ROW_HEIGHT;
            if content <= view {
                return view;
            }
            (view * view / content).max(MIN_THUMB).min(view)
        });
        let top = computed(move || {
            let (max, travel) = (max.get(), (viewport.get() - thumb.get()).max(0.0));
            if max <= 0.0 {
                0.0
            } else {
                target.get() / max * travel
            }
        });
        let dragging = signal(false);
        let start = Rc::new(Cell::new(0.0_f32));
        let (grant_start, move_start) = (start.clone(), start);
        let handle = view()
            .absolute()
            .left(px(THUMB_INSET))
            .right(px(THUMB_INSET))
            .top(move || px(top.get()))
            .h(move || px(thumb.get()))
            .rounded_full()
            .bg(theme::TEXT_FAINT.opacity(0.6))
            .hover(|s| s.bg(theme::TEXT_FAINT))
            .style_when(move || dragging.get(), |s| s.bg(theme::TEXT_MUTED))
            .claim_touches(TouchClaim::on_start())
            .keep_touches(true)
            .on_responder_grant(move |_: &ResponderEvent| {
                grant_start.set(target.get_untracked());
                dragging.set(true);
            })
            .on_responder_move(move |event: &ResponderEvent| {
                let travel = (viewport.get_untracked() - thumb.get_untracked()).max(0.0);
                if travel > 0.0 {
                    let per_point = max.get_untracked() / travel;
                    self.to(move_start.get() + event.gesture.delta.y.0 * per_point);
                }
            })
            .on_responder_release(move |_: &ResponderEvent| dragging.set(false))
            .on_responder_terminate(move |_: &ResponderEvent| dragging.set(false));

        // A track press pages once, then again while held until the thumb
        // reaches the pointer.
        let owner = Owner::current();
        let pointer = Rc::new(Cell::new(0.0_f32));
        let repeat = Rc::new(RefCell::new(None::<Spawned<()>>));
        let towards = move |y: f32| {
            let top = top.get_untracked();
            if y < top {
                -1.0
            } else if y > top + thumb.get_untracked() {
                1.0
            } else {
                0.0
            }
        };
        let stop = {
            let repeat = repeat.clone();
            move || {
                let held = repeat.borrow_mut().take();
                if let Some(held) = held {
                    held.cancel();
                }
            }
        };
        let (grant_pointer, move_pointer) = (pointer.clone(), pointer);
        let grant_repeat = repeat;
        let (release_stop, terminate_stop) = (stop.clone(), stop.clone());
        let label = move || {
            let rows = count.get();
            let first = (target.get() / ROW_HEIGHT).floor() as usize;
            let last = ((target.get() + viewport.get()) / ROW_HEIGHT).ceil() as usize;
            format!(
                "Rows {} to {} of {rows}",
                (first + 1).min(rows),
                last.min(rows)
            )
        };
        view()
            .w(px(BAR))
            .hover(|s| s.bg(theme::SURFACE))
            .style_when(move || max.get() <= 0.0, |s| s.hidden())
            .role(AccessibilityRole::Adjustable)
            .label("Scroll markets")
            .accessibility_value(move || {
                Some(AccessibilityValue {
                    min: Some(0.0),
                    max: Some(max.get().round()),
                    now: Some(target.get().round()),
                    text: Some(label().into()),
                })
            })
            .accessibility_actions(vec![
                AccessibilityActionDef {
                    name: "increment".into(),
                    label: None,
                },
                AccessibilityActionDef {
                    name: "decrement".into(),
                    label: None,
                },
            ])
            .on_accessibility_action(move |name: &str| match name {
                "increment" => self.page(1.0),
                "decrement" => self.page(-1.0),
                _ => {}
            })
            .on_wheel(move |event: &WheelEvent| self.wheel(event))
            .claim_touches(TouchClaim::on_start())
            .keep_touches(true)
            .on_responder_grant(move |event: &ResponderEvent| {
                stop();
                let y = event.location.y.0;
                grant_pointer.set(y);
                let direction = towards(y);
                if direction == 0.0 {
                    return;
                }
                self.page(direction);
                let pointer = grant_pointer.clone();
                let held = owner.run(|| {
                    spawn(async move {
                        timer(REPEAT_DELAY).await;
                        while towards(pointer.get()) == direction {
                            self.page(direction);
                            timer(REPEAT_EVERY).await;
                        }
                    })
                });
                *grant_repeat.borrow_mut() = Some(held);
            })
            .on_responder_move(move |event: &ResponderEvent| move_pointer.set(event.location.y.0))
            .on_responder_release(move |_: &ResponderEvent| release_stop())
            .on_responder_terminate(move |_: &ResponderEvent| terminate_stop())
            .child(handle)
    }
}
