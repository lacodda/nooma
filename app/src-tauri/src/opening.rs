//! Opening a found document at the place it was found.
//!
//! A note found by its seventh section and opened at its first line leaves
//! the person to find the place a second time. So a document is opened by the
//! program that would open it anyway - the one the system has for files like
//! it - and that program is told the place, when it is one whose way of being
//! told is known: an editor the line, a PDF reader the page. Any other is
//! handed the file, as a double click in the file manager would.
//!
//! A note in an Obsidian vault is the exception: it is Obsidian's, whatever
//! opens markdown elsewhere, and Obsidian is told it by its own link - with
//! the heading the note was found under, which is as near a line as the link
//! goes. Which folders are vaults Obsidian says itself, in the list of vaults
//! it keeps; a folder it does not list is not one it would open.
//!
//! What is decided here is decided before anything is opened, so the window
//! can say on the button what will happen: which program, and where.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use nooma_core::document::DocKind;
use serde::Serialize;

/// How long what the system says opens a kind of file is believed: a person
/// may choose another program while the window is open, and asking on every
/// selection costs a process on Linux.
const BELIEVED_FOR: Duration = Duration::from_secs(10);

/// Where a document was found, as it is to be opened.
#[derive(Debug, Clone, Copy)]
pub struct Place<'a> {
    pub path: &'a Path,
    pub kind: DocKind,
    /// The line the chunk starts on.
    pub line: u32,
    /// The page it is on, in a PDF.
    pub page: Option<u32>,
    /// The innermost heading it is under.
    pub heading: Option<&'a str>,
}

/// How a document will be opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Opening {
    /// The program, by the name a person knows it by; `None` when the system
    /// does not say which opens such files.
    pub program: Option<String>,
    /// Where in the document it opens; `None` at its start.
    pub at: Option<At>,
    #[serde(skip)]
    how: How,
}

/// The place a program is told.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum At {
    Line { line: u32 },
    Page { page: u32 },
    Heading { heading: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum How {
    /// A link the system hands to the program registered for it.
    Link(String),
    /// The program itself, with the place on its command line.
    Run { program: PathBuf, args: Vec<OsString> },
    /// The file, to whatever opens it.
    File,
}

/// A program the system opens files with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    /// What runs: an executable, or on macOS the application's bundle.
    pub executable: PathBuf,
    /// The name a person knows it by.
    pub name: String,
}

/// What the machine says: the program for a file, and Obsidian's vaults.
pub trait System {
    /// The program the system opens a file like this one with.
    fn program_for(&self, path: &Path, kind: DocKind) -> Option<Program>;
    /// The vaults Obsidian knows: each one's id and folder.
    fn vaults(&self) -> Vec<(String, PathBuf)>;
}

/// How a known program is told a place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tells {
    /// `-g file:line`: Visual Studio Code and the editors made from it.
    Goto,
    /// `file:line`.
    Colon,
    /// `-n<line> file`: Notepad++.
    Numbered,
    /// `-l <line> file`: Kate.
    LineFlag,
    /// `+<line> file`: gedit.
    Plus,
    /// The file as a link to its page: a browser's PDF viewer.
    PageLink,
    /// `/A page=<page> file`: Adobe's readers.
    OpenAction,
    /// `-page <page> file`: SumatraPDF.
    PageFlag,
    /// `-p <page> file`: Okular.
    ShortPage,
    /// `--page-index=<page> file`: Evince.
    PageIndex,
    /// `-P <page> file`: zathura.
    CapitalPage,
}

impl Tells {
    /// Whether this is a line, which only text has, or a page.
    const fn takes_lines(self) -> bool {
        matches!(self, Self::Goto | Self::Colon | Self::Numbered | Self::LineFlag | Self::Plus)
    }
}

/// A program whose way of being told a place is known.
struct Known {
    /// Its executable's name, lowercased and without its extension - and, on
    /// macOS, its application's.
    names: &'static [&'static str],
    /// On macOS, what inside the application takes the place: `{app}` is the
    /// application's name. `None` where nothing does.
    inside: Option<&'static str>,
    tells: Tells,
}

const KNOWN: &[Known] = &[
    Known {
        names: &["code", "code - insiders", "visual studio code", "visual studio code - insiders"],
        inside: Some("Contents/Resources/app/bin/code"),
        tells: Tells::Goto,
    },
    Known {
        names: &["codium", "vscodium"],
        inside: Some("Contents/Resources/app/bin/codium"),
        tells: Tells::Goto,
    },
    Known {
        names: &["cursor"],
        inside: Some("Contents/Resources/app/bin/cursor"),
        tells: Tells::Goto,
    },
    Known {
        names: &["sublime_text", "subl", "sublime text"],
        inside: Some("Contents/SharedSupport/bin/subl"),
        tells: Tells::Colon,
    },
    Known {
        names: &["zed"],
        inside: Some("Contents/MacOS/cli"),
        tells: Tells::Colon,
    },
    Known {
        names: &["notepad++"],
        inside: None,
        tells: Tells::Numbered,
    },
    Known {
        names: &["kate"],
        inside: None,
        tells: Tells::LineFlag,
    },
    Known {
        names: &["gedit"],
        inside: None,
        tells: Tells::Plus,
    },
    Known {
        names: &["firefox"],
        inside: Some("Contents/MacOS/firefox"),
        tells: Tells::PageLink,
    },
    Known {
        names: &[
            "msedge",
            "microsoft edge",
            "chrome",
            "google chrome",
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "brave",
            "brave browser",
            "brave-browser",
            "vivaldi",
        ],
        inside: Some("Contents/MacOS/{app}"),
        tells: Tells::PageLink,
    },
    Known {
        names: &["acrobat", "acrord32"],
        inside: None,
        tells: Tells::OpenAction,
    },
    Known {
        names: &["sumatrapdf"],
        inside: None,
        tells: Tells::PageFlag,
    },
    Known {
        names: &["okular"],
        inside: None,
        tells: Tells::ShortPage,
    },
    Known {
        names: &["evince"],
        inside: None,
        tells: Tells::PageIndex,
    },
    Known {
        names: &["zathura"],
        inside: None,
        tells: Tells::CapitalPage,
    },
];

/// How to open a document at a place, on this machine.
pub fn plan(place: &Place<'_>, system: &dyn System) -> Opening {
    if let Some(opening) = in_obsidian(place, system) {
        return opening;
    }
    let Some(program) = system.program_for(place.path, place.kind) else {
        return Opening {
            program: None,
            at: None,
            how: How::File,
        };
    };
    let told = told(place, &program);
    Opening {
        program: Some(program.name),
        at: told.as_ref().map(|(at, _)| at.clone()),
        how: told.map_or(How::File, |(_, how)| how),
    }
}

/// A note of a vault Obsidian knows, opened in Obsidian at its heading.
fn in_obsidian(place: &Place<'_>, system: &dyn System) -> Option<Opening> {
    // Obsidian's notes are `.md` files; another markdown file in a vault is
    // not one it shows.
    let is_note = place.kind == DocKind::Markdown && place.path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("md"));
    if !is_note {
        return None;
    }
    let (id, relative) = system
        .vaults()
        .into_iter()
        .filter_map(|(id, folder)| place.path.strip_prefix(&folder).ok().map(|relative| (id, relative.to_path_buf())))
        // A vault inside another: the innermost is the note's.
        .max_by_key(|(_, relative)| std::cmp::Reverse(relative.components().count()))?;
    let mut file: Vec<String> = relative.components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect();
    let name = file.pop()?;
    file.push(name.get(..name.len() - 3).unwrap_or(&name).to_string());
    let mut target = file.join("/");
    if let Some(heading) = place.heading {
        target.push('#');
        target.push_str(heading);
    }
    Some(Opening {
        program: Some("Obsidian".to_string()),
        at: place.heading.map(|heading| At::Heading { heading: heading.to_string() }),
        how: How::Link(format!("obsidian://open?vault={}&file={}", encoded(&id), encoded(&target))),
    })
}

/// The place, as this program is told it, if it is a program that can be.
fn told(place: &Place<'_>, program: &Program) -> Option<(At, How)> {
    let name = stem_of(&program.executable);
    let known = KNOWN.iter().find(|known| known.names.contains(&name.as_str()))?;
    let executable = launcher(&program.executable, known)?;
    let file = place.path.as_os_str().to_os_string();
    let with = |args: Vec<OsString>| How::Run {
        program: executable.clone(),
        args,
    };
    if known.tells.takes_lines() {
        if !place.kind.has_lines() {
            return None;
        }
        let line = place.line;
        let how = match known.tells {
            Tells::Goto => with(vec!["-g".into(), joined(&file, &format!(":{line}"))]),
            Tells::Colon => with(vec![joined(&file, &format!(":{line}"))]),
            Tells::Numbered => with(vec![format!("-n{line}").into(), file]),
            Tells::LineFlag => with(vec!["-l".into(), line.to_string().into(), file]),
            _ => with(vec![format!("+{line}").into(), file]),
        };
        return Some((At::Line { line }, how));
    }
    let page = place.page.filter(|_| place.kind == DocKind::Pdf)?;
    let how = match known.tells {
        Tells::PageLink => {
            let mut link = tauri::Url::from_file_path(place.path).ok()?;
            link.set_fragment(Some(&format!("page={page}")));
            with(vec![link.as_str().into()])
        }
        Tells::OpenAction => with(vec!["/A".into(), format!("page={page}").into(), file]),
        Tells::PageFlag => with(vec!["-page".into(), page.to_string().into(), file]),
        Tells::ShortPage => with(vec!["-p".into(), page.to_string().into(), file]),
        Tells::PageIndex => with(vec![format!("--page-index={page}").into(), file]),
        _ => with(vec!["-P".into(), page.to_string().into(), file]),
    };
    Some((At::Page { page }, how))
}

/// What to run to tell the program: the executable itself, or on macOS the
/// one inside the application that takes a place.
fn launcher(executable: &Path, known: &Known) -> Option<PathBuf> {
    let bundle = executable.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("app"));
    if !bundle {
        return Some(executable.to_path_buf());
    }
    let app = executable.file_stem()?.to_string_lossy().into_owned();
    let inside = executable.join(known.inside?.replace("{app}", &app));
    inside.is_file().then_some(inside)
}

/// An executable's or an application's name, as [`Known::names`] lists it.
fn stem_of(executable: &Path) -> String {
    executable.file_stem().map(|stem| stem.to_string_lossy().to_lowercase()).unwrap_or_default()
}

fn joined(file: &OsString, suffix: &str) -> OsString {
    let mut out = file.clone();
    out.push(suffix);
    out
}

/// A value for a link's query, every character but the unreserved ones
/// percent-encoded as UTF-8: Obsidian reads `+` as itself, so a space is
/// `%20`, and a `/` in a path is `%2F`, as its documentation asks.
fn encoded(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

impl Opening {
    /// Open the document as planned. A program that cannot be started, or a
    /// link nothing takes, falls back to handing the file to the system.
    pub fn open(&self, path: &Path, link: impl Fn(&str) -> Result<(), String>, file: impl Fn(&Path) -> Result<(), String>) -> Result<(), String> {
        match &self.how {
            How::Link(uri) => link(uri).or_else(|_| file(path)),
            How::Run { program, args } => run(program, args).or_else(|_| file(path)),
            How::File => file(path),
        }
    }
}

/// Start a program and leave it running. Its exit is waited for on a thread
/// of its own, so on Unix it does not linger as a zombie of the window.
fn run(program: &Path, args: &[OsString]) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", program.display()))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// The machine nooma runs on, asked once in a while.
#[derive(Default)]
pub struct Os {
    programs: Mutex<HashMap<String, (Instant, Option<Program>)>>,
}

impl System for Os {
    fn program_for(&self, path: &Path, kind: DocKind) -> Option<Program> {
        let key = path
            .extension()
            .map(|extension| extension.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| kind.name().to_string());
        let mut programs = self.programs.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((asked, program)) = programs.get(&key)
            && asked.elapsed() < BELIEVED_FOR
        {
            return program.clone();
        }
        let program = sys::program_for(path, kind);
        programs.insert(key, (Instant::now(), program.clone()));
        program
    }

    fn vaults(&self) -> Vec<(String, PathBuf)> {
        obsidian_lists()
            .into_iter()
            .filter_map(|list| std::fs::read_to_string(list).ok())
            .flat_map(|text| vaults_in(&text))
            .collect()
    }
}

/// Where Obsidian keeps its list of vaults: in the user's configuration, and
/// where a Flatpak or a Snap of it keeps its own.
fn obsidian_lists() -> Vec<PathBuf> {
    let Some(dirs) = directories::BaseDirs::new() else { return Vec::new() };
    let mut lists = vec![dirs.config_dir().join("obsidian").join("obsidian.json")];
    if cfg!(target_os = "linux") {
        let home = dirs.home_dir();
        lists.push(home.join(".var/app/md.obsidian.Obsidian/config/obsidian/obsidian.json"));
        lists.push(home.join("snap/obsidian/current/.config/obsidian/obsidian.json"));
    }
    lists
}

/// The vaults in Obsidian's list: `{"vaults": {"<id>": {"path": "<folder>"}}}`.
fn vaults_in(text: &str) -> Vec<(String, PathBuf)> {
    let Ok(list) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let Some(vaults) = list.get("vaults").and_then(serde_json::Value::as_object) else {
        return Vec::new();
    };
    vaults
        .iter()
        .filter_map(|(id, vault)| Some((id.clone(), PathBuf::from(vault.get("path")?.as_str()?))))
        .collect()
}

/// A desktop entry's program and name, from its `Exec` and `Name` keys: the
/// first word of `Exec` that is not `env` or a variable it sets.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn desktop_entry(text: &str) -> Option<Program> {
    let mut in_entry = false;
    let (mut exec, mut name) = (None, None);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some(value) = line.strip_prefix("Exec=") {
            exec.get_or_insert(value.to_string());
        } else if let Some(value) = line.strip_prefix("Name=") {
            name.get_or_insert(value.to_string());
        }
    }
    let exec = exec?;
    let program = exec
        .split_whitespace()
        .map(|word| word.trim_matches('"'))
        .find(|word| *word != "env" && !word.contains('='))?;
    Some(Program {
        executable: PathBuf::from(program),
        name: name.unwrap_or_else(|| program.to_string()),
    })
}

#[cfg(windows)]
mod sys {
    use std::path::{Path, PathBuf};

    use nooma_core::document::DocKind;
    use windows::Win32::UI::Shell::{ASSOCF_NOTRUNCATE, ASSOCSTR, ASSOCSTR_EXECUTABLE, ASSOCSTR_FRIENDLYAPPNAME, AssocQueryStringW};
    use windows::core::{HSTRING, PWSTR, w};

    use super::Program;

    /// What the shell opens the file's extension with: the program a double
    /// click would start, as the person chose it.
    pub fn program_for(path: &Path, _kind: DocKind) -> Option<Program> {
        let extension = format!(".{}", path.extension()?.to_string_lossy());
        let executable = PathBuf::from(query(&extension, ASSOCSTR_EXECUTABLE)?);
        let name = query(&extension, ASSOCSTR_FRIENDLYAPPNAME).unwrap_or_else(|| super::stem_of(&executable));
        Some(Program { executable, name })
    }

    fn query(extension: &str, what: ASSOCSTR) -> Option<String> {
        let extension = HSTRING::from(extension);
        let mut length = 0u32;
        // Asked once for the length, then for the text.
        // SAFETY: a null output with a length to fill is the documented way
        // to ask for the length.
        unsafe { AssocQueryStringW(ASSOCF_NOTRUNCATE, what, &extension, w!("open"), None, &raw mut length) }
            .ok()
            .ok()?;
        if length == 0 {
            return None;
        }
        let mut buffer = vec![0u16; length as usize];
        // SAFETY: the buffer holds `length` characters, as told.
        unsafe {
            AssocQueryStringW(
                ASSOCF_NOTRUNCATE,
                what,
                &extension,
                w!("open"),
                Some(PWSTR(buffer.as_mut_ptr())),
                &raw mut length,
            )
        }
        .ok()
        .ok()?;
        let text = String::from_utf16_lossy(&buffer[..buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len())]);
        (!text.is_empty()).then_some(text)
    }
}

#[cfg(target_os = "linux")]
mod sys {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use nooma_core::document::DocKind;

    use super::{Program, desktop_entry};

    /// What `xdg-mime` says opens the kind's type, read from its desktop
    /// entry.
    pub fn program_for(_path: &Path, kind: DocKind) -> Option<Program> {
        let mime = match kind {
            DocKind::Markdown => "text/markdown",
            DocKind::Text => "text/plain",
            DocKind::Pdf => "application/pdf",
            _ => return None,
        };
        let output = Command::new("xdg-mime").args(["query", "default", mime]).output().ok()?;
        let id = String::from_utf8(output.stdout).ok()?;
        let id = id.trim();
        if id.is_empty() {
            return None;
        }
        data_dirs()
            .into_iter()
            .find_map(|dir| std::fs::read_to_string(dir.join("applications").join(id)).ok())
            .and_then(|text| desktop_entry(&text))
    }

    fn data_dirs() -> Vec<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut dirs: Vec<PathBuf> = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|home| home.join(".local/share")))
            .into_iter()
            .collect();
        let shared = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
        dirs.extend(shared.split(':').filter(|dir| !dir.is_empty()).map(PathBuf::from));
        dirs
    }
}

#[cfg(target_os = "macos")]
mod sys {
    use std::ffi::{OsStr, c_void};
    use std::os::unix::ffi::OsStrExt as _;
    use std::path::{Path, PathBuf};

    use nooma_core::document::DocKind;

    use super::Program;

    type CFUrl = *const c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFURLCreateFromFileSystemRepresentation(allocator: *const c_void, buffer: *const u8, length: isize, directory: u8) -> CFUrl;
        fn CFURLGetFileSystemRepresentation(url: CFUrl, resolve: u8, buffer: *mut u8, length: isize) -> u8;
        fn CFRelease(object: *const c_void);
    }

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSCopyDefaultApplicationURLForURL(url: CFUrl, roles: u32, error: *mut *const c_void) -> CFUrl;
    }

    /// Every role an application can have for a file.
    const ALL_ROLES: u32 = 0xFFFF_FFFF;

    /// The application Launch Services opens the file with.
    pub fn program_for(path: &Path, _kind: DocKind) -> Option<Program> {
        let bytes = path.as_os_str().as_bytes();
        // SAFETY: the bytes are a path, given with their length; every
        // object created is released below.
        let application = unsafe {
            let file = CFURLCreateFromFileSystemRepresentation(std::ptr::null(), bytes.as_ptr(), isize::try_from(bytes.len()).ok()?, 0);
            if file.is_null() {
                return None;
            }
            let application = LSCopyDefaultApplicationURLForURL(file, ALL_ROLES, std::ptr::null_mut());
            CFRelease(file);
            if application.is_null() {
                return None;
            }
            let mut buffer = vec![0u8; 4096];
            let read = CFURLGetFileSystemRepresentation(application, 1, buffer.as_mut_ptr(), isize::try_from(buffer.len()).ok()?);
            CFRelease(application);
            if read == 0 {
                return None;
            }
            let end = buffer.iter().position(|byte| *byte == 0).unwrap_or(buffer.len());
            PathBuf::from(OsStr::from_bytes(&buffer[..end]))
        };
        let name = application.file_stem()?.to_string_lossy().into_owned();
        Some(Program { executable: application, name })
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod sys {
    use std::path::Path;

    use nooma_core::document::DocKind;

    use super::Program;

    pub fn program_for(_path: &Path, _kind: DocKind) -> Option<Program> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A machine that says what it is told to.
    struct Fake {
        program: Option<&'static str>,
        vaults: Vec<(&'static str, PathBuf)>,
    }

    impl System for Fake {
        fn program_for(&self, _path: &Path, _kind: DocKind) -> Option<Program> {
            self.program.map(|executable| Program {
                executable: PathBuf::from(executable),
                name: format!("the {executable} program"),
            })
        }

        fn vaults(&self) -> Vec<(String, PathBuf)> {
            self.vaults.iter().map(|(id, folder)| ((*id).to_string(), folder.clone())).collect()
        }
    }

    fn opened_by(program: &'static str) -> Fake {
        Fake {
            program: Some(program),
            vaults: Vec::new(),
        }
    }

    fn root() -> PathBuf {
        std::env::temp_dir().join("nooma-opening")
    }

    fn note(name: &str) -> PathBuf {
        root().join("Notes").join(name)
    }

    fn place(path: &Path, kind: DocKind) -> Place<'_> {
        Place {
            path,
            kind,
            line: 12,
            page: (kind == DocKind::Pdf).then_some(4),
            heading: Some("Watering"),
        }
    }

    fn run_of(opening: &Opening) -> (String, Vec<String>) {
        match &opening.how {
            How::Run { program, args } => (
                program.to_string_lossy().into_owned(),
                args.iter().map(|arg| arg.to_string_lossy().into_owned()).collect(),
            ),
            other => panic!("not a program run: {other:?}"),
        }
    }

    #[test]
    fn an_editor_that_takes_a_line_is_told_it() {
        let path = note("greenhouse.md");
        let file = path.to_string_lossy().into_owned();
        let opening = plan(&place(&path, DocKind::Markdown), &opened_by("/apps/Code.exe"));
        assert_eq!(opening.program.as_deref(), Some("the /apps/Code.exe program"));
        assert_eq!(opening.at, Some(At::Line { line: 12 }));
        assert_eq!(run_of(&opening), ("/apps/Code.exe".to_string(), vec!["-g".to_string(), format!("{file}:12")]));

        let cases: [(&str, Vec<String>); 4] = [
            ("/usr/bin/subl", vec![format!("{file}:12")]),
            ("C:/Program Files/Notepad++/notepad++.exe", vec!["-n12".to_string(), file.clone()]),
            ("/usr/bin/kate", vec!["-l".to_string(), "12".to_string(), file.clone()]),
            ("/usr/bin/gedit", vec!["+12".to_string(), file.clone()]),
        ];
        for (program, args) in cases {
            assert_eq!(run_of(&plan(&place(&path, DocKind::Text), &opened_by(program))).1, args, "{program}");
        }
    }

    #[test]
    fn a_pdf_reader_that_takes_a_page_is_told_it() {
        let path = root().join("Archive").join("hand book.pdf");
        let file = path.to_string_lossy().into_owned();
        let browser = plan(&place(&path, DocKind::Pdf), &opened_by("C:/Edge/msedge.exe"));
        assert_eq!(browser.at, Some(At::Page { page: 4 }));
        let (_, args) = run_of(&browser);
        assert!(args[0].starts_with("file:///") && args[0].ends_with("hand%20book.pdf#page=4"), "{args:?}");

        let cases: [(&str, Vec<String>); 4] = [
            ("C:/Adobe/Acrobat.exe", vec!["/A".to_string(), "page=4".to_string(), file.clone()]),
            ("C:/Sumatra/SumatraPDF.exe", vec!["-page".to_string(), "4".to_string(), file.clone()]),
            ("/usr/bin/okular", vec!["-p".to_string(), "4".to_string(), file.clone()]),
            ("/usr/bin/evince", vec!["--page-index=4".to_string(), file.clone()]),
        ];
        for (program, args) in cases {
            assert_eq!(run_of(&plan(&place(&path, DocKind::Pdf), &opened_by(program))).1, args, "{program}");
        }
    }

    #[test]
    fn a_program_not_known_to_take_the_place_is_handed_the_file() {
        let path = note("greenhouse.md");
        let notepad = plan(&place(&path, DocKind::Markdown), &opened_by("C:/Windows/notepad.exe"));
        assert_eq!(
            (notepad.program.as_deref(), &notepad.at, &notepad.how),
            (Some("the C:/Windows/notepad.exe program"), &None, &How::File)
        );

        // An editor is told lines, and a document without lines has none.
        let book = root().join("lighthouse.epub");
        assert_eq!(plan(&place(&book, DocKind::Epub), &opened_by("/apps/code")).how, How::File);
        // A browser is told pages, and a note has none.
        assert_eq!(plan(&place(&path, DocKind::Markdown), &opened_by("/apps/firefox")).how, How::File);

        let nothing = plan(
            &place(&path, DocKind::Markdown),
            &Fake {
                program: None,
                vaults: Vec::new(),
            },
        );
        assert_eq!((nothing.program, nothing.at, nothing.how), (None, None, How::File));
    }

    #[test]
    fn a_note_of_a_vault_obsidian_knows_opens_in_obsidian_at_its_heading() {
        let path = root().join("Vault").join("Home life").join("Зимний сад.md");
        let system = Fake {
            program: Some("/apps/Code.exe"),
            vaults: vec![("4b12", root().join("Vault")), ("other", root().join("Elsewhere"))],
        };
        let opening = plan(&place(&path, DocKind::Markdown), &system);
        assert_eq!(opening.program.as_deref(), Some("Obsidian"));
        assert_eq!(
            opening.at,
            Some(At::Heading {
                heading: "Watering".to_string()
            })
        );
        assert_eq!(
            opening.how,
            How::Link("obsidian://open?vault=4b12&file=Home%20life%2F%D0%97%D0%B8%D0%BC%D0%BD%D0%B8%D0%B9%20%D1%81%D0%B0%D0%B4%23Watering".to_string())
        );

        let top = Place {
            heading: None,
            ..place(&path, DocKind::Markdown)
        };
        let opening = plan(&top, &system);
        assert_eq!(opening.at, None, "before the first heading it opens at the top");
        assert!(matches!(&opening.how, How::Link(link) if link.ends_with("%D1%81%D0%B0%D0%B4")));

        // Outside every vault, and a markdown file Obsidian does not show,
        // go to the editor.
        let outside = note("greenhouse.md");
        assert_eq!(plan(&place(&outside, DocKind::Markdown), &system).at, Some(At::Line { line: 12 }));
        let markdown = root().join("Vault").join("readme.markdown");
        assert_eq!(plan(&place(&markdown, DocKind::Markdown), &system).at, Some(At::Line { line: 12 }));
    }

    #[test]
    fn a_vault_inside_another_is_the_one_its_notes_open_in() {
        let path = root().join("Vault").join("Inner").join("note.md");
        let system = Fake {
            program: None,
            vaults: vec![("outer", root().join("Vault")), ("inner", root().join("Vault").join("Inner"))],
        };
        assert!(
            matches!(&plan(&place(&path, DocKind::Markdown), &system).how, How::Link(link) if link.starts_with("obsidian://open?vault=inner&file=note%23"))
        );
    }

    #[test]
    fn obsidians_list_of_vaults_is_read() {
        let list = r#"{"vaults":{"4b12":{"path":"C:\\Notes\\Vault","ts":1,"open":true},"broken":{"ts":2}},"frame":"hidden"}"#;
        assert_eq!(vaults_in(list), vec![("4b12".to_string(), PathBuf::from("C:\\Notes\\Vault"))]);
        assert!(vaults_in("not json").is_empty());
        assert!(vaults_in("{}").is_empty());
    }

    #[test]
    fn a_desktop_entry_names_its_program() {
        let entry = "[Desktop Entry]\nName=Visual Studio Code\nExec=env BAMF=1 /usr/share/code/code --unity-launch %F\n\n[Desktop Action new-empty-window]\nName=New Empty Window\nExec=/usr/share/code/code --new-window %F\n";
        assert_eq!(
            desktop_entry(entry),
            Some(Program {
                executable: PathBuf::from("/usr/share/code/code"),
                name: "Visual Studio Code".to_string()
            })
        );
        assert_eq!(desktop_entry("[Desktop Entry]\nName=Nothing\n"), None);
    }

    #[test]
    fn a_link_nothing_takes_falls_back_to_the_file() {
        let path = note("greenhouse.md");
        let opening = Opening {
            program: Some("Obsidian".to_string()),
            at: None,
            how: How::Link("obsidian://open?vault=x&file=y".to_string()),
        };
        let handed = Mutex::new(Vec::new());
        opening
            .open(
                &path,
                |_| Err("no handler".to_string()),
                |file| {
                    handed.lock().unwrap().push(file.to_path_buf());
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(handed.into_inner().unwrap(), vec![path.clone()]);

        let missing = Opening {
            program: Some("gone".to_string()),
            at: Some(At::Line { line: 1 }),
            how: How::Run {
                program: root().join("no-such-program"),
                args: Vec::new(),
            },
        };
        let handed = Mutex::new(0);
        missing
            .open(
                &path,
                |_| unreachable!(),
                |_| {
                    *handed.lock().unwrap() += 1;
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(*handed.lock().unwrap(), 1, "a program that cannot start leaves the file to the system");
    }

    /// The machine's own answer, where there is one: an executable that is
    /// there. What a CI machine opens files with is its own business.
    #[test]
    fn the_system_names_a_program_that_exists() {
        let os = Os::default();
        if let Some(program) = os.program_for(Path::new("a.txt"), DocKind::Text) {
            assert!(program.executable.exists(), "{program:?}");
            assert_eq!(
                os.program_for(Path::new("b.txt"), DocKind::Text),
                Some(program),
                "asked again within the while, it is believed"
            );
        }
    }
}
