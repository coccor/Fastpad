//! What the note index knows about a Markdown note (second brain phase 1 spec §6.1): its tags,
//! front matter aliases and properties. Read from the text with `pulldown-cmark`, so nothing in
//! code spans, code blocks or HTML counts. Pure: no disk, no window.

use super::frontmatter;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// A note's tags are capped here; more are dropped (spec §6.4).
pub const MAX_TAGS: usize = 500;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TagSource {
    FrontMatter,
    Inline,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TagUse {
    /// As written, without the `#`.
    pub name: String,
    /// 0-based line of the use (the front matter's line for its tags).
    pub line: u32,
    pub source: TagSource,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NoteFacts {
    pub aliases: Vec<String>,
    pub tags: Vec<TagUse>,
    /// Front matter keys other than tags and aliases, each value as one line of text.
    pub properties: Vec<(String, String)>,
}

/// The parser options the index reads notes with: the preview's, plus front matter.
pub fn options() -> Options {
    crate::preview::model::PARSE_OPTIONS.union(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS)
}

/// Reads `text`'s facts.
pub fn note_facts(text: &str) -> NoteFacts {
    let mut facts = NoteFacts::default();
    if let Some(front) = frontmatter::parse(text) {
        facts.aliases = front.aliases();
        facts.tags = front
            .tags()
            .into_iter()
            .filter(|tag| is_tag(tag))
            .map(|name| TagUse {
                name,
                line: 0,
                source: TagSource::FrontMatter,
            })
            .collect();
        facts.properties = front
            .entries
            .iter()
            .filter(|(key, _)| {
                !["tags", "tag", "aliases", "alias"]
                    .iter()
                    .any(|known| key.eq_ignore_ascii_case(known))
            })
            .map(|(key, value)| (key.clone(), value.display()))
            .collect();
    }
    let line_starts = line_starts(text);
    let line_of = |offset: usize| line_starts.partition_point(|&start| start <= offset) as u32 - 1;
    // Inside these, text is not prose: no tags.
    let mut skip = 0usize;
    // Inline HTML elements open in the current block (`<span>#x</span>`).
    let mut inline_html = 0usize;
    for (event, range) in Parser::new_ext(text, options()).into_offset_iter() {
        match event {
            Event::InlineHtml(html) => {
                if html.starts_with("</") {
                    inline_html = inline_html.saturating_sub(1);
                } else if opens_element(&html) {
                    inline_html += 1;
                }
            }
            Event::End(
                TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item | TagEnd::TableCell,
            ) => inline_html = 0,
            Event::Start(
                Tag::MetadataBlock(_)
                | Tag::CodeBlock(_)
                | Tag::Link { .. }
                | Tag::Image { .. }
                | Tag::HtmlBlock,
            ) => skip += 1,
            Event::End(
                TagEnd::MetadataBlock(_)
                | TagEnd::CodeBlock
                | TagEnd::Link
                | TagEnd::Image
                | TagEnd::HtmlBlock,
            ) => skip = skip.saturating_sub(1),
            Event::Text(run) if skip == 0 && inline_html == 0 => {
                for (at, name) in inline_tags(&run) {
                    if facts.tags.len() >= MAX_TAGS {
                        break;
                    }
                    facts.tags.push(TagUse {
                        name: name.to_owned(),
                        line: line_of(range.start + at),
                        source: TagSource::Inline,
                    });
                }
            }
            _ => {}
        }
    }
    facts.tags.truncate(MAX_TAGS);
    facts
}

/// Whether an inline HTML tag opens an element whose content follows: not a closing tag, a
/// comment, a self-closing tag or a void element such as `<br>`.
fn opens_element(html: &str) -> bool {
    const VOID: [&str; 7] = ["br", "img", "hr", "input", "wbr", "area", "source"];
    let Some(inner) = html.strip_prefix('<') else {
        return false;
    };
    if inner.starts_with(['!', '?', '/']) || html.trim_end().ends_with("/>") {
        return false;
    }
    let name = inner
        .split(|c: char| !c.is_ascii_alphanumeric())
        .next()
        .unwrap_or_default();
    !name.is_empty() && !VOID.iter().any(|void| name.eq_ignore_ascii_case(void))
}

fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(at, _)| at + 1))
        .collect()
}

fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '/')
}

/// Whether `name` (without `#`) is a tag: tag characters only, at least one letter.
pub fn is_tag(name: &str) -> bool {
    !name.is_empty() && name.chars().all(is_tag_char) && name.chars().any(char::is_alphabetic)
}

/// The tags in a run of prose, with each `#`'s byte offset in it (spec §6.3): `#` at the start of
/// the run or after whitespace or `(`, then tag characters with at least one letter. A trailing
/// `/` isn't part of the tag.
pub fn inline_tags(run: &str) -> Vec<(usize, &str)> {
    let mut tags = Vec::new();
    let mut previous: Option<char> = None;
    for (at, c) in run.char_indices() {
        let starts = c == '#' && previous.is_none_or(|p| p.is_whitespace() || p == '(');
        previous = Some(c);
        if !starts {
            continue;
        }
        let rest = &run[at + 1..];
        let length = rest
            .char_indices()
            .find(|(_, c)| !is_tag_char(*c))
            .map_or(rest.len(), |(end, _)| end);
        let name = rest[..length].trim_end_matches('/');
        if is_tag(name) {
            tags.push((at, name));
        }
    }
    tags
}

/// The tag a nested tag belongs to, and so on up: `project/fastpad` gives `project`.
pub fn parent_tags(name: &str) -> impl Iterator<Item = &str> {
    name.match_indices('/').map(move |(at, _)| &name[..at])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(text: &str) -> Vec<String> {
        note_facts(text)
            .tags
            .into_iter()
            .map(|tag| tag.name)
            .collect()
    }

    #[test]
    fn inline_tags_follow_the_spec_examples() {
        assert_eq!(
            names("Plan #project/fastpad and #2026-q4, (#todo) #todo/."),
            ["project/fastpad", "2026-q4", "todo", "todo"]
        );
        // Not tags: digits only, a `#` inside a word, and headings.
        assert!(names("Issue #123, C# and a#b").is_empty());
        assert!(names("# Heading\n\n## Another").is_empty());
        assert_eq!(names("Café #réunion"), ["réunion"]);
    }

    #[test]
    fn tags_in_code_links_and_html_are_not_tags() {
        let text = "`#inline` code\n\n```\n#fenced\n```\n\n    #indented\n\n\
                    [#linked](x.md) and <span>#html</span>\n\n<div>\n#block\n</div>\n\nreal #one";
        assert_eq!(names(text), ["one"]);
        // A void element doesn't hide what follows it: after a line break, a tag may start.
        assert_eq!(names("line<br>#two"), ["two"]);
    }

    #[test]
    fn tags_get_their_line_numbers() {
        let facts = note_facts("first\n\nthird #a\n- item #b");
        let lines = facts
            .tags
            .iter()
            .map(|tag| (tag.name.as_str(), tag.line))
            .collect::<Vec<_>>();
        assert_eq!(lines, [("a", 2), ("b", 3)]);
    }

    #[test]
    fn front_matter_gives_tags_aliases_and_properties_and_is_not_read_as_prose() {
        let facts = note_facts(
            "---\ntitle: Atlas #notatag\ntags: [project/atlas, '#q4']\naliases: [Atlas]\n\
             status: active\n---\nBody #inline\n",
        );
        assert_eq!(facts.aliases, ["Atlas"]);
        let tags = facts
            .tags
            .iter()
            .map(|tag| (tag.name.as_str(), tag.source))
            .collect::<Vec<_>>();
        assert_eq!(
            tags,
            [
                ("project/atlas", TagSource::FrontMatter),
                ("q4", TagSource::FrontMatter),
                ("inline", TagSource::Inline)
            ]
        );
        assert_eq!(
            facts.properties,
            // ` #` starts a YAML comment, so it is no part of the title (and no tag).
            [
                ("title".to_owned(), "Atlas".to_owned()),
                ("status".to_owned(), "active".to_owned())
            ]
        );
    }

    #[test]
    fn nested_tags_name_their_parents() {
        assert_eq!(parent_tags("a/b/c").collect::<Vec<_>>(), ["a", "a/b"]);
        assert_eq!(parent_tags("a").count(), 0);
    }
}
