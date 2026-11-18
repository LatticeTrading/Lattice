//! What the widgets that plot a series over time share (the Chart and the
//! Premium/Arb Chart), so that it exists once: the Chart's GPU plot, the
//! old app's `chart-gl` engine, which both of its charts drew with too.
//!
//! - [`viewport`]: where the plot looks ([`Viewport`]: its slots, its pan
//!   and the price range once the user set one);
//! - [`scene`]: what a series, a viewport and a plot size draw ([`Scene`]),
//!   as candles or as a line ([`Mode`]);
//! - [`shader`]: the WGSL program that draws a tile of a scene (which says
//!   how NSH runs it);
//! - [`view`]: the plot's view ([`plot_view`]): its tiles, axes, newest
//!   price, status and crosshair, and the pointer's handling, which a
//!   widget may take presses from ([`Pointer`]: its drawing tools), with
//!   where a widget places what it lays beside and under the plot's
//!   header ([`header_end`], [`UNDER_HEADER`]);
//! - [`timeline`]: when a series' slots are ([`Timeline`]), for what a
//!   widget fixes to a time;
//! - [`Plot`]: a plot's state, which its widget's session keeps, and when
//!   it is drawn.
//!
//! A series is bars, oldest first: candles, or one price a bar for a line.
//! Its feed tells the session of each change from the socket's callback,
//! where the session only marks the plot ([`Plot::marker`]). While the
//! widget shows ([`Plot::drive`]), the session makes the scene the view
//! draws ([`Plot::frame`], [`Plot::show`]) at most once a frame, and at
//! once when the viewport moves or the plot is resized. A hidden plot does
//! none of it. The viewport lives in the session, so a plot is where it
//! was left when its page shows again.

mod scene;
mod shader;
#[cfg(test)]
mod tests;
mod timeline;
mod view;
mod viewport;

use std::{cell::Cell, rc::Rc};

use nsh_core::prelude::*;

pub(crate) use self::{
    scene::{Mode, Scene},
    timeline::Timeline,
    view::{
        AREA_ORIGIN, HEADER_ORIGIN, PlotLook, Pointer, Times, UNDER_HEADER, area, header,
        header_end, plot_view,
    },
    viewport::{DEFAULT_CANDLES, DEFAULT_TICKS, Viewport},
};
use super::Shared;
use crate::data::ferris::Candle;

/// A plot's state: where it looks, and the scene its view shows.
pub(crate) struct Plot {
    /// What the view shows, updated while the widget is visible.
    pub scene: Signal<Shared<Scene>>,
    /// Where the plot looks.
    pub viewport: Signal<Viewport>,
    /// The plot's size in points, which the view measures.
    pub size: Signal<(f32, f32)>,
    /// Where the pointer is over the plot, in points from its top left,
    /// which the view says: what the crosshair follows, and what a widget
    /// shows of its own for the bar under it.
    pub pointer: Signal<Option<(f32, f32)>>,
    /// How many times the user moved the viewport.
    moves: Cell<u64>,
    /// The series changed (or the widget asked) since the scene was made.
    dirty: Rc<Cell<bool>>,
    /// Bumped when `dirty` is set, so a visible plot schedules a frame.
    wake: Signal<u64>,
    mark: Rc<dyn Fn()>,
}

impl Plot {
    /// A plot with nothing to draw yet, which looks at the newest
    /// `visible` slots. Made under the session's owner, which its signals
    /// live as long as.
    pub fn new(mode: Mode, visible: f64) -> Rc<Self> {
        let dirty = Rc::new(Cell::new(true));
        let wake = signal(0_u64);
        let mark: Rc<dyn Fn()> = {
            let (dirty, wakes) = (dirty.clone(), Cell::new(0_u64));
            Rc::new(move || {
                if !dirty.replace(true) {
                    wakes.set(wakes.get().wrapping_add(1));
                    wake.try_set(wakes.get());
                }
            })
        };
        Rc::new(Self {
            scene: signal(Shared::new(Scene::empty(mode))),
            viewport: signal(Viewport::showing(visible)),
            size: signal((1.0_f32, 1.0_f32)),
            pointer: signal(None),
            moves: Cell::new(0),
            dirty,
            wake,
            mark,
        })
    }

    /// Says the series changed since the plot drew it. What a feed's
    /// `on_change` calls: it only sets a flag.
    pub fn marker(&self) -> Rc<dyn Fn()> {
        self.mark.clone()
    }

    /// [`Self::marker`], called.
    pub fn mark(&self) {
        (self.mark)();
    }

    /// How many times the user moved the viewport ([`Self::move_view`]).
    pub fn moves(&self) -> u64 {
        self.moves.get()
    }

    /// Moves the viewport as the user asked: `change` gets it and the
    /// series' length, and gives the next one.
    pub fn move_view(&self, change: impl FnOnce(Viewport, usize) -> Viewport) {
        let bars = self.scene.with_untracked(|scene| scene.0.bars);
        self.moves.set(self.moves.get() + 1);
        self.viewport
            .set(change(self.viewport.get_untracked(), bars));
    }

    /// What the plot shows of `bars`, of which `added` came in after the
    /// newest since it last drew: the viewport kept on the bars it looked
    /// at, and their scene in `mode`. [`Self::show`] shows them.
    pub fn frame(&self, bars: &[Candle], added: u64, mode: Mode) -> (Viewport, Scene) {
        self.frame_with(bars, added, mode, |_, _| None)
    }

    /// [`Self::frame`] for a widget that draws more than the bars over
    /// them: `also` says how low and high that goes on the bars under the
    /// plot, as [`Scene::build_with`] asks it.
    pub fn frame_with(
        &self,
        bars: &[Candle],
        added: u64,
        mode: Mode,
        also: impl FnOnce(usize, usize) -> Option<(f64, f64)>,
    ) -> (Viewport, Scene) {
        let held = self.viewport.get_untracked().held(added, bars.len());
        let (width, height) = self.size.get_untracked();
        (
            held,
            Scene::build_with(bars, &held, mode, width, height, also),
        )
    }

    /// Shows what [`Self::frame`] made. Call it in the `batch` that sets
    /// what else the view shows.
    pub fn show(&self, held: Viewport, scene: Scene) {
        if self.viewport.with_untracked(|view| *view != held) {
            self.viewport.try_set(held);
        }
        self.scene.try_set(Shared::new(scene));
    }

    /// Runs `draw` a frame after the plot was marked, at most once a frame,
    /// and at once when the viewport moves or the plot is resized, under
    /// the current owner: call it where the widget's rendering work starts
    /// (`SessionScope::while_visible`). `draw` makes the scene and shows it.
    pub fn drive(&self, draw: impl Fn() + 'static) {
        let draw: Rc<dyn Fn()> = {
            let dirty = self.dirty.clone();
            Rc::new(move || {
                dirty.set(false);
                draw();
            })
        };
        // New bars: at most once a frame.
        {
            let (wake, dirty, draw) = (self.wake, self.dirty.clone(), draw.clone());
            effect(move || {
                wake.get();
                if dirty.get() {
                    let (draw, dirty) = (draw.clone(), dirty.clone());
                    spawn(async move {
                        next_frame().await;
                        if dirty.get() {
                            draw();
                        }
                    });
                }
            });
        }
        // A drag, a zoom or a resize: at once.
        let (viewport, size) = (self.viewport, self.size);
        effect(move || {
            viewport.get();
            size.get();
            untracked(|| draw());
        });
    }
}
