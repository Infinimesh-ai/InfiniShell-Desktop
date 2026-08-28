use std::ops::Sub;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Local, Utc};

// Some conversion ratios for time units.
const SEC_TO_MS: f64 = 1000.;
const MIN_TO_MS: f64 = 60. * SEC_TO_MS;
const HOUR_TO_MS: f64 = 60. * MIN_TO_MS;
const DAY_TO_MS: f64 = 24. * HOUR_TO_MS;
const WEEK_TO_MS: f64 = 7. * DAY_TO_MS;
const MONTH_TO_MS: f64 = 30.44 * DAY_TO_MS;
const YEAR_TO_MS: f64 = 365.25 * DAY_TO_MS;

/// Subtract a given DateTime from now and format the duration is a concise, approximated,
/// human-readable form. e.g. "just now"
pub fn format_approx_duration_from_now(datetime: DateTime<Local>) -> String {
    human_readable_approx_duration(Local::now().sub(datetime), false)
}

/// Subtract a given DateTime from now and format the duration is a concise, approximated,
/// human-readable form. e.g. "Just now"
pub fn format_approx_duration_from_now_sentence_case(datetime: DateTime<Local>) -> String {
    human_readable_approx_duration(Local::now().sub(datetime), true)
}

/// Takes a time in UTC and determines roughly how long ago it occurred.
pub fn format_approx_duration_from_now_utc(datetime: DateTime<Utc>) -> String {
    human_readable_approx_duration(Utc::now().sub(datetime), false)
}

/// Format a duration into a human-readable string, e.g. "3.14 sec".
/// Compared to [`human_readable_approx_duration`], this method is for higher-precision, smaller
/// values.
pub fn human_readable_precise_duration(duration: Duration) -> String {
    let ms = duration.num_milliseconds() as f64;
    let weeks = ms / WEEK_TO_MS;
    if weeks >= 1. {
        return crate::t!("time-precise-over-one-week");
    }
    let days = ms / DAY_TO_MS;
    if days >= 1. {
        return crate::t!("time-precise-days", value = format_sigfigs(days, 3));
    }
    let hours = ms / HOUR_TO_MS;
    if hours >= 1. {
        return crate::t!("time-precise-hours", value = format_sigfigs(hours, 3));
    }
    let minutes = ms / MIN_TO_MS;
    if minutes >= 1. {
        return crate::t!("time-precise-minutes", value = format_sigfigs(minutes, 3));
    }
    let seconds = ms / SEC_TO_MS;
    if seconds >= 1. {
        return crate::t!("time-precise-seconds", value = format_sigfigs(seconds, 3));
    }
    crate::t!(
        "time-precise-milliseconds",
        value = duration.num_milliseconds()
    )
}

fn format_sigfigs(num: f64, sigfigs: usize) -> String {
    let a = num.abs();
    let precision = if a > 1. {
        let n = (1. + a.log10().floor()) as usize;
        sigfigs.saturating_sub(n)
    } else if a > 0. {
        let n = -(1. + a.log10().floor()) as usize;
        sigfigs + n
    } else {
        0
    };
    format!("{num:.precision$}")
}

/// Format an approximated duration into a human-readable string, e.g. "2 days ago".
/// Precision is limited to the most significant unit, i.e. 2 days and _n_ hours always displays
/// simply as "2 days ago".
pub fn human_readable_approx_duration(duration: Duration, sentence_case: bool) -> String {
    let ms = duration.num_milliseconds() as f64;
    let years = ms / YEAR_TO_MS;
    if years >= 1. {
        return without_fluent_isolation(crate::t!(
            "time-approx-years-ago",
            count = (years as i32)
        ));
    }
    let months = ms / MONTH_TO_MS;
    if months >= 1. {
        return without_fluent_isolation(crate::t!(
            "time-approx-months-ago",
            count = (months as i32)
        ));
    }
    let weeks = ms / WEEK_TO_MS;
    if weeks >= 1. {
        return without_fluent_isolation(crate::t!(
            "time-approx-weeks-ago",
            count = (weeks as i32)
        ));
    }
    let days = ms / DAY_TO_MS;
    if days >= 1. {
        return without_fluent_isolation(crate::t!("time-approx-days-ago", count = (days as i32)));
    }
    let hours = ms / HOUR_TO_MS;
    if hours >= 1. {
        return without_fluent_isolation(crate::t!(
            "time-approx-hours-ago",
            count = (hours as i32)
        ));
    }
    // Minutes and seconds are both abbreviated, so skip pluralization.
    let minutes = ms / MIN_TO_MS;
    if minutes >= 1. {
        return without_fluent_isolation(crate::t!(
            "time-approx-minutes-ago",
            count = (minutes as i32)
        ));
    }
    if sentence_case {
        crate::t!("time-approx-just-now-sentence")
    } else {
        crate::t!("time-approx-just-now")
    }
}

/// Formats elapsed time as a whole-seconds string with proper singular/plural
/// (e.g. "1 second", "15 seconds").
pub fn format_elapsed_seconds(elapsed: StdDuration) -> String {
    without_fluent_isolation(crate::t!("time-elapsed-seconds", count = elapsed.as_secs()))
}

/// Formats a monotonic `Instant` as a human-readable relative timestamp.
/// (Uses `Instant` rather than wall-clock `DateTime` for elapsed-time display.)
pub fn format_elapsed_since(created_at: instant::Instant) -> String {
    let secs = created_at.elapsed().as_secs();

    if secs < 60 {
        crate::t!("time-approx-just-now-sentence")
    } else if secs < 3600 {
        without_fluent_isolation(crate::t!(
            "time-approx-minutes-ago-long",
            count = (secs / 60)
        ))
    } else if secs < 86400 {
        without_fluent_isolation(crate::t!("time-approx-hours-ago", count = (secs / 3600)))
    } else {
        without_fluent_isolation(crate::t!("time-approx-days-ago", count = (secs / 86400)))
    }
}

fn without_fluent_isolation(text: String) -> String {
    text.replace('\u{2068}', "").replace('\u{2069}', "")
}

#[cfg(test)]
#[path = "time_format_tests.rs"]
mod tests;
