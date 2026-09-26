//! Copying files into the notebook tree (open editors spec §4): where each dropped item lands,
//! which ones are refused, which clash with a name already there, and the words the prompts and
//! notices use. Pure but for the one `exists` check per item the caller passes in.

use crate::library::{at_or_under, model::same_path, normalize_folder};
use std::path::{Path, PathBuf};

/// Why an item is not copied (spec §4.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Refusal {
    /// The destination is the item itself.
    SamePlace,
    /// A folder into itself or a folder inside it.
    IntoItself,
    /// The destination holds the item: replacing it would recycle the item too.
    HoldsSource,
    /// No file name at all: a drive or UNC share root, which nothing can be copied under.
    WholeDrive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    Copy,
    /// Something already has the destination's name: ask before replacing it.
    Clash,
    Refused(Refusal),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Planned {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub outcome: Outcome,
}

/// The refusal for `source` landing at `destination` inside the folder `target`, if any.
fn refusal(source: &Path, target: &Path, destination: &Path) -> Option<Refusal> {
    if same_path(source, destination) {
        Some(Refusal::SamePlace)
    } else if at_or_under(target, source) {
        Some(Refusal::IntoItself)
    } else if at_or_under(source, destination) {
        Some(Refusal::HoldsSource)
    } else {
        None
    }
}

/// Where each of `sources` lands in `folder` (relative to `root`; empty is the root), refused,
/// clashing or free. `exists` is asked once per item that isn't refused. `root`, `folder` and
/// every source are normalized first (`library::normalize_folder`: absolute, one separator
/// style, no trailing separator or `.`/`..` component), so a trailing `\`, a `/`, or a `.`/`..`
/// spelling of the same location compares equal to the canonical one, the way the refusal checks
/// below need it to.
pub(crate) fn plan(
    sources: &[PathBuf],
    root: &Path,
    folder: &Path,
    exists: &dyn Fn(&Path) -> bool,
) -> Vec<Planned> {
    let target = normalize_folder(&root.join(folder));
    sources
        .iter()
        .map(|source| {
            let source = normalize_folder(source);
            // A drive or UNC share root has no file name: it can only ever be refused, before
            // `exists` is asked and without ever treating `target` as its destination.
            let Some(name) = source.file_name() else {
                let destination = source.clone();
                return Planned {
                    source,
                    destination,
                    outcome: Outcome::Refused(Refusal::WholeDrive),
                };
            };
            let destination = target.join(name);
            let outcome = match refusal(&source, &target, &destination) {
                Some(refusal) => Outcome::Refused(refusal),
                None if exists(&destination) => Outcome::Clash,
                None => Outcome::Copy,
            };
            Planned {
                source,
                destination,
                outcome,
            }
        })
        .collect()
}

/// Whether dropping `sources` into `folder` copies anything: the drag's target test, in memory.
/// Normalized the same way `plan` is, so the same spellings agree with it.
pub(crate) fn any_accepted(sources: &[PathBuf], root: &Path, folder: &Path) -> bool {
    let target = normalize_folder(&root.join(folder));
    sources.iter().any(|source| {
        let source = normalize_folder(source);
        source
            .file_name()
            .is_some_and(|name| refusal(&source, &target, &target.join(name)).is_none())
    })
}

pub(crate) fn item_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

pub(crate) fn replace_question(name: &str, folder: &str) -> String {
    format!("{name} already exists in {folder}. Replace it?")
}

/// The notice for copied files the tree doesn't list; `names` holds at least one.
pub(crate) fn hidden_notice(names: &[String]) -> String {
    match names {
        [one] => format!("{one} was copied but isn't shown: the notebook lists text notes only."),
        _ => {
            let mut listed = names.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
            if names.len() > 3 {
                listed.push_str(", …");
            }
            format!(
                "{} files were copied but aren't shown: {listed}",
                names.len()
            )
        }
    }
}

pub(crate) fn dirty_notice(name: &str) -> String {
    format!("Copied the saved version of {name}. Your unsaved changes are still in its tab.")
}

pub(crate) fn failed_notice(name: &str, reason: &str, copied_before: Option<usize>) -> String {
    let reason = reason.trim_end_matches(['.', ' ', '\r', '\n']);
    match copied_before {
        Some(1) => {
            format!("{name} could not be copied: {reason}. 1 file was copied before the failure.")
        }
        Some(files) => format!(
            "{name} could not be copied: {reason}. {files} files were copied before the failure."
        ),
        None => format!("{name} could not be copied: {reason}."),
    }
}

pub(crate) fn refused_notice(name: &str, refusal: Refusal) -> String {
    let why = match refusal {
        Refusal::SamePlace => "it is already there",
        Refusal::IntoItself => "a folder can't be copied into itself",
        Refusal::HoldsSource => "it would replace the folder it is in",
        Refusal::WholeDrive => "a whole drive or share can't be copied into the notebook",
    };
    format!("{name} was not copied: {why}.")
}

pub(crate) fn recycle_failed_notice(name: &str) -> String {
    format!("{name} was not copied: it could not be moved to the Recycle Bin.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    const ROOT: &str = r"C:\notes";

    fn plan_of(sources: &[&str], folder: &str, existing: &[&str]) -> Vec<(String, Outcome)> {
        let sources: Vec<PathBuf> = sources.iter().map(PathBuf::from).collect();
        let existing: Vec<PathBuf> = existing.iter().map(PathBuf::from).collect();
        let exists = |path: &Path| existing.iter().any(|known| same_path(known, path));
        plan(&sources, Path::new(ROOT), Path::new(folder), &exists)
            .into_iter()
            .map(|planned| (planned.destination.display().to_string(), planned.outcome))
            .collect()
    }

    #[test]
    fn a_file_lands_under_its_own_name_in_the_target_folder() {
        assert_eq!(
            plan_of(&[r"D:\x\draft.txt"], "work", &[]),
            [(r"C:\notes\work\draft.txt".into(), Outcome::Copy)]
        );
        assert_eq!(
            plan_of(&[r"D:\x\draft.txt"], "", &[]),
            [(r"C:\notes\draft.txt".into(), Outcome::Copy)],
            "empty is the root"
        );
    }

    #[test]
    fn a_copy_onto_itself_is_refused_before_any_prompt() {
        // Break caught (Review Focus 1): "Replace?" offered for a note onto its own folder, so
        // OK would recycle the source and lose it.
        assert_eq!(
            plan_of(&[r"C:\notes\work\a.md"], "work", &[r"C:\notes\work\a.md"]),
            [(
                r"C:\notes\work\a.md".into(),
                Outcome::Refused(Refusal::SamePlace)
            )]
        );
        assert_eq!(
            plan_of(&[r"C:\NOTES\Work\A.md"], "work", &[r"C:\notes\work\a.md"])[0].1,
            Outcome::Refused(Refusal::SamePlace),
            "letter case doesn't matter"
        );
    }

    #[test]
    fn a_folder_into_itself_or_below_is_refused() {
        assert_eq!(
            plan_of(&[r"C:\notes\work"], r"work\inner", &[])[0].1,
            Outcome::Refused(Refusal::IntoItself)
        );
    }

    #[test]
    fn a_whole_drive_or_share_is_refused_before_any_prompt() {
        // Break caught (Critical): a source with no file name (a drive or a share root) had no
        // file name to compute a destination from, so it fell back to the drop folder itself and
        // came out as a same-named "Clash" there; OK in the copy host would then replace it,
        // recycling the drop folder (or the notebook root) along with everything in it.
        assert_eq!(
            plan_of(&[r"D:\"], "work", &[])[0].1,
            Outcome::Refused(Refusal::WholeDrive)
        );
        assert_eq!(
            plan_of(&[r"\\server\share\"], "", &[])[0].1,
            Outcome::Refused(Refusal::WholeDrive)
        );
        assert_eq!(
            plan_of(&[r"C:\"], "work", &[])[0].1,
            Outcome::Refused(Refusal::WholeDrive),
            "even on the same drive as the notebook"
        );
        assert!(!any_accepted(
            &[PathBuf::from(r"D:\")],
            Path::new(ROOT),
            Path::new("work")
        ));
    }

    #[test]
    fn same_place_and_holds_source_refusals_ignore_the_sources_spelling() {
        // Break caught (Important): `same_path` compares the raw, lowercased string, so a
        // trailing separator, a forward slash, or a `.`/`..` component made an identical location
        // fail to match and land as a same-named "Clash" instead (Review Focus 1): OK would then
        // recycle the source. `plan` and `any_accepted` normalize every path first so none of
        // these spellings can matter.
        for source in [
            r"C:\notes\work\a.md",
            "C:/notes/work/a.md",
            r"C:\notes\.\work\a.md",
            r"C:\notes\x\..\work\a.md",
        ] {
            assert_eq!(
                plan_of(&[source], "work", &[r"C:\notes\work\a.md"])[0].1,
                Outcome::Refused(Refusal::SamePlace),
                "{source}"
            );
            assert!(
                !any_accepted(&[PathBuf::from(source)], Path::new(ROOT), Path::new("work")),
                "{source}"
            );
        }
        for source in [
            r"C:\notes\work\",
            "C:/notes/work",
            r"C:\notes\.\work",
            r"C:\notes\x\..\work",
        ] {
            assert_eq!(
                plan_of(&[source], "", &[r"C:\notes\work"])[0].1,
                Outcome::Refused(Refusal::SamePlace),
                "{source}"
            );
            assert!(
                !any_accepted(&[PathBuf::from(source)], Path::new(ROOT), Path::new("")),
                "{source}"
            );
        }
        // Review Focus 2, the same way: notes\work\work holding notes\work.
        for source in [
            r"C:\notes\work\work\",
            "C:/notes/work/work",
            r"C:\notes\.\work\work",
            r"C:\notes\x\..\work\work",
        ] {
            assert_eq!(
                plan_of(&[source], "", &[r"C:\notes\work"])[0].1,
                Outcome::Refused(Refusal::HoldsSource),
                "{source}"
            );
            assert!(
                !any_accepted(&[PathBuf::from(source)], Path::new(ROOT), Path::new("")),
                "{source}"
            );
        }
    }

    #[test]
    fn into_itself_refusal_ignores_the_sources_spelling() {
        // Break caught (Important): `C:\notes\work\` (a trailing separator) dropped on `work`
        // planned a Copy into `work\work`, and `copy_tree` would nest into it forever.
        for source in [
            r"C:\notes\work\",
            "C:/notes/work",
            r"C:\notes\.\work",
            r"C:\notes\x\..\work",
        ] {
            assert_eq!(
                plan_of(&[source], "work", &[])[0].1,
                Outcome::Refused(Refusal::IntoItself),
                "{source}"
            );
            assert!(
                !any_accepted(&[PathBuf::from(source)], Path::new(ROOT), Path::new("work")),
                "{source}"
            );
        }
    }

    #[test]
    fn a_destination_that_holds_the_source_is_refused() {
        // Break caught (Review Focus 2): notes\work\work dropped on the root would replace
        // notes\work, recycling the source with it.
        assert_eq!(
            plan_of(&[r"C:\notes\work\work"], "", &[r"C:\notes\work"])[0].1,
            Outcome::Refused(Refusal::HoldsSource)
        );
    }

    #[test]
    fn a_taken_name_is_a_clash_and_the_rest_carry_on() {
        assert_eq!(
            plan_of(&[r"D:\a.md", r"D:\b.md"], "", &[r"C:\notes\a.md"]),
            [
                (r"C:\notes\a.md".into(), Outcome::Clash),
                (r"C:\notes\b.md".into(), Outcome::Copy)
            ]
        );
        assert!(any_accepted(
            &[PathBuf::from(r"C:\notes\a.md"), PathBuf::from(r"D:\b.md")],
            Path::new(ROOT),
            Path::new("")
        ));
        assert!(!any_accepted(
            &[PathBuf::from(r"C:\notes\a.md")],
            Path::new(ROOT),
            Path::new("")
        ));
    }

    #[test]
    fn the_notices_read_as_the_spec_words_them() {
        assert_eq!(
            replace_question("a.md", "work"),
            "a.md already exists in work. Replace it?"
        );
        assert_eq!(
            hidden_notice(&["photo.png".into()]),
            "photo.png was copied but isn't shown: the notebook lists text notes only."
        );
        assert_eq!(
            hidden_notice(&[
                "a.png".into(),
                "b.png".into(),
                "c.png".into(),
                "d.png".into()
            ]),
            "4 files were copied but aren't shown: a.png, b.png, c.png, …"
        );
        assert_eq!(
            dirty_notice("draft.txt"),
            "Copied the saved version of draft.txt. Your unsaved changes are still in its tab."
        );
        assert_eq!(
            failed_notice("a.md", "Access is denied", None),
            "a.md could not be copied: Access is denied."
        );
        assert_eq!(
            failed_notice("pics", "The disk is full", Some(3)),
            "pics could not be copied: The disk is full. 3 files were copied before the failure."
        );
        assert_eq!(
            failed_notice("pics", "The disk is full", Some(1)),
            "pics could not be copied: The disk is full. 1 file was copied before the failure."
        );
        assert_eq!(
            refused_notice("a.md", Refusal::SamePlace),
            "a.md was not copied: it is already there."
        );
        assert_eq!(
            refused_notice("work", Refusal::IntoItself),
            "work was not copied: a folder can't be copied into itself."
        );
        assert_eq!(
            refused_notice("work", Refusal::HoldsSource),
            "work was not copied: it would replace the folder it is in."
        );
        assert_eq!(
            refused_notice(r"D:\", Refusal::WholeDrive),
            r"D:\ was not copied: a whole drive or share can't be copied into the notebook."
        );
        assert_eq!(
            recycle_failed_notice("a.md"),
            "a.md was not copied: it could not be moved to the Recycle Bin."
        );
    }
}
