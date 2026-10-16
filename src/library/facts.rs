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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkKind {
    /// `[[target]]`, `[[target#heading|label]]`.
    Wiki,
    /// `[label](relative/path.md)`: a link to a file, not a web page.
    Markdown,
    /// `![alt](attachments/image.png)`: an image or other file shown inline.
    Image,
}

/// One link in a note (spec §7.1).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkUse {
    /// What it points at, as written: a wikilink's target without `#heading`, or a Markdown
    /// link's destination without `#fragment`, percent-encoded as written.
    pub target: String,
    pub heading: Option<String>,
    /// 0-based line of the link.
    pub line: u32,
    pub kind: LinkKind,
    /// Byte range of `target` in the note, for rewriting it after a rename.
    pub start: u32,
    pub end: u32,
}

/// A note's links are capped here; more are dropped (spec §6.4).
pub const MAX_LINKS: usize = 2000;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NoteFacts {
    pub aliases: Vec<String>,
    pub tags: Vec<TagUse>,
    pub links: Vec<LinkUse>,
    /// Front matter keys other than tags and aliases, each value as one line of text.
    pub properties: Vec<(String, String)>,
}

/// The parser options the index reads notes with: the preview's, plus front matter and
/// wikilinks.
pub fn options() -> Options {
    crate::preview::model::PARSE_OPTIONS
        .union(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS)
        .union(Options::ENABLE_WIKILINKS)
}

/// Whether a Markdown link destination may be a file in the notebook: no scheme (`https:`,
/// `mailto:`), not just a `#fragment`, not a network path.
fn is_local_destination(dest: &str) -> bool {
    let dest = dest.trim();
    if dest.is_empty()
        || dest.starts_with('#')
        || dest.starts_with("//")
        || dest.starts_with("\\\\")
    {
        return false;
    }
    let scheme = dest.find(':').is_some_and(|colon| {
        colon > 1
            && dest[..colon]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    });
    !scheme
}

/// The parts of the `[[…]]` at the start of `source` (a wikilink's own text): its target's byte
/// range within `source`, and its `#heading`.
fn wikilink_parts(source: &str) -> Option<(std::ops::Range<usize>, Option<String>)> {
    let inner_start = source.find("[[")? + 2;
    let inner_end = inner_start + source[inner_start..].find("]]")?;
    let inner = &source[inner_start..inner_end];
    let before_label = inner.find('|').map_or(inner, |bar| &inner[..bar]);
    let (target, heading) = match before_label.find('#') {
        Some(hash) => (
            &before_label[..hash],
            Some(before_label[hash + 1..].trim().to_owned()).filter(|heading| !heading.is_empty()),
        ),
        None => (before_label, None),
    };
    let lead = target.len() - target.trim_start().len();
    let start = inner_start + lead;
    Some((start..start + target.trim().len(), heading))
}

/// The destination's byte range within `source`, a Markdown link's or image's own text
/// (`[label](dest "title")`), and its `#fragment`.
fn markdown_destination(source: &str) -> Option<(std::ops::Range<usize>, Option<String>)> {
    let open = source.rfind("](")? + 2;
    let rest = &source[open..];
    let (offset, dest) = if let Some(angled) = rest.strip_prefix('<') {
        (1, &angled[..angled.find('>')?])
    } else {
        let end = rest
            .find(|c: char| c.is_whitespace() || c == ')')
            .unwrap_or(rest.len());
        (0, &rest[..end])
    };
    let (path, fragment) = match dest.find('#') {
        Some(hash) => (&dest[..hash], Some(dest[hash + 1..].to_owned())),
        None => (dest, None),
    };
    let start = open + offset;
    Some((
        start..start + path.len(),
        fragment.filter(|f| !f.is_empty()),
    ))
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
        // Links are read from their own source text. Code and HTML never produce link events.
        if let Event::Start(
            Tag::Link {
                link_type,
                dest_url,
                ..
            }
            | Tag::Image {
                link_type,
                dest_url,
                ..
            },
        ) = &event
            && facts.links.len() < MAX_LINKS
        {
            let source = &text[range.clone()];
            let image = matches!(event, Event::Start(Tag::Image { .. }));
            let parts = match link_type {
                pulldown_cmark::LinkType::WikiLink { .. } => {
                    wikilink_parts(source).map(|parts| (LinkKind::Wiki, parts))
                }
                pulldown_cmark::LinkType::Inline if is_local_destination(dest_url) => {
                    markdown_destination(source).map(|parts| {
                        (
                            if image {
                                LinkKind::Image
                            } else {
                                LinkKind::Markdown
                            },
                            parts,
                        )
                    })
                }
                _ => None,
            };
            if let Some((kind, (target, heading))) = parts
                && !target.is_empty()
            {
                facts.links.push(LinkUse {
                    target: source[target.clone()].to_owned(),
                    heading,
                    line: line_of(range.start),
                    kind,
                    start: (range.start + target.start) as u32,
                    end: (range.start + target.end) as u32,
                });
            }
        }
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

#[cfg(test)]
mod link_tests {
    use super::*;

    fn links(text: &str) -> Vec<(String, Option<String>, LinkKind, String)> {
        note_facts(text)
            .links
            .into_iter()
            .map(|link| {
                let span = text[link.start as usize..link.end as usize].to_owned();
                (link.target, link.heading, link.kind, span)
            })
            .collect()
    }

    #[test]
    fn every_wikilink_form_is_read_with_its_target_range() {
        let found = links(
            "See [[Plan]], [[Project Atlas|Atlas]], [[Plan#Goals]], [[ sub/Note #Top | x ]] \
             and [[#Local]].",
        );
        assert_eq!(
            found,
            [
                ("Plan".into(), None, LinkKind::Wiki, "Plan".into()),
                (
                    "Project Atlas".into(),
                    None,
                    LinkKind::Wiki,
                    "Project Atlas".into()
                ),
                (
                    "Plan".into(),
                    Some("Goals".into()),
                    LinkKind::Wiki,
                    "Plan".into()
                ),
                (
                    "sub/Note".into(),
                    Some("Top".into()),
                    LinkKind::Wiki,
                    "sub/Note".into()
                ),
            ],
            "a same-note [[#heading]] has no target"
        );
    }

    #[test]
    fn markdown_links_to_files_and_images_count_but_web_links_do_not() {
        let found = links(
            "[plan](plan.md) [web](https://x.dev) [mail](mailto:a@b) [frag](#top) \
             [sp](<my notes/a b.md>) [enc](my%20note.md#part) ![shot](att/s.png \"t\")",
        );
        assert_eq!(
            found,
            [
                ("plan.md".into(), None, LinkKind::Markdown, "plan.md".into()),
                (
                    "my notes/a b.md".into(),
                    None,
                    LinkKind::Markdown,
                    "my notes/a b.md".into()
                ),
                (
                    "my%20note.md".into(),
                    Some("part".into()),
                    LinkKind::Markdown,
                    "my%20note.md".into()
                ),
                (
                    "att/s.png".into(),
                    None,
                    LinkKind::Image,
                    "att/s.png".into()
                ),
            ]
        );
    }

    #[test]
    fn links_in_code_are_not_links_and_lines_are_counted() {
        let facts = note_facts("`[[a]]`\n\n```\n[[b]]\n```\n\nreal [[c]]\n");
        assert_eq!(facts.links.len(), 1);
        assert_eq!(
            (facts.links[0].target.as_str(), facts.links[0].line),
            ("c", 6)
        );
    }
}
