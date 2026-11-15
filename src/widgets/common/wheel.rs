//! Wheel events as steps: what the plots that zoom by the wheel (the
//! chart, the depth chart) scale by.

use nsh_core::prelude::*;

/// How far a wheel event turns: a notch of a mouse wheel is one step
/// whatever its size (the old chart zoomed by the wheel's direction), a
/// touchpad's points are a hundred to the step. Down and right are
/// positive; Shift may turn a wheel sideways, so the larger axis counts.
pub(crate) fn wheel_steps(event: &WheelEvent) -> f64 {
    let delta = event.delta.points(px(1.0));
    let turned = if delta.x.0.abs() > delta.y.0.abs() {
        delta.x.0
    } else {
        delta.y.0
    };
    if turned == 0.0 {
        0.0
    } else if event.delta.precise() {
        f64::from(turned) / 100.0
    } else {
        f64::from(turned.signum())
    }
}

/// The factor `steps` of the wheel scale by, `out` a step (the old chart's
/// 1.1 out and 0.9 in for slots, 1.08 and 0.92 for prices).
pub(crate) fn wheel_factor(steps: f64, out: f64) -> f64 {
    if steps >= 0.0 {
        out.powf(steps)
    } else {
        (2.0 - out).powf(-steps)
    }
}
