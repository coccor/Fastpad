//! Front matter: the YAML block between `---` lines at the top of a Markdown note (second brain
//! phase 1 spec §6.2). Only a subset of YAML is read: `key: value`, quoted strings, flow lists
//! (`[a, b]`) and block lists (`- a`). Anything else is kept as raw text for its key and is never
//! an error. Pure: no disk, no window.

/// Front matter must close within this many lines of the top, or the file has none.
pub const MAX_LINES: usize = 200;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Value {
    Text(String),
    List(Vec<String>),
    /// What FastPad doesn't read (a nested map, a multi-line string, …), as written.
    Raw(String),
}

impl Value {
    /// The value as one line of text, for showing it: a list joined with ", ".
    pub fn display(&self) -> String {
        match self {
            Self::Text(text) | Self::Raw(text) => text.clone(),
            Self::List(items) => items.join(", "),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FrontMatter {
    /// Keys in file order, as written.
    pub entries: Vec<(String, Value)>,
    /// The byte offset just after the closing line: where the note's body starts.
    pub body_start: usize,
}

impl FrontMatter {
    /// The first value for `key`, matched without regard to case.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value)
    }

    /// Tags from `tags` or `tag`: a list, or one string of tags separated by commas or spaces.
    /// A leading `#` is optional.
    pub fn tags(&self) -> Vec<String> {
        let items = match self.get("tags").or_else(|| self.get("tag")) {
            Some(Value::List(items)) => items.clone(),
            Some(Value::Text(text)) => text.split([',', ' ', '\t']).map(str::to_owned).collect(),
            _ => Vec::new(),
        };
        items
            .into_iter()
            .map(|item| item.trim().trim_start_matches('#').to_owned())
            .filter(|item| !item.is_empty())
            .collect()
    }

    /// Other names a link can use for this note, from `aliases` or `alias`.
    pub fn aliases(&self) -> Vec<String> {
        let items = match self.get("aliases").or_else(|| self.get("alias")) {
            Some(Value::List(items)) => items.clone(),
            Some(Value::Text(text)) => vec![text.clone()],
            _ => Vec::new(),
        };
        items
            .into_iter()
            .map(|item| item.trim().to_owned())
            .filter(|item| !item.is_empty())
            .collect()
    }
}

/// The front matter at the top of `text`, or `None` when it has none: the first line (after a
/// byte order mark) must be exactly `---`, and a later line within `MAX_LINES` exactly `---` or
/// `...`.
pub fn parse(text: &str) -> Option<FrontMatter> {
    let start = if text.starts_with('\u{feff}') { 3 } else { 0 };
    let mut lines = LineIter::new(text, start);
    let (_, first) = lines.next()?;
    if first != "---" {
        return None;
    }
    let mut body = Vec::new();
    let mut body_start = None;
    for (index, (end, line)) in lines.by_ref().enumerate() {
        if index + 1 >= MAX_LINES {
            break;
        }
        if line == "---" || line == "..." {
            body_start = Some(end);
            break;
        }
        body.push(line);
    }
    let body_start = body_start?;
    Some(FrontMatter {
        entries: entries(&body),
        body_start,
    })
}

/// `text` with `alias` added to its front matter's `aliases` (second brain phase 1 spec §7.6),
/// changing nothing else: the block is created when there is none, a flow list (`[a, b]`) or a
/// block list (`- a`) gets the alias in its own style, and a single `aliases: a` becomes a flow
/// list. `None` when the `aliases` value has a form FastPad doesn't read, or the alias is
/// already there. Line ends follow the file's.
pub fn add_alias(text: &str, alias: &str) -> Option<String> {
    // The first line's ending: the front matter's own, or the note's when it has none.
    let eol = if text
        .split('\n')
        .next()
        .is_some_and(|line| line.ends_with('\r'))
    {
        "\r\n"
    } else {
        "\n"
    };
    let quoted = if alias.contains([',', '[', ']', '#', ':', '"', '\'']) {
        format!("\"{}\"", alias.replace('"', "\\\""))
    } else {
        alias.to_owned()
    };
    let Some(front) = parse(text) else {
        let bom = if text.starts_with('\u{feff}') {
            "\u{feff}"
        } else {
            ""
        };
        let body = text.strip_prefix('\u{feff}').unwrap_or(text);
        return Some(format!(
            "{bom}---{eol}aliases: [{quoted}]{eol}---{eol}{body}"
        ));
    };
    if front
        .aliases()
        .iter()
        .any(|known| known.eq_ignore_ascii_case(alias))
    {
        return None;
    }
    let head = &text[..front.body_start];
    let body = &text[front.body_start..];
    // Find the aliases line in the block, by line.
    let mut lines: Vec<&str> = head.split_inclusive('\n').collect();
    let at = lines.iter().position(|line| {
        key_of(line.trim_end_matches(['\r', '\n'])).is_some_and(|(key, _)| {
            key.eq_ignore_ascii_case("aliases") || key.eq_ignore_ascii_case("alias")
        })
    });
    let Some(at) = at else {
        // No aliases key: a new line just before the closing one.
        let close = lines.len() - 1;
        let line = format!("aliases: [{quoted}]{eol}");
        lines.insert(close, &line);
        return Some(format!("{}{body}", lines.concat()));
    };
    let line = lines[at].trim_end_matches(['\r', '\n']);
    let (key, value) = key_of(line)?;
    let replaced = match front.get(key)? {
        Value::List(_) if value.starts_with('[') => {
            let inner = value.strip_prefix('[')?.strip_suffix(']')?.trim_end();
            let separator = if inner.trim().is_empty() { "" } else { ", " };
            let prefix = &line[..line.len() - value.len()];
            Some(format!("{prefix}[{inner}{separator}{quoted}]{eol}"))
        }
        Value::List(items) => {
            // A block list: a new item after the last one, indented like it.
            let last = (at + 1..lines.len() - 1)
                .take_while(|&index| lines[index].trim_start().starts_with('-'))
                .last()?;
            let indent: String = lines[last]
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            let _ = items;
            let item = format!("{indent}- {quoted}{eol}");
            let mut out = lines.clone();
            out.insert(last + 1, &item);
            return Some(format!("{}{body}", out.concat()));
        }
        // Only a plain value, written exactly as read: no comment, no brackets, no quotes.
        Value::Text(single) if !single.is_empty() && single == value && !value.starts_with('[') => {
            let prefix = &line[..line.len() - value.len()];
            Some(format!("{prefix}[{value}, {quoted}]{eol}"))
        }
        Value::Text(empty) if empty.is_empty() && value.is_empty() => {
            let prefix = &line[..line.len() - value.len()];
            Some(format!(
                "{}[{quoted}]{eol}",
                prefix.trim_end().to_owned() + " "
            ))
        }
        Value::Text(_) | Value::Raw(_) => None,
    }?;
    lines[at] = &replaced;
    Some(format!("{}{body}", lines.concat()))
}

/// Lines of `text` from byte `start`, each without its line end, with the offset just after its
/// line end.
struct LineIter<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> LineIter<'a> {
    fn new(text: &'a str, at: usize) -> Self {
        Self { text, at }
    }
}

impl<'a> Iterator for LineIter<'a> {
    type Item = (usize, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        if self.at >= self.text.len() {
            return None;
        }
        let rest = &self.text[self.at..];
        let (line, advance) = match rest.find('\n') {
            Some(newline) => (&rest[..newline], newline + 1),
            None => (rest, rest.len()),
        };
        self.at += advance;
        Some((self.at, line.strip_suffix('\r').unwrap_or(line)))
    }
}

/// The key of a `key: value` line: no leading space, no space or `#` inside, and the colon
/// followed by a space or the end of the line.
fn key_of(line: &str) -> Option<(&str, &str)> {
    if line.starts_with([' ', '\t', '-', '#']) {
        return None;
    }
    let colon = line.find(':')?;
    let (key, rest) = (&line[..colon], &line[colon + 1..]);
    let key = key.trim_end();
    if key.is_empty() || (key.contains([' ', '\t']) && !key.starts_with(['"', '\''])) {
        return None;
    }
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    Some((unquote(key), rest.trim()))
}

fn entries(lines: &[&str]) -> Vec<(String, Value)> {
    let mut entries: Vec<(String, Value)> = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        index += 1;
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let Some((key, value)) = key_of(line) else {
            // A line FastPad doesn't read belongs to the key above it, as raw text.
            if let Some(last) = entries.last_mut() {
                append_raw(last, line);
            }
            continue;
        };
        let value = if value.is_empty() {
            // A block list or a nested map follows, indented or starting with `-`.
            let mut items = Vec::new();
            let mut raw = Vec::new();
            while let Some(next) = lines.get(index) {
                let trimmed = next.trim_start();
                let continues = next.starts_with([' ', '\t']) || trimmed.starts_with("- ");
                if !continues && trimmed != "-" {
                    break;
                }
                index += 1;
                match trimmed.strip_prefix('-') {
                    Some(item) if raw.is_empty() => items.push(unquote(item.trim()).to_owned()),
                    _ => raw.push(*next),
                }
            }
            if !raw.is_empty() {
                let mut all = items
                    .iter()
                    .map(|item| format!("- {item}"))
                    .collect::<Vec<_>>();
                all.extend(raw.iter().map(|line| line.trim().to_owned()));
                Value::Raw(all.join("\n"))
            } else if !items.is_empty() {
                Value::List(items)
            } else {
                Value::Text(String::new())
            }
        } else if let Some(inner) = value
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
        {
            Value::List(
                split_flow(inner)
                    .into_iter()
                    .map(|item| unquote(item.trim()).to_owned())
                    .filter(|item| !item.is_empty())
                    .collect(),
            )
        } else if value.starts_with(['{', '|', '>', '&', '*']) {
            Value::Raw(value.to_owned())
        } else {
            Value::Text(unquote(strip_comment(value)).to_owned())
        };
        entries.push((key.to_owned(), value));
    }
    entries
}

fn append_raw(entry: &mut (String, Value), line: &str) {
    let text = match &entry.1 {
        Value::Text(text) | Value::Raw(text) => text.clone(),
        Value::List(items) => items.join(", "),
    };
    let joined = if text.is_empty() {
        line.trim().to_owned()
    } else {
        format!("{text}\n{}", line.trim())
    };
    entry.1 = Value::Raw(joined);
}

/// Splits a flow list's inside on commas outside quotes.
fn split_flow(inner: &str) -> Vec<&str> {
    let mut items = Vec::new();
    let mut quote = None;
    let mut start = 0;
    for (at, c) in inner.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(open), _) if c == open => quote = None,
            (None, ',') => {
                items.push(&inner[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    items.push(&inner[start..]);
    items
}

/// A plain value's comment (` #` and after) is not part of it; a quoted value keeps everything.
fn strip_comment(value: &str) -> &str {
    if value.starts_with(['"', '\'']) {
        return value;
    }
    match value.find(" #") {
        Some(at) => value[..at].trim_end(),
        None => value,
    }
}

/// `"text"` or `'text'` without its quotes; anything else as it is.
fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if value.len() >= 2 && value.starts_with(quote) && value.ends_with(quote) {
            return &value[1..value.len() - 1];
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    fn list(items: &[&str]) -> Value {
        Value::List(items.iter().map(|item| (*item).to_owned()).collect())
    }

    #[test]
    fn front_matter_needs_dashes_on_the_first_line_and_a_closing_line() {
        assert!(parse("---\ntitle: x\n---\nbody").is_some());
        assert!(parse("\u{feff}---\r\ntitle: x\r\n...\r\nbody").is_some());
        assert_eq!(
            parse("\n---\ntitle: x\n---\n"),
            None,
            "not on the first line"
        );
        assert_eq!(parse("--- \ntitle: x\n---\n"), None, "not exactly ---");
        assert_eq!(parse("---\ntitle: x\nbody without a close"), None);
        let long = format!("---\n{}---\n", "a: b\n".repeat(MAX_LINES));
        assert_eq!(parse(&long), None, "closing too far down");
    }

    #[test]
    fn the_body_starts_after_the_closing_line() {
        let source = "---\ntitle: x\n---\n# Heading\n";
        let front = parse(source).unwrap();
        assert_eq!(&source[front.body_start..], "# Heading\n");
        let crlf = "---\r\na: b\r\n---\r\nbody";
        assert_eq!(&crlf[parse(crlf).unwrap().body_start..], "body");
    }

    #[test]
    fn scalars_quoted_strings_and_both_list_forms_are_read() {
        let front = parse(
            "---\n\
             title: Plan: phase 1\n\
             quoted: \"a # b\"\n\
             single: 'it''s'\n\
             plain: value # a comment\n\
             tags: [alpha, \"beta, gamma\", 'delta']\n\
             aliases:\n  - Atlas\n  - \"Project A\"\n\
             # a comment line\n\
             empty:\n\
             ---\n",
        )
        .unwrap();
        assert_eq!(front.get("title"), Some(&text("Plan: phase 1")));
        assert_eq!(front.get("QUOTED"), Some(&text("a # b")));
        assert_eq!(front.get("plain"), Some(&text("value")));
        assert_eq!(
            front.get("tags"),
            Some(&list(&["alpha", "beta, gamma", "delta"]))
        );
        assert_eq!(front.get("aliases"), Some(&list(&["Atlas", "Project A"])));
        assert_eq!(front.get("empty"), Some(&text("")));
        assert_eq!(front.entries.len(), 7);
    }

    #[test]
    fn what_fastpad_does_not_read_is_kept_as_raw_text() {
        let front = parse(
            "---\n\
             author:\n  name: Dana\n  role: lead\n\
             summary: |\n\
             ---\n",
        )
        .unwrap();
        assert_eq!(
            front.get("author"),
            Some(&Value::Raw("name: Dana\nrole: lead".to_owned()))
        );
        assert_eq!(front.get("summary"), Some(&Value::Raw("|".to_owned())));
    }

    #[test]
    fn tags_come_from_a_list_or_a_string_with_or_without_hashes() {
        let tags = |source: &str| parse(source).unwrap().tags();
        assert_eq!(tags("---\ntags: [a, '#b']\n---\n"), ["a", "b"]);
        assert_eq!(tags("---\ntags: a, b  #c\n---\n"), ["a", "b"]);
        assert_eq!(
            tags("---\ntag: project/atlas todo\n---\n"),
            ["project/atlas", "todo"]
        );
        assert_eq!(tags("---\nTags:\n  - x\n---\n"), ["x"]);
        assert!(tags("---\ntitle: none\n---\n").is_empty());
    }

    #[test]
    fn aliases_come_from_a_list_or_one_string() {
        let aliases = |source: &str| parse(source).unwrap().aliases();
        assert_eq!(
            aliases("---\naliases: [RAG, Retrieval]\n---\n"),
            ["RAG", "Retrieval"]
        );
        assert_eq!(
            aliases("---\nalias: Project Atlas\n---\n"),
            ["Project Atlas"]
        );
    }
}

#[cfg(test)]
mod alias_tests {
    use super::*;

    #[test]
    fn an_alias_goes_in_the_existing_style_and_nothing_else_changes() {
        assert_eq!(
            add_alias("# Plan\n", "Old").as_deref(),
            Some("---\naliases: [Old]\n---\n# Plan\n"),
            "a block is created"
        );
        assert_eq!(
            add_alias("---\ntitle: x\n---\nBody\r\n", "Old").as_deref(),
            Some("---\ntitle: x\naliases: [Old]\n---\nBody\r\n"),
            "a key is added before the closing line"
        );
        assert_eq!(
            add_alias("---\naliases: [A, B] # kept\n---\n", "Old").as_deref(),
            None,
            "a commented flow list is a form FastPad leaves alone"
        );
        assert_eq!(
            add_alias("---\naliases: [A, B]\ntags: [t]\n---\n", "Old: v2").as_deref(),
            Some("---\naliases: [A, B, \"Old: v2\"]\ntags: [t]\n---\n")
        );
        assert_eq!(
            add_alias("---\r\naliases:\r\n  - A\r\nx: 1\r\n---\r\n", "Old").as_deref(),
            Some("---\r\naliases:\r\n  - A\r\n  - Old\r\nx: 1\r\n---\r\n")
        );
        assert_eq!(
            add_alias("---\nalias: A\n---\n", "Old").as_deref(),
            Some("---\nalias: [A, Old]\n---\n")
        );
        assert_eq!(
            add_alias("---\naliases: [old]\n---\n", "Old"),
            None,
            "already there"
        );
        assert_eq!(
            add_alias("---\naliases: {a: 1}\n---\n", "Old"),
            None,
            "a form FastPad doesn't read"
        );
    }
}
