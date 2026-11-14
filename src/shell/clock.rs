//! The Everything Bar's clock and its Settings: 24- or 12-hour time, at a
//! fixed UTC offset (UTC+0 unless the user picks another).
//!
//! The time is read from NSH's wall clock (`nsh_core::time::unix_ms`), which
//! works on desktop and in WebAssembly alike, and shifted by the chosen
//! offset. Offsets are fixed, so they do not follow daylight saving time.

use std::time::Duration;

use nsh_core::{element::timer, prelude::*, time::unix_ms};

use crate::storage::{self, StoredClock};

/// The offsets Settings offers, in minutes east of UTC: every whole hour
/// from UTC-12 to UTC+14, plus the half- and quarter-hour zones in use.
pub(crate) const OFFSETS: &[i16] = &[
    -720, -660, -600, -570, -540, -480, -420, -360, -300, -240, -210, -180, -120, -60, 0, 60, 120,
    180, 210, 240, 270, 300, 330, 345, 360, 390, 420, 480, 525, 540, 570, 600, 630, 660, 720, 765,
    780, 840,
];

/// How the clock reads. The default is 24-hour UTC+0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ClockPrefs {
    pub hour12: bool,
    /// Minutes east of UTC; always one of [`OFFSETS`].
    pub offset: i16,
}

impl ClockPrefs {
    fn from_stored(stored: &StoredClock) -> Self {
        Self {
            hour12: stored.hour12,
            offset: if OFFSETS.contains(&stored.utc_offset_minutes) {
                stored.utc_offset_minutes
            } else {
                0
            },
        }
    }
}

/// The clock's preferences, which Settings changes and the bar reads.
#[derive(Clone, Copy)]
pub(crate) struct Clock(Signal<ClockPrefs>);

impl Clock {
    /// The saved preferences, or the default when none (or an offset no
    /// longer offered) is saved.
    pub(crate) fn restore() -> Self {
        Self(signal(ClockPrefs::from_stored(
            &storage::load_settings().clock,
        )))
    }

    /// The preferences now (tracked).
    pub(crate) fn get(self) -> ClockPrefs {
        self.0.get()
    }

    /// Changes the preferences and saves them.
    pub(crate) fn update(self, change: impl FnOnce(&mut ClockPrefs)) {
        let mut prefs = self.0.get_untracked();
        change(&mut prefs);
        if prefs == self.0.get_untracked() {
            return;
        }
        self.0.set(prefs);
        let mut settings = storage::load_settings();
        settings.clock = StoredClock {
            hour12: prefs.hour12,
            utc_offset_minutes: prefs.offset,
        };
        if let Err(error) = storage::save_settings(&settings) {
            log::warn!("Lattice: unable to save settings: {error}");
        }
    }
}

/// Whole seconds since the Unix epoch, on the wall clock.
fn now_secs() -> i64 {
    (unix_ms() / 1000.0).floor() as i64
}

/// A signal holding the current second, which ticks as each second starts
/// for as long as the current owner lives.
pub(crate) fn ticking_seconds() -> Signal<i64> {
    let now = signal(now_secs());
    spawn(async move {
        loop {
            let into_second = unix_ms().rem_euclid(1000.0);
            // A hair past the boundary, so the read lands in the new second.
            timer(Duration::from_millis((1000.0 - into_second) as u64 + 5)).await;
            now.set(now_secs());
        }
    });
    now
}

/// `secs` (since the Unix epoch) as the clock shows it: `14:05:09`, or
/// `02:05:09 PM` in 12-hour time.
pub(crate) fn format_time(secs: i64, prefs: ClockPrefs) -> String {
    let local = (secs + i64::from(prefs.offset) * 60).rem_euclid(86_400);
    let (hour, minute, second) = (local / 3600, local / 60 % 60, local % 60);
    if prefs.hour12 {
        let suffix = if hour < 12 { "AM" } else { "PM" };
        let hour = match hour % 12 {
            0 => 12,
            hour => hour,
        };
        format!("{hour:02}:{minute:02}:{second:02} {suffix}")
    } else {
        format!("{hour:02}:{minute:02}:{second:02}")
    }
}

/// `secs` (since the Unix epoch) as a day and a time of day on the clock,
/// without seconds: `Nov 14 14:05`, or `Nov 14 02:05 PM` in 12-hour time
/// (a chart's time axis).
pub(crate) fn format_day_time(secs: i64, prefs: ClockPrefs) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let local = secs + i64::from(prefs.offset) * 60;
    let (_, month, day) = crate::format::civil_date(local.div_euclid(86_400));
    let month = MONTHS[(month - 1).clamp(0, 11) as usize];
    let time = format_time(secs, prefs);
    // `14:05:09` or `02:05:09 PM`, less its seconds.
    let (clock, suffix) = time.split_at(8.min(time.len()));
    format!("{month} {day} {}{suffix}", &clock[..5.min(clock.len())])
}

/// An offset's name: `UTC+0`, `UTC-6`, `UTC+5:30`.
pub(crate) fn offset_label(offset: i16) -> String {
    let sign = if offset < 0 { '-' } else { '+' };
    let (hours, minutes) = (offset.abs() / 60, offset.abs() % 60);
    if minutes == 0 {
        format!("UTC{sign}{hours}")
    } else {
        format!("UTC{sign}{hours}:{minutes:02}")
    }
}
