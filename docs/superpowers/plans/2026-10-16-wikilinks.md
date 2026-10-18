# Wikilinks, Backlinks and Link Updates (PR 1c) Implementation Plan

> **For agentic workers:** Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Notes link to each other with `[[wikilinks]]` (and Markdown links to files). Links can be followed from the preview and the editor, and completed as you type. The Links view shows what links to the active note, what it links to, which link targets don't exist, and which notes nothing links to. Renames and moves, by FastPad or outside it, offer to update the links that would break.

**Spec:** `docs/superpowers/specs/2026-10-16-second-brain-phase-1-design.md` §7, and the link-fixing part of §10.5.

## Architecture

- **Pure** (`library::facts`, `library::links`, `library::frontmatter`, done first):
  - link extraction with exact target byte ranges;
  - `Resolver` (§7.2);
  - `backlinks`, `outgoing`, `missing`, `unlinked`;
  - `edits_for_moves` (inbound links, and a moved note's own relative links);
  - `frontmatter::add_alias`;
  - `renamed_since_cache` plus `own_renames`, so FastPad's own renames are never taken for outside ones.
- **Window:** `library_host::links` builds a `Resolver` from the live note list and the index on demand, follows a link, creates a missing note, and applies link edits (open tabs as one undo step each, closed files with a stamp check).
- **Preview:** wikilinks render as links with destination `fastpad-wiki:<target>#<heading>`; `classify_link` maps them to `LinkAction::Wiki`.
- **Editor:** an indicator under the `[[…]]` links on screen in Markdown tabs; Ctrl+click and F12 follow them; typing `[[` opens Scintilla's autocompletion with note names.
- **Links view:** a sixth sidebar view (`SidebarView::Links`).

## Deliberate deviations from the spec

- **Unresolved links in the preview look like other links** (§7.3 asks for a muted, dashed style). The preview model doesn't know the notebook; following one still creates the note.
- **Completion uses Scintilla's own list**, filtered by prefix and ignoring case, not re-ranked by the quick-open matcher per keystroke (§7.4).
- **"Keep the old name as an alias" is a prompt button**, "Update links and keep alias", not a checkbox: FastPad's prompts have none (§7.6).
- **Link snippets in the Links view** read the linking line from the file when the view is built, only for the active note's backlinks.
- **Missing notes count wikilinks only.** A Markdown link to a missing file is usually a typo, not a note waiting to be written.
- **Closed files are rewritten on the UI thread.** A rename touches a handful of notes; each write is atomic, stamp-checked and recorded as FastPad's own.

## Parts

1. [x] Pure link logic and tests.
2. [ ] `library_host::links`: the resolver, following, creating, and applying edits.
3. [ ] Preview: wikilink spans, `LinkAction::Wiki`, click to follow or create.
4. [ ] Editor: indicators on visible lines, Ctrl+click, F12 (`NotesFollowLink`), `[[` completion.
5. [ ] Links view: active note's Linked from, Links to; whole-notebook Missing notes and Not linked from anywhere; palette commands.
6. [ ] Link updates on rename and move: FastPad's (prompt with alias button; own links without prompt), outside (notice plus `NotesUpdateRenamedLinks`).
7. [ ] README, spec §17, full tests, commit.
