//! That a version number which forces a rebuild says so where people read it.
//!
//! The two stored version numbers are the only thing standing between an
//! upgrade and an index quietly read under the wrong assumptions. Bumping one
//! is therefore a breaking change for anyone holding an index — and the place
//! they find that out is the release page, which is generated from the commit
//! trail rather than from the changelog.
//!
//! v0.3.0 shipped with the chunker version bumped and the release notes
//! silent about it: the migration path was written in `CHANGELOG.md`, where
//! git-cliff never looked, because Conventional Commits carries a breaking
//! change in a `!` or a `BREAKING CHANGE:` footer and neither was there. The
//! notes had to be corrected after publishing. This test is what makes the
//! next one a red gate instead.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The section of the changelog for one version, if it has one.
fn changelog_section(changelog: &str, version: &str) -> Option<String> {
    let start = changelog.find(&format!("## [{version}]"))?;
    let rest = &changelog[start..];
    let end = rest[1..].find("\n## [").map(|at| at + 1).unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// The commits since the last tag, subjects and bodies.
///
/// Empty when git cannot be asked — a source tarball with no history, which
/// is a normal way to build and not a reason to fail.
fn commits_since_last_tag() -> Vec<String> {
    let root = repo_root();
    let describe = Command::new("git").args(["describe", "--tags", "--abbrev=0"]).current_dir(&root).output();
    let Ok(describe) = describe else { return Vec::new() };
    if !describe.status.success() {
        return Vec::new();
    }
    let tag = String::from_utf8_lossy(&describe.stdout).trim().to_string();
    let log = Command::new("git")
        .args(["log", &format!("{tag}..HEAD"), "--format=%s%n%b%n---"])
        .current_dir(&root)
        .output();
    let Ok(log) = log else { return Vec::new() };
    String::from_utf8_lossy(&log.stdout).split("\n---\n").map(str::to_string).collect()
}

/// Whether a commit message marks itself as a breaking change.
///
/// Conventional Commits allows two spellings, and git-cliff reads both: a `!`
/// before the colon of the subject, or a `BREAKING CHANGE:` footer.
fn marks_a_breaking_change(message: &str) -> bool {
    let subject = message.lines().next().unwrap_or_default();
    let bang = subject.split_once(':').is_some_and(|(kind, _)| kind.ends_with('!') && !kind.contains(' '));
    bang || message.contains("BREAKING CHANGE:") || message.contains("BREAKING-CHANGE:")
}

/// The version in the workspace manifest.
fn workspace_version() -> String {
    let manifest = read("Cargo.toml");
    manifest
        .lines()
        .find_map(|line| line.trim().strip_prefix("version = \""))
        .and_then(|rest| rest.split('"').next())
        .expect("the workspace manifest declares a version")
        .to_string()
}

/// The changelog has to describe the version that is about to ship.
#[test]
fn the_changelog_describes_the_version_in_the_manifest() {
    let version = workspace_version();
    let changelog = read("CHANGELOG.md");
    assert!(
        changelog_section(&changelog, &version).is_some(),
        "Cargo.toml says {version}, and CHANGELOG.md has no `## [{version}]` section"
    );
}

/// A stored-format bump forces every holder of an index to rebuild it, which
/// is a breaking change whatever else the release contains. The release notes
/// are generated from commits, so the commit trail is where it has to be said
/// — writing it only in the changelog is what left v0.3.0's notes silent.
#[test]
fn a_version_bump_that_forces_a_rebuild_is_marked_breaking_in_a_commit() {
    let commits = commits_since_last_tag();
    if commits.is_empty() {
        return;
    }

    // Did anything in this batch change a number that invalidates an index?
    let root = repo_root();
    let diff = Command::new("git")
        .args(["diff", "--unified=0", "HEAD~1..HEAD", "--", "crates/nooma-core/src/index.rs"])
        .current_dir(&root)
        .output();
    let Ok(diff) = diff else { return };
    let changed = String::from_utf8_lossy(&diff.stdout);
    let bumped = changed
        .lines()
        .any(|line| line.starts_with('+') && (line.contains("FORMAT_VERSION: u32") || line.contains("CHUNKER_VERSION: u32")));
    if !bumped {
        return;
    }

    assert!(
        commits.iter().any(|commit| marks_a_breaking_change(commit)),
        "a stored version number was bumped, which forces every held index to be \
         rebuilt, but no commit since the last tag marks itself breaking. Release \
         notes are generated from commits: add a `BREAKING CHANGE:` footer, or the \
         migration path reaches nobody who did not read the changelog in the repository."
    );
}

#[cfg(test)]
mod tests {
    use super::marks_a_breaking_change;

    #[test]
    fn both_spellings_of_a_breaking_change_are_recognized() {
        assert!(marks_a_breaking_change("feat(core)!: change the stored format"));
        assert!(marks_a_breaking_change("feat!: change it"));
        assert!(marks_a_breaking_change("feat(core): change it\n\nBREAKING CHANGE: the format is 3."));
        assert!(marks_a_breaking_change("feat(core): change it\n\nBREAKING-CHANGE: the format is 3."));
    }

    /// The spelling that shipped v0.3.0's notes empty: the migration path was
    /// in the body as prose, with no marker git-cliff reads.
    #[test]
    fn prose_about_a_change_is_not_a_marker() {
        assert!(!marks_a_breaking_change(
            "feat(core): summarize a module\n\nThe chunker version goes to 2: an index built before \
             summaries existed would be reused and every file in it would answer no summary forever."
        ));
        assert!(!marks_a_breaking_change("fix: a normal fix"));
        // An exclamation inside the description is not the Conventional
        // Commits marker, which sits before the colon.
        assert!(!marks_a_breaking_change("fix: this one is important!"));
    }
}
