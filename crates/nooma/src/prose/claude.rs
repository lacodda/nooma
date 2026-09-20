//! Asking the Claude Code CLI the user already installed.
//!
//! nooma holds no API key and no account. The user installed Claude Code for
//! their own reasons, logged into their own subscription, and this asks it a
//! question the way a person would — see [ADR 0004] for why that rather than a
//! key in a config file.
//!
//! [ADR 0004]: https://github.com/lacodda/nooma/blob/main/docs/adr/0004-prose-summaries-shell-out.md
//!
//! # Why the call is stripped down to nothing
//!
//! `claude --print` is an *agent*: every invocation carries a system prompt,
//! the descriptions of a dozen tools, the user's skills, their MCP servers and
//! any `CLAUDE.md` in reach. Measured on this line's own machine, one call
//! that answered with the single word "pong" carried 20775 tokens of prompt
//! before the question was even asked, and cost $0.216.
//!
//! The flags below take all of it away — no tools, no MCP servers, no skills,
//! no settings files — leaving the system prompt written here and the summary
//! being asked about. The same "pong" then carries 452 tokens and costs
//! $0.00066, a factor of 300.
//!
//! What a *real* module costs is larger than that floor and worth stating
//! honestly: measured over the ten modules of `nooma-core`, $0.2021, or about
//! two cents each. A well-documented module is mostly prompt — its header and
//! its docs are the input — so the bill scales with how much the author
//! already wrote. Call it $2 per hundred modules, paid once per distinct file
//! and never again.
//!
//! `--bare` looks like the flag for this and is not: it refuses OAuth and
//! insists on `ANTHROPIC_API_KEY`, so on a subscription it answers every
//! prompt with "Not logged in".

use std::process::{Command, Stdio};

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;

/// The executable to run.
///
/// On Windows the installer puts a `.cmd` shim on the PATH, and
/// `Command::new` does not find a `.cmd` by its bare name.
#[cfg(windows)]
const EXECUTABLE: &str = "claude.cmd";
#[cfg(not(windows))]
const EXECUTABLE: &str = "claude";

/// What to say when the CLI is nowhere on the PATH.
///
/// One sentence in one place, so the probe and a failed run say the same
/// thing rather than two things that drift apart.
pub const MISSING: &str = "Claude Code is not on the PATH. Install it from https://claude.com/claude-code, then run this again — or drop --prose for the summary lifted from the source.";

/// What the model is told it is doing.
///
/// Two things are being defended against, and both were seen in a real run
/// before this wording existed.
///
/// The first is invention. The model is shown a *surface* — signatures and
/// docs, never a body — and is trained to complete it into something
/// plausible. Given a module stripped of its docs it wrote "a comprehensive
/// list of all supported languages" about a constant it knew only the name of.
/// Every rule below that sounds redundant is there because of that run.
///
/// The second is length. A summary is meant to be read at a glance and
/// embedded whole; left alone the model writes an essay with headings.
const SYSTEM_PROMPT: &str = "\
You describe what a source module is for, in one short paragraph.

You are given a module summary: the path, the header comment if the file has one, and each public declaration with its signature and its documentation. Every word of it was written by the module's author.

Write 2 to 4 sentences of plain prose. No markdown, no lists, no headings, no preamble, no closing remark. Reply with the paragraph and nothing else.

Rules:
- Say what the module is for, and what a caller comes to it for.
- Do not restate the declarations. Whoever reads this already has the list.
- Write only what the summary supports. You are reading a surface, not the code: you cannot see a single function body, so never say what a function does internally, never say how complete or exhaustive a list is, and never explain what an identifier means beyond what its own name and documentation say.
- Prefer a shorter, duller sentence over a confident one the summary does not support.
- If the summary is too thin to say what the module is for, say what it declares and stop. A short honest answer is a correct answer.";

/// Where the CLI is and whether it can be used.
#[derive(Debug, Clone)]
pub struct Availability {
    /// Whether a call would be worth attempting.
    pub available: bool,
    /// What `claude --version` reported.
    pub version: Option<String>,
    /// What to tell the user when it is not available.
    pub reason: Option<String>,
}

/// Look for the CLI and report what was found.
///
/// This never fails. "Not installed" is an answer to show the user, not an
/// error that stops the command: the structural summary is still there, and
/// it is the part that does not need anything installed.
pub fn probe() -> Availability {
    match version() {
        Ok(version) => Availability {
            available: true,
            version: Some(version),
            reason: None,
        },
        Err(error) => Availability {
            available: false,
            version: None,
            reason: Some(error.to_string()),
        },
    }
}

fn version() -> Result<String> {
    let output = command().arg("--version").output().map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => anyhow!("{MISSING}"),
        _ => anyhow!("could not run `{EXECUTABLE}`: {error}"),
    })?;
    if !output.status.success() {
        bail!("`{EXECUTABLE} --version` failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// What one turn of the CLI reports.
///
/// Only the fields used are named, and unknown ones are ignored: the CLI's
/// output grows over time, and nooma does not version it.
#[derive(Debug, Deserialize)]
struct CliResult {
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    total_cost_usd: Option<f64>,
}

/// One answer, and what it cost.
#[derive(Debug, Clone)]
pub struct Turn {
    /// The paragraph.
    pub text: String,
    /// What the CLI said the call cost, when it said.
    pub cost_usd: Option<f64>,
}

/// Ask for a paragraph about one module summary.
pub fn describe(summary_text: &str) -> Result<Turn> {
    let prompt = format!("Module summary:\n\n{summary_text}");
    let raw = run(&prompt)?;
    let parsed: CliResult = serde_json::from_str(raw.trim()).map_err(|error| anyhow!("could not read the reply from Claude Code ({error}): {}", raw.trim()))?;
    let text = parsed.result.unwrap_or_default();

    if parsed.is_error {
        // The CLI puts its own diagnosis in `result` — "Not logged in",
        // "Credit balance too low". Passing it through tells the user what to
        // do; replacing it with wording of our own would not.
        bail!(if text.trim().is_empty() {
            "Claude Code reported an error".to_owned()
        } else {
            text
        });
    }

    let text = text.trim().to_owned();
    if text.is_empty() {
        bail!("Claude Code returned an empty summary");
    }
    Ok(Turn {
        text,
        cost_usd: parsed.total_cost_usd,
    })
}

/// Run one non-interactive turn and return its stdout.
fn run(prompt: &str) -> Result<String> {
    let mut command = command();
    command.args(["--print", "--output-format", "json"]);
    for argument in stripped_harness() {
        command.arg(argument);
    }
    // The working directory decides which `CLAUDE.md` files are in reach and
    // which repository the CLI thinks it is in. The answer must depend on the
    // summary alone, so it is asked from a directory with nothing in it.
    let empty = tempfile::tempdir()?;
    command.current_dir(empty.path());

    // The system prompt goes in a file, for the same reason the user prompt
    // goes over stdin: on Windows the executable is a `.cmd`, and Rust refuses
    // to pass an argument containing a newline to a batch file. `--version`
    // takes no such argument and succeeds, so this shows up only on a real
    // call, as `batch file arguments are invalid`.
    //
    // `--system-prompt-file` is not in `claude --help`'s option list — it is
    // named only inside the description of `--bare` — but it is accepted, and
    // it is the only form that survives a multi-line prompt here.
    let prompt_file = empty.path().join("system-prompt.txt");
    std::fs::write(&prompt_file, SYSTEM_PROMPT)?;
    command.arg("--system-prompt-file").arg(&prompt_file);
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => anyhow!("{MISSING}"),
        _ => anyhow!("could not run `{EXECUTABLE}`: {error}"),
    })?;

    {
        use std::io::Write as _;
        // The prompt goes over stdin, not as an argument. On Windows the
        // executable is a `.cmd`, and Rust refuses to pass an argument
        // containing a newline to a batch file — a deliberate guard against
        // argument injection. A module summary is nothing but newlines.
        let mut stdin = child.stdin.take().ok_or_else(|| anyhow!("could not write to Claude Code"))?;
        stdin.write_all(prompt.as_bytes())?;
    }

    let output = child.wait_with_output()?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if stdout.trim().is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(if stderr.trim().is_empty() {
            "Claude Code returned nothing".to_owned()
        } else {
            stderr.trim().to_owned()
        });
    }
    Ok(stdout)
}

/// The flags that take the agent away and leave the question.
///
/// Each one removes a source of prompt the answer does not need: the built-in
/// tools, the user's MCP servers, their skills, and their settings files
/// (which is what pulls in project and user `CLAUDE.md`). Together they are
/// what makes the feature affordable — see the note at the top of this file.
///
/// Held as a function rather than written inline so the cost test has
/// something to name.
fn stripped_harness() -> [&'static str; 12] {
    [
        // The smallest model, named rather than inherited. Without this the
        // call runs on whatever the user's default is — which for the author's
        // own machine is Opus, and which is what made the first measurement
        // $0.216 for the single word "pong". Rewriting a summary already in
        // hand is not work that wants the largest model, and letting the
        // choice depend on someone's settings would make the price of a
        // repository depend on a file this command has otherwise taken care
        // to ignore.
        "--model",
        "haiku",
        // No built-in tools: nothing here asks Claude to read or write a file.
        "--tools",
        "",
        // The lowest reasoning effort the CLI accepts. Rewriting a summary
        // that is already in hand into three sentences is not a task that
        // needs deliberation, and by default the model spends far more on
        // thinking than on the answer: measured on one module, 332 of 341
        // output tokens were thinking, and the call cost $0.0148 rather than
        // $0.002. `low` is the floor — `none` and `minimal` are refused with a
        // warning and the default is used instead, which is the expensive
        // case arriving silently.
        "--effort",
        "low",
        // No MCP servers, and none picked up from the user's configuration.
        "--strict-mcp-config",
        "--mcp-config",
        "{\"mcpServers\":{}}",
        // No skills.
        "--disable-slash-commands",
        // No user, project or local settings, which is also what keeps a
        // stray CLAUDE.md out of the prompt.
        "--setting-sources",
        "",
    ]
}

fn command() -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut command = Command::new(EXECUTABLE);
    // Without this a console window flashes on every call on Windows.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_never_panics_and_always_explains_itself() {
        let availability = probe();
        if availability.available {
            assert!(availability.version.is_some());
        } else {
            assert!(availability.reason.is_some(), "an unavailable CLI must come with something to show the user");
        }
    }

    #[test]
    fn an_error_reply_is_reported_in_the_clis_own_words() {
        let raw = r#"{"is_error":true,"result":"Not logged in · Please run /login"}"#;
        let parsed: CliResult = serde_json::from_str(raw).unwrap();
        assert!(parsed.is_error);
        assert_eq!(parsed.result.as_deref(), Some("Not logged in · Please run /login"));
    }

    #[test]
    fn a_successful_reply_carries_the_text_and_the_cost() {
        let raw = r#"{"is_error":false,"result":"What it is for.","total_cost_usd":0.0007}"#;
        let parsed: CliResult = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.result.as_deref(), Some("What it is for."));
        assert_eq!(parsed.total_cost_usd, Some(0.0007));
    }

    /// The CLI's output grows between releases and nooma does not version it.
    #[test]
    fn unknown_fields_in_the_reply_do_not_break_parsing() {
        let raw = r#"{"is_error":false,"result":"ok","brand_new_field":{"nested":true}}"#;
        let parsed: CliResult = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.result.as_deref(), Some("ok"));
    }

    /// Every one of these flags was measured to matter: without them a call
    /// costs 20775 prompt tokens instead of 452, which is the difference
    /// between $50 and $0.30 for one repository. Losing one silently would
    /// look like nothing at all until a bill arrived, so the set is asserted
    /// rather than trusted to review.
    #[test]
    fn the_harness_is_stripped_off_every_call() {
        let flags = stripped_harness();
        for expected in [
            "--tools",
            "--strict-mcp-config",
            "--mcp-config",
            "--disable-slash-commands",
            "--setting-sources",
            "--effort",
            "--model",
        ] {
            assert!(flags.contains(&expected), "{expected} is what keeps a call from carrying the whole agent");
        }
    }

    /// The two flags whose *value* decides the price, checked as values.
    ///
    /// Asserting the flag names alone is not enough, and that is not a
    /// hypothetical: every cost in this file was measured with `--model
    /// haiku`, the code went without it for a while, and the calls quietly
    /// ran on the user's default model at three times the measured price. A
    /// test that listed flag names stayed green throughout, because it was
    /// checking what had been written rather than what had been measured.
    #[test]
    fn the_model_and_the_effort_are_the_ones_that_were_measured() {
        let flags = stripped_harness();
        let value_after = |name: &str| flags.iter().position(|flag| *flag == name).and_then(|at| flags.get(at + 1)).copied();
        assert_eq!(value_after("--model"), Some("haiku"), "the measurements in this file were made on haiku");
        assert_eq!(
            value_after("--effort"),
            Some("low"),
            "`none` and `minimal` are refused with a warning and the expensive default is used instead"
        );
    }

    /// The system prompt is the only defence against the model describing code
    /// it was never shown, and that failure was seen in a real run.
    #[test]
    fn the_system_prompt_forbids_describing_what_was_not_shown() {
        assert!(SYSTEM_PROMPT.contains("surface"));
        assert!(SYSTEM_PROMPT.contains("never say what a function does internally"));
        assert!(SYSTEM_PROMPT.contains("how complete or exhaustive a list is"));
    }
}
