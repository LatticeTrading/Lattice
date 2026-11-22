//! The tool in hand and what the pointer does with it ([`Editor`]), which
//! the plot asks before it pans ([`Pointer`]).
//!
//! With a drawing tool, the pointer makes a drawing ([`Draft`]): a press
//! places its first point, and the next follows the pointer, which places
//! it where the press is let go once it was dragged, and otherwise where
//! the next press is (so a drag makes a line, and so do two clicks; a
//! kind of three points takes one press more, for its third). With
//! its last point placed, a kept kind is saved in the chart's settings and
//! picked, another (a measurement) shows until the next press, and the
//! cursor is back in hand.
//!
//! With the cursor, a press on a drawing picks it ([`hit`]: on one of its
//! points takes that point, on its lines all of it) and a drag then moves
//! what was taken ([`moved`]), which is saved when the press is let go. A
//! press on nothing picks nothing and is the plot's.
//!
//! Escape puts away what is being made, the measurement and the tool, and
//! with none of them the pick (and goes on to whatever else it closes);
//! Delete and Backspace remove the picked drawing. The keys are the
//! editor's only while it has something for them, so of two charts the
//! one used last hears them.

use std::{cell::RefCell, rc::Rc};

use nsh_core::prelude::*;

use super::{
    Anchor, Drawing, Kind, Tool,
    layer::{Emphasis, Item, paint},
    market_key,
    scale::Scale,
    shape::{Ink, Spot},
};
use crate::widgets::{
    TabContext,
    chart::{ChartSettings, session::ChartSession},
    common::{CLICK_SLOP, plot::Pointer},
};

/// How near one of a drawing's points, and its lines, the pointer takes
/// them, in points.
const POINT_REACH: f64 = 8.0;
const LINE_REACH: f64 = 5.0;

/// A drawing being made: the points placed so far, then the one that
/// follows the pointer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Draft {
    kind: Kind,
    points: Vec<Anchor>,
}

impl Draft {
    /// A `kind` whose first point is `at`, where the next one starts.
    pub fn begin(kind: Kind, at: Anchor) -> Self {
        let points = vec![at; kind.anchors().min(2)];
        Self { kind, points }
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Its points so far, the last of them the one following the pointer.
    pub fn points(&self) -> &[Anchor] {
        &self.points
    }

    /// Whether a click (a press let go where it began) leaves its first
    /// point placed and the next following the pointer: one of a single
    /// point is made by the click.
    fn takes_clicks(&self) -> bool {
        self.kind.anchors() > 1
    }

    /// Moves the point that follows the pointer to `at`.
    pub fn aim(&mut self, at: Anchor) {
        if let Some(last) = self.points.last_mut() {
            *last = at;
        }
    }

    /// Places the point that follows the pointer where it is: the
    /// drawing's points once that was the last, and otherwise another
    /// point follows from there.
    pub fn place(&mut self) -> Option<Vec<Anchor>> {
        if self.points.len() >= self.kind.anchors() {
            return Some(std::mem::take(&mut self.points));
        }
        self.points.extend(self.points.last().copied());
        None
    }
}

/// What a press takes of a drawing: one of its points, or all of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Grip {
    Point(usize),
    Body,
}

/// What a press at `at` (points from the plot's top left) takes of
/// `drawings`, each with its number, in the order painted: a point of the
/// `picked` one first (its handles show), else of the topmost drawing with
/// a point or a line there.
pub(crate) fn hit(
    scale: &Scale<'_>,
    drawings: &[(usize, &Drawing)],
    picked: Option<usize>,
    at: (f64, f64),
) -> Option<(usize, Grip)> {
    // The point of `drawing` grabbed there: where its kind has it grabbed.
    let point = |drawing: &Drawing, spots: &[Spot]| {
        let near = |grip: &(f64, f64)| (grip.0 - at.0).hypot(grip.1 - at.1) <= POINT_REACH;
        let grips = drawing.kind().shape().grips(spots);
        grips.iter().position(near).map(Grip::Point)
    };
    let placed: Vec<_> = drawings
        .iter()
        .filter_map(|(number, drawing)| Some((*number, *drawing, scale.spots(drawing.points())?)))
        .collect();
    let of_picked = placed
        .iter()
        .filter(|(number, ..)| Some(*number) == picked)
        .find_map(|(number, drawing, spots)| Some((*number, point(drawing, spots)?)));
    of_picked.or_else(|| {
        placed.iter().rev().find_map(|(number, drawing, spots)| {
            let on_line = || drawing.kind().shape().distance(spots, at) <= LINE_REACH;
            let grip = point(drawing, spots).or_else(|| on_line().then_some(Grip::Body))?;
            Some((*number, grip))
        })
    })
}

/// `points` once the pointer that took them by `grip` at the point
/// `grabbed` is at `to` (points from the plot's top left): what was taken
/// moves by as much as the pointer did on the plot as it is now, so it
/// stays under the pointer while the plot itself moves.
pub(crate) fn moved(
    scale: &Scale<'_>,
    points: &[Anchor],
    grip: Grip,
    grabbed: Anchor,
    to: (f64, f64),
) -> Option<Vec<Anchor>> {
    let held = scale.spot(grabbed)?;
    let (dx, dy) = (to.0 - held.x, to.1 - held.y);
    let points = points.iter().enumerate();
    let moved = points.map(|(number, point)| match grip {
        Grip::Point(taken) if taken != number => Some(*point),
        _ => {
            let spot = scale.spot(*point)?;
            scale.anchor(spot.x + dx, spot.y + dy)
        }
    });
    moved.collect()
}

/// What is drawn in place of the saved drawings while the pointer works.
#[derive(Clone, Debug, PartialEq)]
enum Work {
    /// A new drawing.
    Making(Draft),
    /// The saved drawing of this number, with its points where the
    /// pointer moved them so far.
    Moving(usize, Vec<Anchor>),
}

/// The press the editor took, which is still down.
#[derive(Clone)]
enum Press {
    /// It places a point of the draft: where it began, and whether it
    /// began the draft too.
    Draw { from: (f32, f32), began: bool },
    /// It took the saved drawing of this number by `grip`: where it began,
    /// the point of the plot under it then, and the drawing's points then.
    Move {
        number: usize,
        grip: Grip,
        from: (f32, f32),
        grabbed: Anchor,
        points: Vec<Anchor>,
    },
}

/// Whether a press that began at `from` and is at `to` was dragged.
fn dragged(from: (f32, f32), to: (f32, f32)) -> bool {
    (to.0 - from.0).abs().max((to.1 - from.1).abs()) > CLICK_SLOP
}

/// Sets `signal` to `value` unless it holds it already.
fn put<T: Clone + PartialEq + 'static>(signal: Signal<T>, value: T) {
    if signal.with_untracked(|held| *held != value) {
        signal.set(value);
    }
}

/// The drawing tools of one chart's view: the tool in hand, the drawing
/// picked, and what the pointer is making or moving. The drawings
/// themselves are the settings'.
pub(crate) struct Editor {
    session: Rc<ChartSession>,
    tab: TabContext<ChartSettings>,
    /// The market charted, as drawings name theirs.
    market: Memo<String>,
    /// The tool in hand.
    tool: Signal<Tool>,
    work: Signal<Option<Work>>,
    /// The number of the saved drawing that is picked, and of the one
    /// under the pointer.
    picked: Signal<Option<usize>>,
    hovered: Signal<Option<usize>>,
    /// The measurement last made, which shows until the next press.
    measured: Signal<Option<(Kind, Vec<Anchor>)>>,
    press: RefCell<Option<Press>>,
}

impl Editor {
    /// The drawing tools of the view of `session` in `tab`, under the
    /// view's owner.
    pub fn new(session: &Rc<ChartSession>, tab: &TabContext<ChartSettings>) -> Rc<Self> {
        let saved = tab.settings;
        let market = computed(move || saved.with(|settings| market_key(&settings.market())));
        let editor = Rc::new(Self {
            session: session.clone(),
            tab: tab.clone(),
            market,
            tool: signal(Tool::Cursor),
            work: signal(None),
            picked: signal(None),
            hovered: signal(None),
            measured: signal(None),
            press: RefCell::new(None),
        });
        // Another market's drawings are other drawings: nothing of the
        // one charted before is in hand.
        {
            let editor = editor.clone();
            effect(move || {
                market.get();
                untracked(|| editor.put_away(true));
            });
        }
        editor.bind_keys();
        editor
    }

    /// Escape, Delete and Backspace, each only while the chart shows and
    /// the editor has something for it.
    fn bind_keys(self: &Rc<Self>) {
        let shown = self.tab.shown;
        let (tool, work, measured, picked) = (self.tool, self.work, self.measured, self.picked);
        let picks = computed(move || shown.get() && picked.get().is_some());
        let busy = computed(move || {
            let in_hand = tool.get() != Tool::Cursor
                || work.with(Option::is_some)
                || measured.with(Option::is_some);
            picks.get() || (shown.get() && in_hand)
        });
        let editor = self.clone();
        effect(move || {
            if !busy.get() {
                return;
            }
            // Escape goes on after it, so what else it closes (the
            // settings panel, a list) closes with the same press.
            let passed = KeyOptions {
                pass_through: true,
                ..KeyOptions::default()
            };
            let escaped = editor.clone();
            on_key_with("escape", passed, move |_| escaped.escape());
            if picks.get() {
                for key in ["delete", "backspace"] {
                    let removed = editor.clone();
                    on_key(key, move |_| removed.remove_picked());
                }
            }
        });
    }

    /// The tool in hand (tracked).
    pub fn tool(&self) -> Tool {
        self.tool.get()
    }

    /// Takes `tool` in hand, or the cursor when it is in hand already;
    /// what was being made goes.
    pub fn take(&self, tool: Tool) {
        let next = if self.tool.get_untracked() == tool {
            Tool::Cursor
        } else {
            tool
        };
        batch(|| {
            self.put_away(false);
            put(self.tool, next);
        });
    }

    /// How many drawings the market charted has (tracked).
    pub fn count(&self) -> usize {
        let market = self.market.get();
        let saved = self.tab.settings;
        saved.with(|settings| {
            let drawings = settings.drawings().iter();
            drawings
                .filter(|drawing| drawing.market() == market)
                .count()
        })
    }

    /// Removes every drawing of the market charted.
    pub fn remove_all(&self) {
        let market = self.market.get_untracked();
        let settings = self.tab.settings.get_untracked();
        let kept: Vec<Drawing> = settings
            .drawings()
            .iter()
            .filter(|drawing| drawing.market() != market)
            .cloned()
            .collect();
        batch(|| {
            self.put_away(true);
            self.tab.save(&settings.with_drawings(kept));
        });
    }

    /// Removes the picked drawing.
    fn remove_picked(&self) {
        let Some(number) = self.picked.get_untracked() else {
            return;
        };
        let market = self.market.get_untracked();
        let settings = self.tab.settings.get_untracked();
        let mut drawings = settings.drawings().to_vec();
        let ours = drawings
            .get(number)
            .is_some_and(|drawing| drawing.market() == market);
        batch(|| {
            self.put_away(true);
            if ours {
                drawings.remove(number);
                self.tab.save(&settings.with_drawings(drawings));
            }
        });
    }

    /// Escape: what is being made, the measurement and the tool go; with
    /// none of them in hand, the pick does.
    fn escape(&self) {
        let in_hand = self.tool.get_untracked() != Tool::Cursor
            || self.work.with_untracked(Option::is_some)
            || self.measured.with_untracked(Option::is_some);
        batch(|| {
            self.put_away(!in_hand);
            put(self.tool, Tool::Cursor);
        });
    }

    /// Puts away what the pointer is making or moving and the measurement
    /// last made, and with `pick` what is picked and under the pointer too.
    fn put_away(&self, pick: bool) {
        *self.press.borrow_mut() = None;
        put(self.work, None);
        put(self.measured, None);
        if pick {
            put(self.picked, None);
            put(self.hovered, None);
        }
    }

    /// Calls `f` with the plot's scale now; nothing before the chart has a
    /// series. `f` only reads: the series is borrowed while it runs.
    fn with_scale<R>(&self, f: impl FnOnce(&Scale<'_>) -> R) -> Option<R> {
        let scene = self.session.plot.scene.get_untracked();
        self.session
            .with_times(|times| f(&Scale::new(&scene.0, times)))
    }

    /// The point of the plot at `at`; none while it has no bar.
    fn anchor(&self, at: (f32, f32)) -> Option<Anchor> {
        self.with_scale(|scale| scale.anchor(f64::from(at.0), f64::from(at.1)))?
    }

    /// What a press at `at` takes of the market's drawings.
    fn hit(&self, at: (f32, f32)) -> Option<(usize, Grip)> {
        let picked = self.picked.get_untracked();
        let at = (f64::from(at.0), f64::from(at.1));
        self.market.with_untracked(|market| {
            self.tab.settings.with_untracked(|settings| {
                let drawings: Vec<(usize, &Drawing)> = settings
                    .drawings()
                    .iter()
                    .enumerate()
                    .filter(|(_, drawing)| drawing.market() == market)
                    .collect();
                if drawings.is_empty() {
                    return None;
                }
                self.with_scale(|scale| hit(scale, &drawings, picked, at))?
            })
        })
    }

    /// The draft's last point was placed: its drawing is made of `points`.
    fn finish(&self, kind: Kind, points: Vec<Anchor>) {
        put(self.work, None);
        // Points all in one place draw nothing: the tool stays in hand
        // for another go.
        if points.len() > 1 && points.windows(2).all(|pair| pair[0] == pair[1]) {
            return;
        }
        put(self.tool, Tool::Cursor);
        if !kind.kept() {
            self.measured.set(Some((kind, points)));
            return;
        }
        let settings = self.tab.settings.get_untracked();
        let Some(drawing) = Drawing::new(kind, &self.market.get_untracked(), points) else {
            return;
        };
        let mut drawings = settings.drawings().to_vec();
        drawings.push(drawing);
        put(self.picked, Some(drawings.len() - 1));
        self.tab.save(&settings.with_drawings(drawings));
    }

    /// Saves the drawing of this `number` with its points at `points`.
    fn save_moved(&self, number: usize, points: Vec<Anchor>) {
        let settings = self.tab.settings.get_untracked();
        let mut drawings = settings.drawings().to_vec();
        let moved = drawings
            .get(number)
            .and_then(|drawing| drawing.with_points(points));
        if let Some(moved) = moved {
            drawings[number] = moved;
            self.tab.save(&settings.with_drawings(drawings));
        }
    }

    /// What the layer paints (tracked): the market's saved drawings, the
    /// one being moved where the pointer has it, then the measurement and
    /// the one being made; nothing while there is none of them, so the
    /// plot's moves paint nothing then.
    pub fn painted(&self) -> Option<Painter> {
        let market = self.market.get();
        let (work, picked, hovered) = (self.work.get(), self.picked.get(), self.hovered.get());
        let saved = self.tab.settings;
        let mut items = saved.with(|settings| {
            let ours = settings.drawings().iter().enumerate();
            let ours = ours.filter(|(_, drawing)| drawing.market() == market);
            ours.map(|(number, drawing)| {
                let (points, emphasis) = match &work {
                    Some(Work::Moving(moving, points)) if *moving == number => {
                        (points.clone(), Emphasis::Picked)
                    }
                    _ if picked == Some(number) => (drawing.points().to_vec(), Emphasis::Picked),
                    _ if hovered == Some(number) => (drawing.points().to_vec(), Emphasis::Hovered),
                    _ => (drawing.points().to_vec(), Emphasis::Plain),
                };
                Item {
                    kind: drawing.kind(),
                    points,
                    emphasis,
                }
            })
            .collect::<Vec<_>>()
        });
        items.extend(self.measured.get().map(|(kind, points)| Item {
            kind,
            points,
            emphasis: Emphasis::Plain,
        }));
        if let Some(Work::Making(draft)) = &work {
            items.push(Item {
                kind: draft.kind(),
                points: draft.points().to_vec(),
                emphasis: Emphasis::Picked,
            });
        }
        if items.is_empty() {
            return None;
        }
        let ink = Ink::themed();
        let scene = self.session.plot.scene.get();
        self.session
            .with_times(|times| paint(&Scale::new(&scene.0, times), &items, &ink))
    }
}

impl Pointer for Editor {
    fn hover(&self, at: Option<(f32, f32)>) {
        if self.press.borrow().is_some() {
            return;
        }
        if let Some(Work::Making(mut draft)) = self.work.get_untracked() {
            // The draft's next point follows the pointer.
            if let Some(anchor) = at.and_then(|at| self.anchor(at)) {
                draft.aim(anchor);
                put(self.work, Some(Work::Making(draft)));
            }
            return;
        }
        let over = match self.tool.get_untracked() {
            Tool::Cursor => at.and_then(|at| self.hit(at)).map(|(number, _)| number),
            Tool::Draw(_) => None,
        };
        put(self.hovered, over);
    }

    fn press(&self, at: (f32, f32)) -> bool {
        // A measurement shows until the next press.
        put(self.measured, None);
        match self.tool.get_untracked() {
            Tool::Draw(kind) => {
                // Without a bar there is nowhere to draw: the plot's.
                let Some(anchor) = self.anchor(at) else {
                    return false;
                };
                let (draft, began) = match self.work.get_untracked() {
                    Some(Work::Making(mut draft)) => {
                        draft.aim(anchor);
                        (draft, false)
                    }
                    _ => (Draft::begin(kind, anchor), true),
                };
                *self.press.borrow_mut() = Some(Press::Draw { from: at, began });
                batch(|| {
                    put(self.picked, None);
                    put(self.hovered, None);
                    self.work.set(Some(Work::Making(draft)));
                });
                true
            }
            Tool::Cursor => {
                let Some((number, grip)) = self.hit(at) else {
                    put(self.picked, None);
                    return false;
                };
                let settings = self.tab.settings.get_untracked();
                let (Some(drawing), Some(grabbed)) =
                    (settings.drawings().get(number), self.anchor(at))
                else {
                    return false;
                };
                *self.press.borrow_mut() = Some(Press::Move {
                    number,
                    grip,
                    from: at,
                    grabbed,
                    points: drawing.points().to_vec(),
                });
                put(self.picked, Some(number));
                true
            }
        }
    }

    fn drag(&self, at: (f32, f32)) {
        let Some(press) = self.press.borrow().clone() else {
            return;
        };
        match press {
            Press::Draw { .. } => {
                if let (Some(Work::Making(mut draft)), Some(anchor)) =
                    (self.work.get_untracked(), self.anchor(at))
                {
                    draft.aim(anchor);
                    put(self.work, Some(Work::Making(draft)));
                }
            }
            Press::Move {
                number,
                grip,
                from,
                grabbed,
                points,
            } => {
                // A click picks; only a drag moves.
                let moving = matches!(self.work.get_untracked(), Some(Work::Moving(..)));
                if !moving && !dragged(from, at) {
                    return;
                }
                let to = (f64::from(at.0), f64::from(at.1));
                let next = self.with_scale(|scale| moved(scale, &points, grip, grabbed, to));
                if let Some(Some(next)) = next {
                    put(self.work, Some(Work::Moving(number, next)));
                }
            }
        }
    }

    fn release(&self, at: Option<(f32, f32)>) {
        let Some(press) = self.press.borrow_mut().take() else {
            return;
        };
        let work = self.work.get_untracked();
        match (press, work) {
            (Press::Draw { from, began }, Some(Work::Making(mut draft))) => {
                let Some(at) = at else {
                    // Taken away: the draft goes with the press that
                    // began it.
                    if began {
                        put(self.work, None);
                    }
                    return;
                };
                if let Some(anchor) = self.anchor(at) {
                    draft.aim(anchor);
                }
                if began && draft.takes_clicks() && !dragged(from, at) {
                    // A click: its first point is placed, and the next
                    // follows the pointer to the next press.
                    put(self.work, Some(Work::Making(draft)));
                    return;
                }
                let kind = draft.kind();
                batch(|| match draft.place() {
                    Some(points) => self.finish(kind, points),
                    None => put(self.work, Some(Work::Making(draft))),
                });
            }
            (Press::Move { number, .. }, Some(Work::Moving(moving, points))) => batch(|| {
                put(self.work, None);
                if at.is_some() && moving == number {
                    self.save_moved(number, points);
                }
            }),
            _ => {}
        }
    }

    fn grabs(&self) -> bool {
        self.tool.get() == Tool::Cursor && self.hovered.get().is_some()
    }
}
