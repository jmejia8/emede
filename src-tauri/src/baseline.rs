//! The committed version of a document, for change highlighting to diff against.
//!
//! This shells out to the `git` binary rather than linking libgit2. Three
//! plumbing calls do not justify a C dependency in the release binary, shelling
//! out degrades cleanly when git is absent, and anyone editing markdown inside a
//! repository has git installed. emede already spawns processes elsewhere.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};

/// Set once `git` turns out not to be installed, so the miss costs one failed
/// spawn per process rather than one per reload.
static GIT_MISSING: AtomicBool = AtomicBool::new(false);

/// What we learned about one document's place in a repository.
///
/// The blob is keyed on the HEAD commit id, which is the only thing that can
/// change it: `git add` does not alter `HEAD:path`, and committing does. So a
/// reload of an unchanged HEAD reuses the cached bytes after a single cheap
/// `rev-parse`, while a commit made in a terminal correctly invalidates.
pub(crate) struct GitCache {
    /// `None` once we know this path is not inside a work tree.
    repo: Option<Repo>,
    head_oid: String,
    /// `None` means "resolved against this HEAD, and the path is not in it" —
    /// an untracked file, or a repo with no commits. A cached negative, not a
    /// missing lookup.
    blob: Option<String>,
}

struct Repo {
    root: PathBuf,
    /// Repo-root-relative, forward slashes: what `git show HEAD:<path>` wants.
    /// An absolute path there resolves to nothing.
    rel: String,
}

fn run_git(cwd: &Path, args: &[&OsStr]) -> Option<Output> {
    if GIT_MISSING.load(Ordering::Relaxed) {
        return None;
    }

    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(cwd)
        .args(args)
        // Read-only queries have no business taking the index lock.
        .env("GIT_OPTIONAL_LOCKS", "0");

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW — otherwise every reload flashes a console.
        command.creation_flags(0x0800_0000);
    }

    match command.output() {
        Ok(output) => Some(output),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            GIT_MISSING.store(true, Ordering::Relaxed);
            None
        }
        Err(_) => None,
    }
}

/// Run a git command whose output is a single line, e.g. an object id or path.
fn git_line(cwd: &Path, args: &[&OsStr]) -> Option<String> {
    let output = run_git(cwd, args)?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let line = text.trim_end_matches(['\n', '\r']).to_string();
    if line.is_empty() {
        None
    } else {
        Some(line)
    }
}

fn find_repo(document: &Path) -> Option<Repo> {
    let parent = document.parent()?;
    let root = PathBuf::from(git_line(
        parent,
        &[OsStr::new("rev-parse"), OsStr::new("--show-toplevel")],
    )?);

    // Both sides are canonicalized before stripping so a symlinked path or a
    // `/tmp` → `/private/tmp` style alias still matches the toplevel git
    // reports.
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.clone());
    let canonical_doc = document.canonicalize().ok()?;
    let rel = canonical_doc.strip_prefix(&canonical_root).ok()?;

    // A path git can address has to survive being spelled as UTF-8. If it
    // does not, fall back to the snapshot baseline rather than guessing.
    let rel = rel
        .components()
        .map(|c| c.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()?
        .join("/");

    Some(Repo {
        root: canonical_root,
        rel,
    })
}

fn head_oid(repo: &Repo) -> Option<String> {
    git_line(
        &repo.root,
        &[OsStr::new("rev-parse"), OsStr::new("HEAD")],
    )
}

fn show_head_blob(repo: &Repo) -> Option<String> {
    let spec = format!("HEAD:{}", repo.rel);
    // `--` guards against a filename that begins with `-`; the revision spec
    // itself is unambiguous because it is prefixed with `HEAD:`.
    let output = run_git(
        &repo.root,
        &[OsStr::new("show"), OsStr::new(&spec), OsStr::new("--")],
    )?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// The committed contents of `document`, or `None` when there is no committed
/// version to compare against — outside a repo, untracked, or a fresh `git
/// init` with no commits yet, all of which fall through to the snapshot
/// baseline.
///
/// `cache` is this document's entry, updated in place.
pub(crate) fn head_blob(document: &Path, cache: &mut Option<GitCache>) -> Option<String> {
    // Which repo a file belongs to does not change while it is open, so that
    // lookup is cached even across a negative result.
    if cache.is_none() {
        let repo = find_repo(document);
        *cache = Some(GitCache {
            repo,
            head_oid: String::new(),
            blob: None,
        });
    }

    let entry = cache.as_mut()?;
    let repo = entry.repo.as_ref()?;

    // A HEAD-less repo (fresh `git init`) has no committed version at all.
    let oid = head_oid(repo)?;
    if oid != entry.head_oid {
        entry.blob = show_head_blob(repo);
        entry.head_oid = oid;
    }

    entry.blob.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Build a throwaway repo under the OS temp dir; `None` if git is missing.
    fn scratch_repo(name: &str) -> Option<PathBuf> {
        let root = std::env::temp_dir().join(format!("emede-baseline-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).ok()?;

        let git = |args: &[&str]| -> bool {
            Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };

        if !git(&["init", "-q"]) {
            return None;
        }
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "emede test"]);
        Some(root)
    }

    #[test]
    fn tracked_file_resolves_to_its_committed_contents() {
        let Some(root) = scratch_repo("tracked") else {
            return;
        };
        let file = root.join("note.md");
        std::fs::write(&file, "committed\n").unwrap();

        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap()
        };
        git(&["add", "note.md"]);
        git(&["commit", "-qm", "first"]);

        std::fs::write(&file, "edited\n").unwrap();

        let mut cache = None;
        assert_eq!(head_blob(&file, &mut cache).as_deref(), Some("committed\n"));
        // Second call goes through the oid cache and must agree.
        assert_eq!(head_blob(&file, &mut cache).as_deref(), Some("committed\n"));
    }

    #[test]
    fn untracked_file_in_a_repo_has_no_committed_version() {
        let Some(root) = scratch_repo("untracked") else {
            return;
        };
        let seed = root.join("seed.md");
        std::fs::write(&seed, "seed\n").unwrap();
        for args in [
            vec!["add", "seed.md"],
            vec!["commit", "-qm", "seed"],
        ] {
            Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(&args)
                .output()
                .unwrap();
        }

        let file = root.join("fresh.md");
        std::fs::write(&file, "fresh\n").unwrap();
        let mut cache = None;
        assert_eq!(head_blob(&file, &mut cache), None);
    }

    #[test]
    fn a_repo_with_no_commits_has_no_committed_version() {
        let Some(root) = scratch_repo("headless") else {
            return;
        };
        let file = root.join("note.md");
        std::fs::write(&file, "hello\n").unwrap();
        let mut cache = None;
        assert_eq!(head_blob(&file, &mut cache), None);
    }

    #[test]
    fn a_path_outside_any_repo_has_no_committed_version() {
        let dir = std::env::temp_dir().join("emede-baseline-norepo");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("note.md");
        std::fs::write(&file, "hello\n").unwrap();

        let mut cache = None;
        // `git rev-parse --show-toplevel` may still succeed if the temp dir
        // happens to sit inside a repository; only assert the interesting case.
        if find_repo(&file).is_none() {
            assert_eq!(head_blob(&file, &mut cache), None);
        }
    }
}
