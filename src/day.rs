//! Turning a day description into a calendar date, and rendering dates
//! into daily note file names.

use jiff::ToSpan;
use jiff::civil::{Date, DateTime};
use jiff::fmt::strtime;

/// Format for daily note file names when `daily-date-format` is unset.
pub const DEFAULT_FORMAT: &str = "%Y-%m-%d";

/// Format for stamp values when `stamp-format` is unset: a shape
/// markdown editors read as a date and time.
pub const DEFAULT_STAMP_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";

/// Failure while interpreting a day description or a date format.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "invalid date \"{value}\": expected today, yesterday, tomorrow, or YYYY-MM-DD ({cause})"
    )]
    Invalid { value: String, cause: jiff::Error },
    #[error("date format \"{value}\" is invalid: {cause}")]
    InvalidFormat { value: String, cause: jiff::Error },
    #[error("date format \"{value}\" renders an empty file name")]
    EmptyFormat { value: String },
    #[error("date format \"{value}\" renders a line break into a file name")]
    LineBreakFormat { value: String },
    #[error("timestamp format \"{value}\" is invalid: {cause}")]
    InvalidStamp { value: String, cause: jiff::Error },
    #[error("timestamp format \"{value}\" renders nothing")]
    EmptyStamp { value: String },
    #[error("timestamp format \"{value}\" renders an unprintable character")]
    UnprintableStamp { value: String },
}

/// A strftime format proven, at construction, to render a date to a
/// non-empty file name. Carrying the proof in the type means rendering
/// cannot fail later, when a daily note is targeted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Format(String);

impl Format {
    /// Validates `value` by rendering a probe date through it.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` cannot render a date (an unknown
    /// directive, or one needing more than a calendar date, like `%H`),
    /// renders an empty file name, or renders a line break (`%n`) into
    /// the file name — a note named across lines would break the
    /// one-path-per-line output of the listing commands.
    pub fn new(value: &str) -> Result<Self, Error> {
        let probe = jiff::civil::date(2001, 2, 3);
        let rendered = strtime::format(value, probe).map_err(|cause| Error::InvalidFormat {
            value: value.to_owned(),
            cause,
        })?;
        if rendered.is_empty() {
            return Err(Error::EmptyFormat {
                value: value.to_owned(),
            });
        }
        if rendered.contains(['\n', '\r']) {
            return Err(Error::LineBreakFormat {
                value: value.to_owned(),
            });
        }
        Ok(Self(value.to_owned()))
    }

    /// Renders `date` through the format.
    ///
    /// # Panics
    ///
    /// Panics when rendering fails, which construction rules out: a format
    /// that renders one calendar date renders them all.
    #[must_use]
    pub fn render(&self, date: Date) -> String {
        strtime::format(&self.0, date).expect("formats are validated at construction")
    }

    /// The format as it was given.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for Format {
    /// The [`DEFAULT_FORMAT`] format.
    fn default() -> Self {
        Self(DEFAULT_FORMAT.to_owned())
    }
}

/// A strftime format proven, at construction, to render a datetime to a
/// non-empty single line. Carrying the proof in the type means rendering
/// cannot fail later, when a note is stamped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StampFormat(String);

impl StampFormat {
    /// Validates `value` by rendering a probe datetime through it.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` cannot render a datetime (an unknown
    /// directive, or one needing more than a civil datetime, like `%Z`),
    /// or when it renders nothing or a character a property value cannot
    /// hold — a stamp must stay a value the frontmatter subset can
    /// write, which allows a tab and nothing else of the kind.
    pub fn new(value: &str) -> Result<Self, Error> {
        let probe = jiff::civil::datetime(2001, 2, 3, 4, 5, 6, 0);
        let rendered = strtime::format(value, probe).map_err(|cause| Error::InvalidStamp {
            value: value.to_owned(),
            cause,
        })?;
        if rendered.is_empty() {
            return Err(Error::EmptyStamp {
                value: value.to_owned(),
            });
        }
        if rendered
            .chars()
            .any(crate::frontmatter::forbidden_character)
        {
            return Err(Error::UnprintableStamp {
                value: value.to_owned(),
            });
        }
        Ok(Self(value.to_owned()))
    }

    /// Renders `datetime` through the format.
    ///
    /// # Panics
    ///
    /// Panics when rendering fails, which construction rules out: a
    /// format that renders one datetime renders them all.
    #[must_use]
    pub fn render(&self, datetime: DateTime) -> String {
        strtime::format(&self.0, datetime).expect("formats are validated at construction")
    }

    /// The format as it was given.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for StampFormat {
    /// The [`DEFAULT_STAMP_FORMAT`] format.
    fn default() -> Self {
        Self(DEFAULT_STAMP_FORMAT.to_owned())
    }
}

/// Interprets `value` as a calendar day: `today`, `yesterday`, `tomorrow`,
/// or a `YYYY-MM-DD` date, whose digits need not be zero-padded. The
/// caller supplies what day today is, so parsing stays deterministic.
///
/// # Errors
///
/// Returns an error when `value` is none of the accepted forms.
pub fn parse(value: &str, today: Date) -> Result<Date, Error> {
    match value {
        "today" => Ok(today),
        "yesterday" => Ok(today.saturating_sub(1.day())),
        "tomorrow" => Ok(today.saturating_add(1.day())),
        other => Date::strptime("%Y-%m-%d", other).map_err(|cause| Error::Invalid {
            value: other.to_owned(),
            cause,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::{date, datetime};

    #[test]
    fn format_renders_a_validated_format() {
        let format = Format::new("%Y/%m/%d").expect("format validates");
        assert_eq!(format.render(date(2026, 8, 4)), "2026/08/04");
        assert_eq!(format.as_str(), "%Y/%m/%d");
    }

    #[test]
    fn format_default_is_iso() {
        assert_eq!(Format::default().render(date(2026, 8, 4)), "2026-08-04");
        assert_eq!(
            Format::default(),
            Format::new(DEFAULT_FORMAT).expect("default validates")
        );
    }

    #[test]
    fn format_rejects_unknown_directives() {
        let error = Format::new("%Q").expect_err("unknown directive fails");
        assert!(error.to_string().contains("date format \"%Q\" is invalid"));
    }

    /// The probe is a date without a clock, so time-of-day directives are
    /// rejected too.
    #[test]
    fn format_rejects_time_directives() {
        Format::new("%Y-%H").expect_err("time directive fails");
    }

    #[test]
    fn format_rejects_an_empty_rendering() {
        let error = Format::new("").expect_err("empty format fails");
        assert!(error.to_string().contains("renders an empty file name"));
    }

    /// `%n` renders a real newline, which a file name may legally hold
    /// on Unix — and which would then break the one-path-per-line
    /// output of the listing commands, so it never gets that far.
    #[test]
    fn format_rejects_a_line_break_rendering() {
        let error = Format::new("%Y%n").expect_err("line break fails");
        assert!(
            error
                .to_string()
                .contains("renders a line break into a file name")
        );
    }

    #[test]
    fn parse_accepts_today() {
        assert_eq!(
            parse("today", date(2026, 8, 4)).expect("parses"),
            date(2026, 8, 4)
        );
    }

    #[test]
    fn parse_accepts_yesterday() {
        assert_eq!(
            parse("yesterday", date(2026, 8, 4)).expect("parses"),
            date(2026, 8, 3)
        );
        assert_eq!(
            parse("yesterday", date(2026, 3, 1)).expect("parses"),
            date(2026, 2, 28)
        );
    }

    #[test]
    fn parse_saturates_yesterday_at_the_first_date() {
        assert_eq!(parse("yesterday", Date::MIN).expect("parses"), Date::MIN);
    }

    #[test]
    fn parse_accepts_tomorrow() {
        assert_eq!(
            parse("tomorrow", date(2026, 8, 4)).expect("parses"),
            date(2026, 8, 5)
        );
        assert_eq!(
            parse("tomorrow", date(2026, 2, 28)).expect("parses"),
            date(2026, 3, 1)
        );
    }

    #[test]
    fn parse_saturates_tomorrow_at_the_last_date() {
        assert_eq!(parse("tomorrow", Date::MAX).expect("parses"), Date::MAX);
    }

    #[test]
    fn parse_accepts_iso_dates() {
        assert_eq!(
            parse("2026-01-05", date(2026, 8, 4)).expect("parses"),
            date(2026, 1, 5)
        );
    }

    /// jiff's strptime does not require zero padding; pin that laxness so a
    /// change in jiff surfaces here rather than in the field.
    #[test]
    fn parse_accepts_unpadded_iso_dates() {
        assert_eq!(
            parse("2026-1-5", date(2026, 8, 4)).expect("parses"),
            date(2026, 1, 5)
        );
    }

    #[test]
    fn parse_rejects_unknown_words() {
        let error = parse("someday", date(2026, 8, 4)).expect_err("word fails");
        assert!(error.to_string().contains("invalid date \"someday\""));
        assert!(
            error
                .to_string()
                .contains("expected today, yesterday, tomorrow")
        );
    }

    #[test]
    fn parse_rejects_impossible_dates() {
        parse("2026-02-30", date(2026, 8, 4)).expect_err("bad calendar date fails");
    }

    #[test]
    fn parse_rejects_trailing_input() {
        parse("2026-08-03T10:00", date(2026, 8, 4)).expect_err("datetime fails");
    }

    #[test]
    fn stamp_format_renders_a_validated_format() {
        let format = StampFormat::new("%Y-%m-%dT%H:%M:%S").expect("format validates");
        assert_eq!(
            format.render(datetime(2026, 8, 5, 10, 11, 12, 0)),
            "2026-08-05T10:11:12"
        );
        assert_eq!(format.as_str(), "%Y-%m-%dT%H:%M:%S");
    }

    #[test]
    fn stamp_format_default_is_the_datetime_shape() {
        assert_eq!(
            StampFormat::default(),
            StampFormat::new(DEFAULT_STAMP_FORMAT).expect("default validates")
        );
        assert_eq!(
            StampFormat::default().render(datetime(2026, 8, 5, 1, 2, 3, 0)),
            "2026-08-05T01:02:03"
        );
    }

    /// A date-only stamp format is allowed: it degrades to a date-shaped
    /// value, not an error.
    #[test]
    fn stamp_format_accepts_date_only_formats() {
        let format = StampFormat::new("%Y-%m-%d").expect("format validates");
        assert_eq!(
            format.render(datetime(2026, 8, 5, 10, 11, 12, 0)),
            "2026-08-05"
        );
    }

    #[test]
    fn stamp_format_rejects_unknown_directives() {
        let error = StampFormat::new("%Q").expect_err("unknown directive fails");
        assert!(
            error
                .to_string()
                .contains("timestamp format \"%Q\" is invalid")
        );
    }

    /// The probe is a civil datetime without a zone, so zone directives
    /// are rejected.
    #[test]
    fn stamp_format_rejects_zone_directives() {
        StampFormat::new("%Y %Z").expect_err("zone directive fails");
    }

    #[test]
    fn stamp_format_rejects_an_empty_rendering() {
        let error = StampFormat::new("").expect_err("empty format fails");
        assert!(error.to_string().contains("renders nothing"));
    }

    /// A stamp must stay a writable property value, so the rendering may
    /// hold a tab and no other control character.
    #[test]
    fn stamp_format_rejects_control_renderings() {
        let error = StampFormat::new("%Y\n%m").expect_err("line break fails");
        assert!(
            error
                .to_string()
                .contains("renders an unprintable character")
        );
        StampFormat::new("%Y\r%m").expect_err("carriage return fails");
        StampFormat::new("a\u{7}b").expect_err("control character fails");
        StampFormat::new("a\u{fffe}b").expect_err("noncharacter fails");
        StampFormat::new("a\u{2028}b").expect_err("line separator fails");
        StampFormat::new("%H\t%M").expect("a tab is allowed");
    }
}
