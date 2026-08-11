//! Rendering a daily note template: the `{{title}}`, `{{date}}`, and
//! `{{time}}` variables, with Moment-style formats after a colon, over
//! a note's date, the clock, and the note's title.

use jiff::civil::{Date, Time};
use jiff::fmt::strtime;

/// Failure while rendering a template.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the template format \"{}\" holds an unsupported token \"{token}\"", format.escape_debug())]
    UnsupportedToken { format: String, token: String },
    #[error("the template format \"{}\" has an unclosed '['", format.escape_debug())]
    UnclosedBracket { format: String },
}

/// Renders `template` for a note: `{{title}}` becomes `title`, `{{date}}`
/// the note's date as `YYYY-MM-DD`, `{{time}}` the given time as `HH:mm`,
/// and `{{date:FORMAT}}` or `{{time:FORMAT}}` a Moment-style format —
/// both variables accept every token, they differ only in their default.
/// Any other `{{…}}`, including an unclosed one, is not a variable and
/// passes through as written, so a template can hold other tools' syntax.
///
/// # Errors
///
/// Returns an error when a format holds an alphabetic token outside the
/// supported set or an unclosed `[` literal — never a silently wrong
/// rendering.
pub fn rendered(template: &str, date: Date, time: Time, title: &str) -> Result<String, Error> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        out.push_str(&rest[..start]);
        expanded(&after[..end], date, time, title, &mut out)?;
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Renders one `{{…}}` body into `out`: the three template variables,
/// matched exactly, with `YYYY-MM-DD` and `HH:mm` as their defaults;
/// anything else is reconstructed as written.
fn expanded(
    inner: &str,
    date: Date,
    time: Time,
    title: &str,
    out: &mut String,
) -> Result<(), Error> {
    match inner.split_once(':') {
        None if inner == "title" => out.push_str(title),
        None if inner == "date" => moment("YYYY-MM-DD", date, time, out)?,
        None if inner == "time" => moment("HH:mm", date, time, out)?,
        Some(("date" | "time", format)) => moment(format, date, time, out)?,
        _ => {
            out.push_str("{{");
            out.push_str(inner);
            out.push_str("}}");
        }
    }
    Ok(())
}

/// Renders a Moment-style format into `out`: `[bracketed]` text is
/// literal, a run of one repeated ASCII letter is a token, and any other
/// character copies through.
fn moment(format: &str, date: Date, time: Time, out: &mut String) -> Result<(), Error> {
    let mut rest = format;
    while let Some(character) = rest.chars().next() {
        if character == '[' {
            let Some(end) = rest.find(']') else {
                return Err(Error::UnclosedBracket {
                    format: format.to_owned(),
                });
            };
            out.push_str(&rest[1..end]);
            rest = &rest[end + 1..];
        } else if character.is_ascii_alphabetic() {
            let length = rest.len() - rest.trim_start_matches(character).len();
            let Some(rendered) = token(character, length, date, time) else {
                return Err(Error::UnsupportedToken {
                    format: format.to_owned(),
                    token: rest[..length].to_owned(),
                });
            };
            out.push_str(&rendered);
            rest = &rest[length..];
        } else {
            out.push(character);
            rest = &rest[character.len_utf8()..];
        }
    }
    Ok(())
}

/// One Moment token's rendering, or `None` outside the supported set.
/// The meridiem arms keep their branches on one line deliberately: the
/// binary renders with the real clock, so a branch the time of day picks
/// must not span lines a coverage gate could find half-taken.
fn token(letter: char, length: usize, date: Date, time: Time) -> Option<String> {
    match (letter, length) {
        ('Y', 4) => Some(format!("{:04}", date.year())),
        ('Y', 2) => Some(format!("{:02}", date.year().rem_euclid(100))),
        ('M', 4) => Some(named("%B", date)),
        ('M', 3) => Some(named("%b", date)),
        ('M', 2) => Some(format!("{:02}", date.month())),
        ('M', 1) => Some(date.month().to_string()),
        ('D', 2) => Some(format!("{:02}", date.day())),
        ('D', 1) => Some(date.day().to_string()),
        ('d', 4) => Some(named("%A", date)),
        ('d', 3) => Some(named("%a", date)),
        ('H', 2) => Some(format!("{:02}", time.hour())),
        ('H', 1) => Some(time.hour().to_string()),
        ('h', 2) => Some(format!("{:02}", twelve_hour(time))),
        ('h', 1) => Some(twelve_hour(time).to_string()),
        ('m', 2) => Some(format!("{:02}", time.minute())),
        ('m', 1) => Some(time.minute().to_string()),
        ('s', 2) => Some(format!("{:02}", time.second())),
        ('s', 1) => Some(time.second().to_string()),
        ('A', 1) => Some(if time.hour() < 12 { "AM" } else { "PM" }.to_owned()),
        ('a', 1) => Some(if time.hour() < 12 { "am" } else { "pm" }.to_owned()),
        _ => None,
    }
}

/// A name-producing strftime directive rendered for `date`; the
/// directives this module passes are statically valid, so the expect
/// cannot fire.
fn named(directive: &str, date: Date) -> String {
    strtime::format(directive, date).expect("the directive is statically valid")
}

/// The clock hour on a twelve-hour dial, where 0 and 12 both read 12.
/// One line, for the reason the meridiem tokens give.
fn twelve_hour(time: Time) -> i8 {
    let hour = time.hour() % 12;
    if hour == 0 { 12 } else { hour }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(template: &str, time: Time) -> String {
        rendered(template, jiff::civil::date(2026, 1, 5), time, "2026-01-05")
            .expect("template renders")
    }

    fn afternoon() -> Time {
        jiff::civil::time(14, 3, 9, 0)
    }

    #[test]
    fn rendered_leaves_plain_text_alone() {
        assert_eq!(
            render("no variables here\n", afternoon()),
            "no variables here\n"
        );
        assert_eq!(render("", afternoon()), "");
    }

    #[test]
    fn rendered_renders_the_default_variables() {
        assert_eq!(
            render("# {{title}}\n{{date}} {{time}}\n", afternoon()),
            "# 2026-01-05\n2026-01-05 14:03\n"
        );
    }

    #[test]
    fn rendered_renders_every_date_token() {
        assert_eq!(
            render("{{date:YYYY YY MMMM MMM MM M DD D dddd ddd}}", afternoon()),
            "2026 26 January Jan 01 1 05 5 Monday Mon"
        );
    }

    #[test]
    fn rendered_renders_every_time_token() {
        let cases = [
            (afternoon(), "14 14 02 2 03 3 09 9 PM pm"),
            (jiff::civil::time(9, 5, 7, 0), "09 9 09 9 05 5 07 7 AM am"),
            (
                jiff::civil::time(0, 30, 0, 0),
                "00 0 12 12 30 30 00 0 AM am",
            ),
            (
                jiff::civil::time(12, 0, 0, 0),
                "12 12 12 12 00 0 00 0 PM pm",
            ),
        ];
        for (time, expected) in cases {
            assert_eq!(
                render("{{time:HH H hh h mm m ss s A a}}", time),
                expected,
                "at {time}"
            );
        }
    }

    /// One renderer serves both variables: the split is only about the
    /// default format, so date tokens work in `{{time:…}}` and clock
    /// tokens in `{{date:…}}`.
    #[test]
    fn rendered_mixes_dates_and_times_in_either_variable() {
        assert_eq!(render("{{time:YYYY}} {{date:HH}}", afternoon()), "2026 14");
    }

    #[test]
    fn rendered_keeps_bracket_literals() {
        assert_eq!(
            render("{{date:[on] dddd [at MM]}}", afternoon()),
            "on Monday at MM"
        );
        assert_eq!(render("{{date:[]YYYY}}", afternoon()), "2026");
    }

    #[test]
    fn rendered_copies_format_punctuation_and_non_ascii() {
        assert_eq!(render("{{date:D. M. YYYY г}}", afternoon()), "5. 1. 2026 г");
    }

    #[test]
    fn rendered_renders_an_empty_format_as_nothing() {
        assert_eq!(render("x{{date:}}y", afternoon()), "xy");
    }

    #[test]
    fn rendered_passes_unknown_variables_through() {
        let template = "{{tags}} {{ date }} {{DATE}} {{title:x}}";
        assert_eq!(render(template, afternoon()), template);
    }

    #[test]
    fn rendered_keeps_unclosed_braces() {
        assert_eq!(render("still {{date", afternoon()), "still {{date");
        assert_eq!(render("{{", afternoon()), "{{");
    }

    /// A stray opener swallows up to the next closer, so the variable
    /// inside never renders; the text passes through unchanged, which is
    /// the refusal-free reading of a malformed template.
    #[test]
    fn rendered_passes_a_stray_opener_through() {
        assert_eq!(render("x {{ y {{date}}", afternoon()), "x {{ y {{date}}");
    }

    #[test]
    fn rendered_reports_an_unsupported_token() {
        let error = rendered(
            "{{date:Q}}",
            jiff::civil::date(2026, 1, 5),
            afternoon(),
            "t",
        )
        .expect_err("unsupported token fails");
        assert_eq!(
            error.to_string(),
            "the template format \"Q\" holds an unsupported token \"Q\""
        );
        let error = rendered(
            "{{time:hh:mm:YYYYY}}",
            jiff::civil::date(2026, 1, 5),
            afternoon(),
            "t",
        )
        .expect_err("overlong run fails");
        assert_eq!(
            error.to_string(),
            "the template format \"hh:mm:YYYYY\" holds an unsupported token \"YYYYY\""
        );
    }

    #[test]
    fn rendered_reports_an_unclosed_bracket() {
        let error = rendered(
            "{{date:[oops}}",
            jiff::civil::date(2026, 1, 5),
            afternoon(),
            "t",
        )
        .expect_err("unclosed bracket fails");
        assert_eq!(
            error.to_string(),
            "the template format \"[oops\" has an unclosed '['"
        );
    }

    #[test]
    fn rendered_uses_the_given_title() {
        assert_eq!(
            rendered(
                "{{title}}!",
                jiff::civil::date(2026, 1, 5),
                afternoon(),
                "My Note"
            )
            .expect("template renders"),
            "My Note!"
        );
    }
}
