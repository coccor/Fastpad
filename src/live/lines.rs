//! Per-line slots (laid-out lines, in the view) that follow the document's edits: an edit
//! empties the slot of the line it starts on and adds or drops slots for the lines it added or
//! removed, so every other line keeps its value.

use std::ops::Range;

#[derive(Debug)]
pub struct LineCache<T> {
    slots: Vec<Option<T>>,
}

impl<T> Default for LineCache<T> {
    fn default() -> Self {
        Self { slots: Vec::new() }
    }
}

impl<T> LineCache<T> {
    /// Empties every slot, for a document of `line_count` lines.
    pub fn reset(&mut self, line_count: usize) {
        self.slots.clear();
        self.slots.resize_with(line_count, || None);
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn get(&self, line: usize) -> Option<&T> {
        self.slots.get(line).and_then(Option::as_ref)
    }

    pub fn set(&mut self, line: usize, value: T) {
        if let Some(slot) = self.slots.get_mut(line) {
            *slot = Some(value);
        }
    }

    pub fn invalidate(&mut self, lines: Range<usize>) {
        let end = lines.end.min(self.slots.len());
        for slot in &mut self.slots[lines.start.min(end)..end] {
            *slot = None;
        }
    }

    /// An edit that starts on `first_line` and added (`lines_delta > 0`) or removed lines.
    /// The first line is emptied, removed lines' slots are dropped and added lines start empty.
    /// Returns the lines that now need laying out again.
    pub fn apply(&mut self, first_line: usize, lines_delta: isize) -> Range<usize> {
        if first_line >= self.slots.len() {
            // An edit at the very end of a document whose cache is short: grow to fit.
            let wanted = first_line + 1 + lines_delta.max(0) as usize;
            self.slots.resize_with(wanted, || None);
            return first_line..wanted;
        }
        self.slots[first_line] = None;
        let after = first_line + 1;
        if lines_delta > 0 {
            let added = lines_delta as usize;
            self.slots
                .splice(after..after, std::iter::repeat_with(|| None).take(added));
            first_line..after + added
        } else {
            let removed = (lines_delta.unsigned_abs()).min(self.slots.len() - after);
            self.slots.drain(after..after + removed);
            first_line..after
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LineCache;

    fn filled(lines: &[&str]) -> LineCache<String> {
        let mut cache = LineCache::default();
        cache.reset(lines.len());
        for (n, line) in lines.iter().enumerate() {
            cache.set(n, (*line).to_owned());
        }
        cache
    }

    fn contents(cache: &LineCache<String>) -> Vec<Option<&str>> {
        (0..cache.len())
            .map(|n| cache.get(n).map(String::as_str))
            .collect()
    }

    #[test]
    fn insert_within_a_line_invalidates_only_that_line() {
        let mut cache = filled(&["a", "b", "c"]);
        assert_eq!(cache.apply(1, 0), 1..2);
        assert_eq!(contents(&cache), [Some("a"), None, Some("c")]);
    }

    #[test]
    fn inserting_lines_shifts_later_slots_down() {
        let mut cache = filled(&["a", "b", "c"]);
        assert_eq!(cache.apply(1, 2), 1..4);
        assert_eq!(contents(&cache), [Some("a"), None, None, None, Some("c")]);
    }

    #[test]
    fn removing_lines_drops_their_slots() {
        let mut cache = filled(&["a", "b", "c", "d"]);
        assert_eq!(cache.apply(0, -2), 0..1);
        assert_eq!(contents(&cache), [None, Some("d")]);
    }

    #[test]
    fn invalidate_clamps_to_the_cache() {
        let mut cache = filled(&["a", "b"]);
        cache.invalidate(1..9);
        assert_eq!(contents(&cache), [Some("a"), None]);
        cache.invalidate(5..9);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn an_edit_past_the_end_grows_the_cache() {
        let mut cache: LineCache<String> = LineCache::default();
        assert_eq!(cache.apply(0, 1), 0..2);
        assert_eq!(cache.len(), 2);
    }

    /// Lines as Scintilla counts them: a trailing newline starts an empty last line.
    fn lines_of(text: &str) -> Vec<&str> {
        text.split('\n').collect()
    }

    #[test]
    fn cache_follows_edits_like_a_rebuild() {
        let mut seed = 0x2545_f491_u32;
        let mut next = move |bound: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as usize % bound.max(1)
        };
        let pieces = ["x", "yz", "\n", "a\nb", "\n\n", "long line "];
        let mut text = String::from("one\ntwo\nthree");
        let mut cache = LineCache::default();
        cache.reset(lines_of(&text).len());
        for step in 0..2000 {
            let old_lines = lines_of(&text).len();
            let position = next(text.len() + 1);
            if text.is_empty() || next(3) > 0 {
                text.insert_str(position, pieces[next(pieces.len())]);
            } else {
                let end = (position + next(6)).min(text.len());
                text.replace_range(position.min(end)..end, "");
            }
            let first_line = text[..position.min(text.len())].matches('\n').count();
            let delta = lines_of(&text).len() as isize - old_lines as isize;
            cache.apply(first_line, delta);
            let lines = lines_of(&text);
            assert_eq!(cache.len(), lines.len(), "step {step}");
            for (n, line) in lines.iter().enumerate() {
                if let Some(cached) = cache.get(n) {
                    assert_eq!(cached, line, "step {step}, line {n}");
                } else {
                    cache.set(n, (*line).to_owned());
                }
            }
        }
    }
}
