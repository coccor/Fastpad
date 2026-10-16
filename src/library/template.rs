//! Note templates (second brain phase 1 spec §9.3): placeholders filled in when a note is
//! made from a template.

use super::dates::{self, Date, Time};

/// What the placeholders stand for.
pub struct Context<'a> {
    pub date: Date,
    pub time: Time,
    /// The notebook's daily note format, for `{{date}}`.
    pub daily_format: &'a str,
    /// The new note's file stem, for `{{title}}`.
    pub title: &'a str,
}

/// A filled-in template.
#[derive(Debug, Eq, PartialEq)]
pub struct Filled {
    pub text: String,
    /// Where `{{cursor}}` was, as a byte offset into `text`; `None` puts the caret at the end.
    pub cursor: Option<usize>,
}

/// Replaces `{{date}}`, `{{date:FORMAT}}`, `{{time}}`, `{{title}}` and `{{cursor}}` (removed,
/// remembered as the caret) everywhere in `template`. Unknown placeholders stay as they are.
pub fn fill(template: &str, context: &Context<'_>) -> Filled {
    let mut text = String::with_capacity(template.len());
    let mut cursor = None;
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        text.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            text.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let name = &after[..close];
        let value = match name.trim() {
            "date" => Some(dates::format(context.date, context.daily_format)),
            "time" => Some(format!(
                "{:02}:{:02}",
                context.time.hour, context.time.minute
            )),
            "title" => Some(context.title.to_owned()),
            "cursor" => {
                if cursor.is_none() {
                    cursor = Some(text.len());
                }
                Some(String::new())
            }
            other => other
                .strip_prefix("date:")
                .map(|format| dates::format(context.date, format.trim())),
        };
        match value {
            Some(value) => text.push_str(&value),
            None => text.push_str(&rest[open..open + 2 + close + 2]),
        }
        rest = &after[close + 2..];
    }
    text.push_str(rest);
    Filled { text, cursor }
}

/// A daily note made without a template: its date as a heading, then a blank line.
pub fn default_daily(date: Date) -> Filled {
    Filled {
        text: format!(
            "# {}\n\n",
            dates::format(date, dates::DEFAULT_DAILY_HEADING)
        ),
        cursor: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(title: &str) -> Context<'_> {
        Context {
            date: Date::new(2026, 10, 16).unwrap(),
            time: Time { hour: 9, minute: 5 },
            daily_format: "YYYY-MM-DD",
            title,
        }
    }

    #[test]
    fn placeholders_fill_in_everywhere_including_front_matter() {
        let template = "---\ncreated: {{date:YYYY-MM-DD}}\n---\n# {{title}}\n\n{{date}} at {{time}}: {{cursor}}\n";
        let filled = fill(template, &context("Atlas"));
        assert_eq!(
            filled.text,
            "---\ncreated: 2026-10-16\n---\n# Atlas\n\n2026-10-16 at 09:05: \n"
        );
        assert_eq!(filled.cursor, Some(filled.text.len() - 1));
    }

    #[test]
    fn unknown_and_unclosed_placeholders_stay_as_they_are() {
        let filled = fill("{{weather}} and {{date:ddd}} {{oops", &context("x"));
        assert_eq!(filled.text, "{{weather}} and Fri {{oops");
        assert_eq!(filled.cursor, None);
    }

    #[test]
    fn only_the_first_cursor_places_the_caret() {
        let filled = fill("a{{cursor}}b{{cursor}}c", &context("x"));
        assert_eq!(filled.text, "abc");
        assert_eq!(filled.cursor, Some(1));
    }

    #[test]
    fn a_daily_note_without_a_template_gets_its_date_as_a_heading() {
        let filled = default_daily(Date::new(2026, 10, 16).unwrap());
        assert_eq!(filled.text, "# Friday, 16 October 2026\n\n");
    }
}
