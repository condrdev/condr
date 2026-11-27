//! Text the interface writes for a quantity: how long ago something was, when a commit was
//! made, and a size in bytes.

use chrono::{DateTime, Local, TimeZone};
use condr_core::protocol::RelativeAge;
use rust_i18n::t;
use std::borrow::Cow;

/// How long ago `unix_seconds` was, in the steps the CLI's `relative_age` takes and the
/// interface's words; Settings uses it too.
pub(super) fn relative_age(unix_seconds: u64) -> Cow<'static, str> {
    match RelativeAge::since(unix_seconds) {
        RelativeAge::JustNow => t!("time.just_now"),
        RelativeAge::Minutes(count) if count == 1 => t!("time.minutes_ago.one", count = count),
        RelativeAge::Minutes(count) => t!("time.minutes_ago.other", count = count),
        RelativeAge::Hours(count) if count == 1 => t!("time.hours_ago.one", count = count),
        RelativeAge::Hours(count) => t!("time.hours_ago.other", count = count),
        RelativeAge::Days(count) if count == 1 => t!("time.days_ago.one", count = count),
        RelativeAge::Days(count) => t!("time.days_ago.other", count = count),
    }
}

pub(super) fn file_size_text(bytes: usize) -> String {
    const KIB: f64 = 1024.;
    match bytes as f64 {
        size if size < KIB => format!("{bytes} B"),
        size if size < KIB * KIB => format!("{:.1} KiB", size / KIB),
        size => format!("{:.1} MiB", size / (KIB * KIB)),
    }
}

/// A commit's time in local time.
fn local_time(time: i64) -> Option<DateTime<Local>> {
    Local.timestamp_opt(time, 0).single()
}

/// A commit's date as Git Graph's table shows it, as IntelliJ's log does (ADR 0079): within
/// the hour today, how many minutes ago; then "Today 14:25" and "Yesterday 14:25"; else
/// the date and time.
pub(super) fn commit_date(time: i64, now: DateTime<Local>) -> String {
    let Some(at) = local_time(time) else {
        return String::new();
    };
    let clock = at.format("%H:%M").to_string();
    let today = now.date_naive();
    if at.date_naive() == today {
        let seconds = (now - at).num_seconds();
        if (0..=3600).contains(&seconds) {
            let minutes = (seconds as f64 / 60.).round() as i64;
            return match minutes {
                0 => t!("time.moments_ago"),
                1 => t!("time.a_minute_ago"),
                60 => t!("time.an_hour_ago"),
                count => t!("time.minutes_ago.other", count = count),
            }
            .into_owned();
        }
        return t!("time.today_at", time = clock).into_owned();
    }
    if today.pred_opt() == Some(at.date_naive()) {
        return t!("time.yesterday_at", time = clock).into_owned();
    }
    at.format("%Y/%m/%d %H:%M").to_string()
}

/// A commit's date and time of day apart, for "on 2026/11/25 at 14:25".
pub(super) fn commit_day_and_clock(time: i64) -> (String, String) {
    local_time(time).map_or_else(Default::default, |at| {
        (
            at.format("%Y/%m/%d").to_string(),
            at.format("%H:%M").to_string(),
        )
    })
}

/// What `relative_age` says of a commit time, which Git gives signed.
pub(super) fn age(time: i64) -> Cow<'static, str> {
    relative_age(u64::try_from(time).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::{commit_date, file_size_text};
    use chrono::{Duration, Local, TimeZone};

    #[test]
    fn commit_dates_read_as_intellij_writes_them() {
        let now = Local.with_ymd_and_hms(2026, 11, 25, 14, 25, 0).unwrap();
        let ago = |duration: Duration| commit_date((now - duration).timestamp(), now);
        assert_eq!(ago(Duration::seconds(20)), "Moments ago");
        assert_eq!(ago(Duration::seconds(70)), "A minute ago");
        assert_eq!(ago(Duration::minutes(12)), "12 minutes ago");
        assert_eq!(ago(Duration::minutes(60)), "1 hour ago");
        assert_eq!(ago(Duration::minutes(61)), "Today 13:24");
        assert_eq!(ago(Duration::hours(20)), "Yesterday 18:25");
        assert_eq!(ago(Duration::days(9)), "2026/11/16 14:25");
    }

    #[test]
    fn sizes_read_in_the_largest_whole_unit() {
        assert_eq!(file_size_text(512), "512 B");
        assert_eq!(file_size_text(1536), "1.5 KiB");
        assert_eq!(file_size_text(3 << 20), "3.0 MiB");
    }
}
