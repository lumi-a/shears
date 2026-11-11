//! Finds the export's root directory inside a set of possibly nested dirs.
//!
//! Sometimes, an uploaded `.zip` / `.tar.zst` (or a picked folder) might wrap the export in a
//! directory itself, maybe even several levels deep (for some reason, idk). But
//! [`super::ingest`] expects paths relative to the export root itself
//! (`registries/item.json`).
//!
//! So, descend while a directory holds exactly one subdirectory and nothing else, then check
//! that wherever that stops looks like an export. In practice, we match for largest common
//! initial string.

use std::collections::BTreeSet;

use super::RawFile;

/// Apparently macOS adds some strange directory to zips that we'd like to ignore.
const MACOS_METADATA: &str = "__MACOSX/";

/// Files that [`super::ingest`] reads and that every export must have.
const REQUIRED_FILES: [&str; 2] = ["registries/item.json", "registries/fluid.json"];

/// Where an export's recipes live. Only there at all if some script registered a
/// `ServerEvents.recipes` handler when exporting. This is also kinda required for shears to
/// function, so we should check for its presence.
const RECIPES: &str = "recipes/";

/// Why the files don't look like a `/kubejs export`.
#[derive(Debug, thiserror::Error)]
pub enum RootError {
    /// Descending stopped at a directory holding one subdirectory plus some loose files, and
    /// that directory isn't an export itself.
    ///
    /// Perhaps the subdirectory is the export?
    #[error(
        "{} has files next to its only directory, {subdirectory:?}, so we aren't sure if \
        this really is a kubejs directory. Please move the following files either to \
        the root directory (the one that contains the `recipes` directory), or remove \
        them entirely: {loose_files:?}",
        shown(.supposed_root_directory)
    )]
    StrayFiles {
        /// Where descending stopped.
        supposed_root_directory: String,
        /// The one subdirectory there.
        subdirectory: String,
        /// The loose files in this directory.
        loose_files: Vec<String>,
    },
    /// A file that we'd expect the export to have isn't in the directory where we stopped
    /// descending.
    #[error("{} doesn't look like a /kubejs export: We're missing {missing}", shown(.supposed_root_directory))]
    MissingFile {
        /// Where descending stopped.
        supposed_root_directory: String,
        /// The missing file, relative to `supposed_root_directory`.
        missing: &'static str,
    },
    /// The export has no recipes directory.
    #[error(
        "the export at {} has no recipes; KubeJS only exports them if a script registers a \
         ServerEvents.recipes handler, e.g. `ServerEvents.recipes(event => {{}})`",
        shown(.supposed_root_directory)
    )]
    NoRecipes {
        /// The export's root.
        supposed_root_directory: String,
    },
}

/// `root` for an error message, where the empty root directory needs a name of its own.
fn shown(root_directory: &str) -> String {
    if root_directory.is_empty() {
        "the top level".to_owned()
    } else {
        format!("{root_directory:?}")
    }
}

/// The longest directory prefix (empty, or ending in `/`) that every path shares.
///
/// Top-level `__MACOSX/` gets ignored.
fn common_directory(files: &[RawFile]) -> String {
    let mut paths = files
        .iter()
        .map(|file| file.path.as_str())
        .filter(|path| !path.starts_with(MACOS_METADATA));
    let Some(first) = paths.next() else {
        return String::new();
    };
    let mut prefix = first
        .rfind('/')
        .map_or("", |slash| first.split_at(slash + 1).0)
        .to_owned();
    for path in paths {
        if prefix.is_empty() {
            break;
        }
        if path.starts_with(prefix.as_str()) {
            continue;
        }
        let shared = prefix
            .bytes()
            .zip(path.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        // Cut back to the last `/` both share. `/` is ASCII, so that's a char boundary.
        let keep = prefix
            .bytes()
            .take(shared)
            .rposition(|byte| byte == b'/')
            .map_or(0, |slash| slash + 1);
        prefix.truncate(keep);
    }
    prefix
}

/// What the root directory holds.
///
/// This is used for the [`RootError::StrayFiles`] check.
struct RootDirectoryContents<'a> {
    /// Names of the direct subdirectories.
    directories: BTreeSet<&'a str>,
    /// Names of the files directly inside it.
    files: Vec<&'a str>,
}

impl<'a> RootDirectoryContents<'a> {
    /// What the top level of `files` holds.
    fn top(files: &'a [RawFile]) -> Self {
        let mut level = RootDirectoryContents {
            directories: BTreeSet::new(),
            files: Vec::new(),
        };
        for file in files {
            match file.path.split_once('/') {
                Some((directory, _)) => {
                    let _: bool = level.directories.insert(directory);
                }
                None => level.files.push(&file.path),
            }
        }
        level
    }
}

/// Finds the export's root directory, keeps only the files under it, and strips that
/// directory's prefix from their paths.
///
/// Returns the stripped prefix (empty if `files` were already relative to the root, otherwise
/// ending in `/`) alongside the kept files. A top-level `__MACOSX/` is dropped.
///
/// # Errors
///
/// If descending stops somewhere without `registries/item.json`, `registries/fluid.json`, and
/// at least one file under `recipes/`. If it stopped because of loose files next to a single
/// subdirectory, the error names those files.
#[inline]
pub fn strip(files: Vec<RawFile>) -> Result<(String, Vec<RawFile>), RootError> {
    let supposed_root_directory = common_directory(&files);

    let mut found = [false; REQUIRED_FILES.len()];
    let mut has_recipes = false;
    let kept: Vec<RawFile> = files
        .into_iter()
        .filter(|file| !file.path.starts_with(MACOS_METADATA))
        .filter_map(|mut file| {
            if !file.path.starts_with(supposed_root_directory.as_str()) {
                return None;
            }
            // In place: no new allocation per path.
            file.path.replace_range(..supposed_root_directory.len(), "");
            for (required, was_found) in REQUIRED_FILES.iter().zip(&mut found) {
                *was_found |= file.path == *required;
            }
            has_recipes |= file.path.starts_with(RECIPES);
            Some(file)
        })
        .collect();

    if let Some(missing) = REQUIRED_FILES
        .iter()
        .zip(found)
        .find_map(|(&required, was_found)| (!was_found).then_some(required))
    {
        let top = RootDirectoryContents::top(&kept);
        if let (Some(only), 1) = (top.directories.first(), top.directories.len()) {
            return Err(RootError::StrayFiles {
                subdirectory: (*only).to_owned(),
                loose_files: top.files.iter().map(|&file| file.to_owned()).collect(),
                supposed_root_directory,
            });
        }
        return Err(RootError::MissingFile {
            supposed_root_directory,
            missing,
        });
    }
    if !has_recipes {
        return Err(RootError::NoRecipes {
            supposed_root_directory,
        });
    }
    Ok((supposed_root_directory, kept))
}

#[cfg(test)]
mod tests {
    use super::{RootError, strip};
    use crate::ingest::RawFile;

    /// The least that passes the export check, under `prefix`.
    fn export_under(prefix: &str) -> Vec<String> {
        [
            "registries/item.json",
            "registries/fluid.json",
            "recipes/minecraft/dirt.json",
        ]
        .iter()
        .map(|path| format!("{prefix}{path}"))
        .collect()
    }

    fn files<S: AsRef<str>>(paths: &[S]) -> Vec<RawFile> {
        paths
            .iter()
            .map(|path| RawFile {
                path: path.as_ref().to_owned(),
                bytes: Vec::new(),
            })
            .collect()
    }

    fn paths(files: &[RawFile]) -> Vec<&str> {
        files.iter().map(|file| file.path.as_str()).collect()
    }

    #[test]
    fn leaves_already_rooted_paths_alone() {
        let (root, kept) = strip(files(&export_under(""))).expect("an export");

        assert_eq!(root, "");
        assert_eq!(paths(&kept), export_under(""));
    }

    #[test]
    fn descends_through_nested_wrappers_ignoring_macos_metadata() {
        let mut all = export_under("outer/moni/");
        all.push("__MACOSX/outer/moni/registries/._item.json".to_owned());

        let (root, kept) = strip(files(&all)).expect("an export");

        assert_eq!(root, "outer/moni/");
        assert_eq!(paths(&kept), export_under(""));
    }

    #[test]
    fn drops_macos_metadata_next_to_an_unwrapped_export() {
        let mut all = export_under("");
        all.push("__MACOSX/registries/._item.json".to_owned());

        let (root, kept) = strip(files(&all)).expect("an export");

        assert_eq!(root, "");
        assert_eq!(paths(&kept), export_under(""));
    }

    /// Paths sharing a name prefix that isn't a whole directory (`moni/` vs `monifactory/`)
    /// don't share that directory.
    #[test]
    fn a_shared_name_prefix_is_not_a_shared_directory() {
        let mut all = export_under("moni/");
        all.extend(export_under("monifactory/"));

        let result = strip(files(&all));

        assert!(
            matches!(&result, Err(RootError::MissingFile { supposed_root_directory, missing: "registries/item.json" }) if supposed_root_directory.is_empty()),
            "got {result:?}"
        );
    }

    /// A mod with namespace `registries` and a recipe or tag named `item` puts a second
    /// `registries/item.json` inside the export. It must not get in the way.
    #[test]
    fn a_recipe_named_like_a_required_file_is_just_a_recipe() {
        let mut all = export_under("moni/");
        all.push("moni/recipes/registries/item.json".to_owned());

        let (root, _kept) = strip(files(&all)).expect("an export");

        assert_eq!(root, "moni/");
    }

    #[test]
    fn a_loose_file_next_to_the_wrapper_is_named_in_the_error() {
        let mut all = export_under("outer/moni/");
        all.push("outer/notes.txt".to_owned());

        let result = strip(files(&all));

        assert!(
            matches!(
                &result,
                Err(RootError::StrayFiles { supposed_root_directory, subdirectory, loose_files })
                    if supposed_root_directory == "outer/" && subdirectory == "moni" && loose_files == &["notes.txt"]
            ),
            "got {result:?}"
        );
    }

    #[test]
    fn two_exports_side_by_side_are_not_an_export() {
        let mut all = export_under("a/");
        all.extend(export_under("b/"));

        let result = strip(files(&all));

        assert!(
            matches!(&result, Err(RootError::MissingFile { supposed_root_directory, missing: "registries/item.json" }) if supposed_root_directory.is_empty()),
            "got {result:?}"
        );
    }

    #[test]
    fn a_missing_registry_is_named_in_the_error() {
        let result = strip(files(&[
            "moni/registries/item.json",
            "moni/recipes/minecraft/dirt.json",
        ]));

        assert!(
            matches!(
                &result,
                Err(RootError::MissingFile { supposed_root_directory, missing: "registries/fluid.json" })
                    if supposed_root_directory == "moni/"
            ),
            "got {result:?}"
        );
    }

    #[test]
    fn an_export_without_recipes_says_how_to_get_them() {
        let result = strip(files(&[
            "moni/registries/item.json",
            "moni/registries/fluid.json",
            "moni/index.json",
        ]));

        assert!(
            matches!(&result, Err(RootError::NoRecipes { supposed_root_directory }) if supposed_root_directory == "moni/"),
            "got {result:?}"
        );
        assert!(
            result
                .expect_err("checked above")
                .to_string()
                .contains("ServerEvents.recipes"),
            "the message should say how to get recipes exported"
        );
    }

    #[test]
    fn nothing_at_all_is_not_an_export() {
        let result = strip(Vec::new());

        assert!(
            matches!(&result, Err(RootError::MissingFile { supposed_root_directory, missing: "registries/item.json" }) if supposed_root_directory.is_empty()),
            "got {result:?}"
        );
    }
}
