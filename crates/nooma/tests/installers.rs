//! The installers, held to what the line learned about them the hard way.
//!
//! They only ever run on somebody else's machine, after a release, so a
//! mistake in them arrives as a confused user rather than a failed build. Each
//! test here is a mistake one of the line's tools shipped once.

use std::path::{Path, PathBuf};

/// The repository's root: the nearest directory above this crate holding the
/// installers. A crate in a workspace sits two levels below it.
fn root() -> PathBuf {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    crate_dir
        .ancestors()
        .find(|dir| dir.join("tools").join("install.sh").is_file())
        .unwrap_or_else(|| panic!("no tools/install.sh in or above {}", crate_dir.display()))
        .to_path_buf()
}

fn read(installer: &str) -> String {
    let path = root().join("tools").join(installer);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// An installer without its comment lines: the comments explaining a rule
/// name what the rule forbids.
fn code(installer: &str) -> String {
    read(installer)
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The prefix of the installers' environment: `DEMO_TOOL` for `demo-tool`.
fn env_prefix() -> String {
    env!("CARGO_PKG_NAME").to_ascii_uppercase().replace('-', "_")
}

/// Git Bash, MSYS2 and Cygwin all run a `sh` script on Windows. Without a case
/// of its own for them the script answers "no prebuilt binary for
/// MINGW64_NT-...", which reads as "there is no Windows build" - when there is
/// one, installed by the other script.
///
/// Run rather than read: a case that exists but sits below the catch-all
/// would satisfy a search of the text and never match. `uname` is replaced by
/// a shell function, so nothing executable is written for the test to run.
#[cfg(unix)]
#[test]
fn the_unix_installer_sends_windows_shells_to_powershell() {
    let installer = root().join("tools").join("install.sh");
    for system in ["MINGW64_NT-10.0-26200", "MSYS_NT-10.0-26200", "CYGWIN_NT-10.0-26200"] {
        let script = ["uname() { case \"$1\" in -s) echo ", system, " ;; *) echo x86_64 ;; esac; }; . \"$0\""].concat();
        let output = std::process::Command::new("sh")
            .args(["-c", &script])
            .arg(&installer)
            .output()
            .expect("cannot run sh");
        let said = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "on {system}, install.sh went on as if it could install");
        assert!(
            said.contains("install.ps1"),
            "on {system}, install.sh does not send the user to install.ps1:\n{said}"
        );
        assert!(!said.contains("No prebuilt binary"), "on {system}, install.sh says there is no build:\n{said}");
    }
}

/// The fallback an installer suggests has to work: `cargo install` with a
/// name that is not the crate's fails, and a suggestion that fails is worse
/// than none. kasl suggested `kasl` while publishing `kasl-cli`.
#[test]
fn the_installers_suggest_the_crate_that_is_published() {
    let name = env!("CARGO_PKG_NAME");
    for installer in ["install.sh", "install.ps1"] {
        for (number, line) in read(installer).lines().enumerate() {
            let Some(at) = line.find("cargo install ") else { continue };
            let named = line[at + "cargo install ".len()..]
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_');
            assert_eq!(
                named,
                name,
                "{installer} line {} suggests `cargo install {named}`, but the crate is `{name}`",
                number + 1
            );
        }
    }
}

/// The Windows installer adds itself to the user PATH, which is almost always
/// an expandable string: entries like `%JAVA_HOME%\bin` are stored unexpanded.
/// `[Environment]::SetEnvironmentVariable` writes it back as a plain string,
/// and every such entry becomes literal text - one installer of the line did.
/// Comments are left out of the search: the one explaining this names the call.
#[test]
fn the_windows_installer_keeps_the_path_expandable() {
    let code = code("install.ps1");
    assert!(
        !code.contains("SetEnvironmentVariable"),
        "install.ps1 calls SetEnvironmentVariable, which turns the PATH into a plain string"
    );
    assert!(
        code.contains("DoNotExpandEnvironmentNames") && code.contains("ExpandString"),
        "install.ps1 has to read the PATH unexpanded and write it back as an expandable string"
    );
}

/// `<APP>_NO_PATH` leaves the PATH alone. The release check that proves an
/// installer works installs into a scratch directory, and an installer that
/// edits the PATH anyway leaves that directory on the user's PATH for good.
/// The flag has to be read before the first edit, not after it.
#[test]
fn the_installers_leave_the_path_alone_when_asked() {
    let flag = format!("{}_NO_PATH", env_prefix());

    let ps1 = code("install.ps1");
    let read_at = ps1.find(&format!("$env:{flag}")).unwrap_or_else(|| panic!("install.ps1 never reads {flag}"));
    let edit_at = ps1.find("OpenSubKey(\"Environment\"").expect("install.ps1 edits the user PATH");
    assert!(read_at < edit_at, "install.ps1 reads {flag} only after it has edited the PATH");

    let sh = code("install.sh");
    let read_at = sh.find(&flag).unwrap_or_else(|| panic!("install.sh never reads {flag}"));
    let note_at = sh.find("to your PATH").expect("install.sh says where the program went");
    assert!(read_at < note_at, "install.sh reads {flag} only after it has talked about the PATH");
}

/// Windows will not overwrite a program that is running, but it will rename
/// it. An installer that copies over the old exe fails exactly when it is
/// needed most: updating a program from inside itself, or while it serves.
#[test]
fn the_windows_installer_moves_a_running_program_aside() {
    let ps1 = code("install.ps1");
    let aside = ps1.find("Move-Item $target").expect("install.ps1 never moves the installed program aside");
    let copy = ps1.find("Copy-Item").expect("install.ps1 copies the program into place");
    assert!(aside < copy, "install.ps1 moves the old program aside only after copying over it");
    assert!(ps1.contains(".exe.old-*"), "install.ps1 never cleans up the programs it moved aside");
}

/// The targets a text names: every `<arch>-<vendor>-<os>[-<env>]` triple in it.
fn targets(text: &str) -> std::collections::BTreeSet<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
        .map(|word| word.trim_matches('-'))
        .filter_map(|word| {
            let at = ["x86_64-", "aarch64-"].iter().find_map(|arch| word.find(arch))?;
            let triple = &word[at..];
            (triple.split('-').count() >= 3).then(|| triple.to_string())
        })
        .collect()
}

fn text(relative: &str) -> String {
    let path = root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// An installer asks the release page for an archive by its target, the
/// release workflow builds the archives, and `release.toml` lists what the
/// page has to carry. Three places, written apart: a target added to one of
/// them alone is a 404 for whoever runs the installer on that machine, or an
/// archive nobody can install.
#[test]
fn the_installers_ask_for_what_a_release_carries() {
    let asked: std::collections::BTreeSet<String> = targets(&code("install.sh")).union(&targets(&code("install.ps1"))).cloned().collect();
    assert!(!asked.is_empty(), "the installers name no target at all");

    let workflow = text(".github/workflows/release.yml");
    let jobs = workflow.find("\njobs:").expect("release.yml has jobs");
    let desktop = workflow.find("\n  desktop:").expect("release.yml builds the window apart from the program");
    let built = targets(&workflow[jobs..desktop]);
    assert_eq!(asked, built, "the installers ask for one set of targets and release.yml builds another");

    let name = env!("CARGO_PKG_NAME");
    let listed: std::collections::BTreeSet<String> = text("release.toml")
        .lines()
        .filter(|line| line.trim_start().starts_with(&format!("\"{name}-{{tag}}-")))
        .flat_map(targets)
        .collect();
    assert_eq!(asked, listed, "the installers ask for one set of targets and release.toml lists another");

    for (installer, archive) in [("install.sh", ".tar.gz"), ("install.ps1", ".zip")] {
        for target in targets(&code(installer)) {
            let file = format!("\"{name}-{{tag}}-{target}{archive}\"");
            assert!(
                text("release.toml").contains(&file),
                "release.toml does not list {file}, which {installer} fetches"
            );
        }
    }
}

/// The line a person copies is written three times: at the head of each
/// installer, in the README and on the Getting Started page. They have to be
/// one line, and it has to name the script that is there.
#[test]
fn the_documentation_installs_with_the_installers() {
    for installer in ["install.sh", "install.ps1"] {
        let script = read(installer);
        let line = script
            .lines()
            .filter_map(|line| line.strip_prefix('#'))
            .map(str::trim)
            .find(|line| line.contains("raw.githubusercontent.com"))
            .unwrap_or_else(|| panic!("{installer} does not say how it is run"));
        assert!(
            line.contains(&format!("/main/tools/{installer}")),
            "{installer} is run from another address: {line}"
        );
        for page in ["README.md", "docs/src/content/docs/getting-started.md"] {
            assert!(text(page).contains(line), "{page} does not install with `{line}`");
        }
    }
}

/// The variables an installer hears are a promise of the documentation, and
/// the release engine sets them by name: `<APP>_VERSION` to pin the release,
/// `<APP>_INSTALL_DIR` and `<APP>_NO_PATH` for a copy in a scratch directory.
#[test]
fn the_installers_hear_the_three_variables() {
    let page = text("docs/src/content/docs/getting-started.md");
    for variable in ["VERSION", "INSTALL_DIR", "NO_PATH"] {
        let variable = format!("{}_{variable}", env_prefix());
        for installer in ["install.sh", "install.ps1"] {
            assert!(code(installer).contains(&variable), "{installer} never reads {variable}");
        }
        assert!(page.contains(&variable), "Getting Started does not say what {variable} does");
    }
}
