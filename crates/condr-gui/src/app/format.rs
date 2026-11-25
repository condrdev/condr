//! Text the interface writes for a quantity: how long ago something was, and a size in
//! bytes.

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

/// What `relative_age` says of a commit time, which Git gives signed.
pub(super) fn age(time: i64) -> Cow<'static, str> {
    relative_age(u64::try_from(time).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::file_size_text;
    #[test]
    fn sizes_read_in_the_largest_whole_unit() {
        assert_eq!(file_size_text(512), "512 B");
        assert_eq!(file_size_text(1536), "1.5 KiB");
        assert_eq!(file_size_text(3 << 20), "3.0 MiB");
    }
}
