//! `nooma repo` — the repository index, on the command line.
//!
//! `--json` exists because `rigger` reads this before nooma speaks MCP, and a
//! CLI with machine output is the line's standard way for one product to ask
//! another a question. What `--json` prints is a contract: fields are added,
//! never renamed or dropped without a format bump.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::{Args, Subcommand};
use nooma_core::{FileIndex, History, RepoIndex, Store, Symbol, SymbolKind, Update, history, incremental, repo::Repo};

/// What is in a repository.
#[derive(Debug, Args)]
pub struct RepoArgs {
    #[command(subcommand)]
    command: RepoCommand,
}

#[derive(Debug, Subcommand)]
enum RepoCommand {
    /// Read the repository and store the index, reparsing only what changed
    Index(IndexArgs),
    /// List the symbols the index holds
    Symbols(SymbolsArgs),
    /// Show which files import which, within this repository
    Deps(ReadArgs),
    /// Describe what a module offers: its header, signatures and docs
    Summary(SummaryArgs),
    /// Search the commit messages: what a change was for
    History(HistoryArgs),
    /// Say what the stored index describes, without changing it
    Status(CommonArgs),
}

/// The arguments every subcommand takes.
#[derive(Debug, Args)]
struct CommonArgs {
    /// The repository to read; any path inside it will do
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Print machine-readable JSON instead of a report
    #[arg(long)]
    json: bool,
    /// Keep the index in this directory instead of the user's data directory
    #[arg(long, value_name = "DIR")]
    store: Option<PathBuf>,
}

/// The arguments a command that reads the index takes.
#[derive(Debug, Args)]
struct ReadArgs {
    #[command(flatten)]
    common: CommonArgs,
    // A reading command brings the index up to date first, because refreshing
    // costs only the files that changed and answering from a stale index costs
    // the caller a wrong answer. This flag is for the caller that wants what
    // is stored, exactly as stored - comparing two revisions, or reading an
    // index of a tree it cannot touch.
    /// Answer from the stored index without bringing it up to date
    #[arg(long)]
    no_refresh: bool,
}

#[derive(Debug, Args)]
struct IndexArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// Reparse every file, even ones whose contents have not changed
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
struct SymbolsArgs {
    #[command(flatten)]
    read: ReadArgs,
    /// Only symbols whose name contains this text, compared without case
    #[arg(long, value_name = "TEXT")]
    name: Option<String>,
    /// Only symbols of this kind
    #[arg(long, value_name = "KIND", value_parser = parse_kind)]
    kind: Option<SymbolKind>,
    // Named `--under` rather than `--path` because the repository itself is
    // the positional `PATH`: two arguments sharing a value name parse as one
    // given twice, and clap refuses the whole command.
    //
    // A plain comment, not a doc comment: clap reads a doc comment's body as
    // the long help, and one long help anywhere in a command strips the short
    // descriptions off every flag in it — `--help` then lists bare flag names.
    /// Only symbols in files whose path starts with this
    #[arg(long, value_name = "PREFIX")]
    under: Option<String>,
}

#[derive(Debug, Args)]
struct SummaryArgs {
    #[command(flatten)]
    read: ReadArgs,
    // A plain comment, not a doc comment: clap reads a doc comment's body as
    // the long help, and one long help anywhere in a command strips the short
    // descriptions off every flag in it.
    //
    // Named `--under` to match `symbols`, and for the same reason: the
    // repository itself is the positional `PATH`.
    /// Only modules whose path starts with this
    #[arg(long, value_name = "PREFIX")]
    under: Option<String>,
    /// Include declarations the language keeps private
    #[arg(long)]
    all: bool,
    // A plain comment, not a doc comment, for the reason given above.
    //
    // Present only in a build made with the `prose` feature. In a default
    // build the flag does not exist, so `--prose` is an unknown argument
    // rather than a flag that silently does nothing — the difference between
    // "this build cannot" and "this build chose not to".
    /// Add a generated paragraph saying what each module is for, using your
    /// own installed Claude Code
    #[cfg(feature = "prose")]
    #[arg(long)]
    prose: bool,
}

#[derive(Debug, Args)]
struct HistoryArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// Only commits whose message contains this text, compared without case
    #[arg(long, value_name = "TEXT")]
    matching: Option<String>,
    /// Only commits that touched this path, or anything beneath it
    #[arg(long, value_name = "PATH")]
    touching: Option<String>,
    /// How many commits to read from the top of the history
    #[arg(long, value_name = "N", default_value_t = DEFAULT_HISTORY_LIMIT)]
    limit: usize,
    /// Answer from the stored history without reading new commits
    #[arg(long)]
    no_refresh: bool,
}

/// How far back a history pass reads when the caller does not say.
///
/// Large enough to cover the whole history of every repository on this line,
/// and small enough that a first run on something the size of the Linux kernel
/// answers rather than appearing to hang.
const DEFAULT_HISTORY_LIMIT: usize = 5_000;

pub fn run(args: RepoArgs) -> Result<()> {
    match args.command {
        RepoCommand::Index(args) => index_command(args),
        RepoCommand::Symbols(args) => symbols_command(args),
        RepoCommand::Deps(args) => deps_command(args),
        RepoCommand::Summary(args) => summary_command(args),
        RepoCommand::History(args) => history_command(args),
        RepoCommand::Status(args) => status_command(args),
    }
}

/// Bring the index up to date and store it.
fn index_command(args: IndexArgs) -> Result<()> {
    let repo = open(&args.common)?;
    let store = open_store(&args.common)?;
    let (index, update) = refresh(&repo, &store, args.force)?;
    let path = store.path_for(repo.root());

    if args.common.json {
        print_json(&serde_json::json!({
            "root": index.root,
            "commit": index.revision.commit,
            "dirty": index.revision.dirty,
            "files": index.files.len(),
            "symbols": index.symbol_count(),
            "added": update.added,
            "changed": update.changed,
            "unchanged": update.unchanged,
            "removed": update.removed,
            "stored_at": path,
        }))
    } else {
        println!("{}: {} files, {} symbols", index.revision.short(), index.files.len(), index.symbol_count());
        println!("{}", describe(&update));
        println!("stored at {}", path.display());
        Ok(())
    }
}

/// List symbols, filtered.
fn symbols_command(args: SymbolsArgs) -> Result<()> {
    let index = read_index(&args.read)?;
    let name = args.name.map(|n| n.to_lowercase());
    let matched: Vec<(&FileIndex, &Symbol)> = index
        .files
        .iter()
        .filter(|file| args.under.as_ref().is_none_or(|prefix| file.path.starts_with(prefix)))
        .flat_map(|file| file.symbols.iter().map(move |symbol| (file, symbol)))
        .filter(|(_, symbol)| args.kind.is_none_or(|kind| symbol.kind == kind))
        .filter(|(_, symbol)| name.as_ref().is_none_or(|text| symbol.name.to_lowercase().contains(text)))
        .collect();

    if args.read.common.json {
        let rows: Vec<_> = matched
            .iter()
            .map(|(file, symbol)| {
                serde_json::json!({
                    "path": file.path,
                    "language": file.language,
                    "name": symbol.name,
                    "kind": symbol.kind.name(),
                    "line": symbol.line,
                    "parent": symbol.parent,
                })
            })
            .collect();
        print_json(&serde_json::json!({
            "commit": index.revision.commit,
            "dirty": index.revision.dirty,
            "symbols": rows,
        }))
    } else {
        for (file, symbol) in &matched {
            let qualified = match &symbol.parent {
                Some(parent) => format!("{parent}::{}", symbol.name),
                None => symbol.name.clone(),
            };
            println!("{}:{}  {}  {qualified}", file.path, symbol.line, symbol.kind);
        }
        eprintln!("{} symbols", matched.len());
        Ok(())
    }
}

/// Show the module dependency graph.
fn deps_command(args: ReadArgs) -> Result<()> {
    let index = read_index(&args)?;
    let graph = index.module_dependencies();
    if args.common.json {
        print_json(&serde_json::json!({
            "commit": index.revision.commit,
            "dirty": index.revision.dirty,
            "dependencies": graph,
        }))
    } else {
        for (file, imports) in &graph {
            println!("{file}");
            for import in imports {
                println!("  -> {import}");
            }
        }
        eprintln!("{} files with resolved imports", graph.len());
        Ok(())
    }
}

/// Describe what each module offers.
///
/// This is the command `rigger` reads to put one line per module in a packet:
/// the header sentence is what a module is *about*, which no list of symbol
/// names conveys.
fn summary_command(args: SummaryArgs) -> Result<()> {
    let index = read_index(&args.read)?;
    let modules: Vec<&FileIndex> = index
        .files
        .iter()
        .filter(|file| args.under.as_ref().is_none_or(|prefix| file.path.starts_with(prefix)))
        .collect();

    // Prose is asked for before anything is printed, so that a run which
    // cannot reach Claude Code fails before writing half a report — and so
    // that `--json` never emits a document with some modules described and
    // others not because the CLI stopped answering partway.
    let prose = prose_for(&args, &modules)?;

    if args.read.common.json {
        let rows: Vec<_> = modules
            .iter()
            .filter_map(|file| file.summary.as_ref())
            .map(|summary| {
                let entries: Vec<_> = summary
                    .entries
                    .iter()
                    .filter(|entry| args.all || entry.public)
                    .map(|entry| {
                        serde_json::json!({
                            "name": entry.name,
                            "kind": entry.kind.name(),
                            "line": entry.line,
                            "parent": entry.parent,
                            "signature": entry.signature,
                            "doc": entry.doc,
                            "public": entry.public,
                        })
                    })
                    .collect();
                let mut row = serde_json::json!({
                    "path": summary.path,
                    "language": summary.language,
                    "header": summary.header,
                    "public": summary.public_count(),
                    "text": summary.to_text(),
                    "entries": entries,
                });
                // A field of its own, never folded into `header` or `text`.
                // Those two are lifted from the source word for word, and a
                // consumer that cannot tell them from a generated paragraph
                // has lost the one distinction this product is built on.
                if let Some(paragraph) = prose.get(summary.path.as_str()) {
                    row["prose"] = serde_json::json!(paragraph);
                }
                row
            })
            .collect();
        print_json(&serde_json::json!({
            "commit": index.revision.commit,
            "dirty": index.revision.dirty,
            "modules": rows,
        }))
    } else {
        for file in &modules {
            let Some(summary) = &file.summary else { continue };
            println!("{}", summary.path);
            if let Some(header) = &summary.header {
                // The first line only: a header can run to a paragraph, and a
                // listing that prints all of it stops being a listing.
                println!("  {}", header.lines().next().unwrap_or_default());
            }
            // Marked, every time it is shown. The line above it was written
            // by the module's author and this one was not, and nothing but
            // the label says so.
            if let Some(paragraph) = prose.get(summary.path.as_str()) {
                println!("  [generated] {paragraph}");
            }
            for entry in summary.entries.iter().filter(|entry| args.all || entry.public) {
                // The line alone: the path is the heading directly above, and
                // repeating it on every row pushes the signatures — the part
                // worth reading — off to the right.
                println!("  {:>5}  {}", entry.line, entry.signature);
            }
        }
        eprintln!("{} modules", modules.len());
        Ok(())
    }
}

/// The prose to show beside each summary, keyed by module path.
///
/// Empty unless this build has the `prose` feature *and* the caller passed
/// `--prose`. Both conditions are deliberate: the feature decides whether the
/// code exists, the flag decides whether it runs, and neither defaults to yes.
#[cfg(feature = "prose")]
fn prose_for(args: &SummaryArgs, modules: &[&FileIndex]) -> Result<BTreeMap<String, String>> {
    use crate::prose;

    if !args.prose {
        return Ok(BTreeMap::new());
    }

    // Checked before the first call rather than discovered on it, so that
    // "you do not have Claude Code" is one clear sentence instead of a
    // process-spawn failure repeated once per module.
    let availability = prose::probe();
    if !availability.available {
        anyhow::bail!("{}", availability.reason.unwrap_or_else(|| prose::MISSING.to_owned()));
    }

    let store = open_store(&args.read.common)?;
    let cache = prose::Cache::open(store.root())?;
    // Said before the first call rather than after the last: a run over a
    // large repository takes minutes and spends money, and the user is
    // entitled to know what is about to answer and how much of the work is
    // already paid for while there is still time to stop it.
    eprintln!(
        "describing with {} ({} modules already in {})",
        availability.version.as_deref().unwrap_or("Claude Code"),
        cache.len(),
        cache.path().display()
    );

    let mut spend = prose::Spend::default();
    let mut described = BTreeMap::new();

    for file in modules {
        let Some(summary) = &file.summary else { continue };
        // Keyed by what the file hashes to, not by where it sits: the same
        // bytes vendored into a second repository are the same module.
        let prose = prose::describe(&cache, &summary.to_text(), &file.content_hash, &mut spend)?;
        described.insert(summary.path.clone(), prose);
    }

    // On stderr, so `--json` still redirects to a file that parses — and
    // never silent, because this is the one command in nooma that spends the
    // user's money.
    eprintln!("{}", spend.describe());
    Ok(described)
}

/// Without the feature there is nothing to ask and no flag to ask with.
#[cfg(not(feature = "prose"))]
fn prose_for(_args: &SummaryArgs, _modules: &[&FileIndex]) -> Result<BTreeMap<String, String>> {
    Ok(BTreeMap::new())
}

/// Search the commit messages.
fn history_command(args: HistoryArgs) -> Result<()> {
    let repo = open(&args.common)?;
    let store = open_store(&args.common)?;
    let history = read_history(&repo, &store, &args)?;

    // Both filters narrow the same list, so asking for a message *and* a path
    // means both, which is what someone naming two things expects.
    let mut commits: Vec<&nooma_core::CommitDoc> = match &args.matching {
        Some(text) => history.matching(text),
        None => history.commits.iter().collect(),
    };
    if let Some(path) = &args.touching {
        let touching = history.touching(path);
        commits.retain(|commit| touching.iter().any(|other| other.id == commit.id));
    }

    if args.common.json {
        let rows: Vec<_> = commits
            .iter()
            .map(|commit| {
                serde_json::json!({
                    "id": commit.id,
                    "summary": commit.summary,
                    "body": commit.body,
                    "author": commit.author,
                    "time": commit.time,
                    "paths": commit.paths,
                })
            })
            .collect();
        print_json(&serde_json::json!({
            "head": history.head,
            "commits": rows,
        }))
    } else {
        for commit in &commits {
            println!("{}  {}", commit.short(), commit.summary);
        }
        eprintln!("{} commits", commits.len());
        Ok(())
    }
}

/// The history a reading command should answer from.
///
/// Brought up to date first unless the caller asked otherwise, for the same
/// reason the index is: reading costs only the commits that are new, and
/// answering from a stale history costs the caller a wrong answer about their
/// own newest work.
fn read_history(repo: &Repo, store: &Store, args: &HistoryArgs) -> Result<History> {
    // A stored history that cannot be read is not a failure: reading from
    // scratch is exactly what fixes it.
    let previous = match history::load(store, repo.root()) {
        Ok(previous) => previous,
        Err(error) if error.is_stale_index() => None,
        Err(error) => return Err(error.into()),
    };

    if args.no_refresh {
        return match previous {
            Some(history) => Ok(history),
            None => anyhow::bail!("{} has no stored history — run `nooma repo history`", repo.root().display()),
        };
    }

    let (history, update) = history::read(repo, previous.as_ref(), args.limit).context("reading the history")?;
    if previous.as_ref() != Some(&history) {
        history::save(store, &history).context("storing the history")?;
    }
    // On stderr, so `--json` still redirects to a file that parses, and so
    // work done on the caller's behalf is never silent.
    if !update.is_noop() {
        eprintln!("read {} commits; kept {}", update.read, update.kept);
    }
    Ok(history)
}

/// Say what the stored index describes, without changing it.
///
/// The one command that never refreshes: it is the question `rigger` asks
/// before deciding whether to do anything, and a question that changes what it
/// asks about cannot be asked twice.
fn status_command(args: CommonArgs) -> Result<()> {
    let repo = open(&args)?;
    let store = open_store(&args)?;
    let stored = match store.load(repo.root()) {
        Ok(stored) => stored,
        // A stale format is not a failure to report — it is the same answer as
        // "not indexed", with a different reason. Both mean: run `index`.
        Err(error) if error.is_stale_index() => None,
        Err(error) => return Err(error.into()),
    };
    // Reusable and current are different questions. An index cut up by older
    // rules describes this commit perfectly well and still has to be rebuilt,
    // so a caller reading only `current` would skip the rebuild it needs.
    let reusable = stored.as_ref().is_some_and(RepoIndex::is_reusable);
    let current = stored
        .as_ref()
        .is_some_and(|index| index.is_reusable() && !index.revision.dirty && index.revision.commit == repo.commit());

    if args.json {
        print_json(&serde_json::json!({
            "root": repo.root(),
            "commit": repo.commit(),
            "indexed": stored.is_some(),
            "current": current,
            "reusable": reusable,
            "indexed_commit": stored.as_ref().map(|index| &index.revision.commit),
            "indexed_dirty": stored.as_ref().map(|index| index.revision.dirty),
            "files": stored.as_ref().map(|index| index.files.len()),
            "symbols": stored.as_ref().map(RepoIndex::symbol_count),
        }))
    } else {
        match &stored {
            None => println!("not indexed — run `nooma repo index`"),
            Some(index) if current => {
                println!(
                    "current at {}: {} files, {} symbols",
                    index.revision.short(),
                    index.files.len(),
                    index.symbol_count()
                );
            }
            Some(index) if !index.is_reusable() => {
                println!("indexed at {}, but by older rules — run `nooma repo index`", index.revision.short());
            }
            // A dirty index is honestly not current, but no command makes it
            // current: the tree has edits, and it will keep having them until
            // they are committed. Telling the caller to reindex here would
            // prescribe the command they just ran - the index is as fresh as
            // an index of a dirty tree can be.
            Some(index) if index.revision.commit == repo.commit() && index.revision.dirty => {
                println!(
                    "indexed at {} with uncommitted edits: {} files, {} symbols",
                    index.revision.short(),
                    index.files.len(),
                    index.symbol_count()
                );
            }
            Some(index) => {
                println!(
                    "stale: indexed at {}, checked out {} — run `nooma repo index`",
                    index.revision.short(),
                    &repo.commit()[..8]
                );
            }
        }
        Ok(())
    }
}

/// Bring the stored index up to date, reparsing only what changed.
fn refresh(repo: &Repo, store: &Store, force: bool) -> Result<(RepoIndex, Update)> {
    // A stored index that cannot be read is not a failure: this is the path
    // that fixes it, so it starts from nothing and rebuilds.
    let previous = if force { None } else { store.load(repo.root()).ok().flatten() };
    let (index, update) = incremental::update(repo, previous.as_ref()).context("indexing the work tree")?;
    // Writing an identical index would rewrite the same bytes for nothing and
    // touch the file's timestamp, which is the one thing a person looking at
    // the store has to go on.
    //
    // The whole index is compared, not the file counts: committing changes no
    // file's bytes and still changes what the index describes, from one commit
    // with edits to the next one without. Skipping on `update.is_noop()` alone
    // left the store claiming the old revision, and `status` then reported a
    // dirty tree that had just been committed.
    if previous.as_ref() == Some(&index) {
        return Ok((index, update));
    }
    store.save(&index).context("storing the index")?;
    Ok((index, update))
}

/// The index a reading command should answer from.
///
/// Brought up to date first unless the caller asked otherwise: refreshing
/// costs only the files that changed, and answering from a stale index costs
/// the caller a wrong answer about their own newest work.
fn read_index(args: &ReadArgs) -> Result<RepoIndex> {
    let repo = open(&args.common)?;
    let store = open_store(&args.common)?;

    if args.no_refresh {
        return match store.load(repo.root()) {
            Ok(Some(index)) => Ok(index),
            Ok(None) => anyhow::bail!("{} is not indexed — run `nooma repo index`", repo.root().display()),
            Err(error) if error.is_stale_index() => anyhow::bail!("{error} — run `nooma repo index`"),
            Err(error) => Err(error.into()),
        };
    }

    let (index, update) = refresh(&repo, &store, false)?;
    // On stderr, so that `--json` still redirects to a file that parses, and
    // so that work done on the caller's behalf is never silent.
    if !update.is_noop() {
        eprintln!("{}", describe(&update));
    }
    Ok(index)
}

/// What a pass did, in one line.
fn describe(update: &Update) -> String {
    if update.is_noop() {
        return format!("nothing changed; reused {} files", update.unchanged);
    }
    let mut parts = Vec::new();
    if update.added > 0 {
        parts.push(format!("{} added", update.added));
    }
    if update.changed > 0 {
        parts.push(format!("{} changed", update.changed));
    }
    if update.removed > 0 {
        parts.push(format!("{} removed", update.removed));
    }
    format!("parsed {} files ({}); reused {}", update.parsed(), parts.join(", "), update.unchanged)
}

fn open(args: &CommonArgs) -> Result<Repo> {
    Repo::discover(&args.path).with_context(|| format!("reading {}", args.path.display()))
}

fn open_store(args: &CommonArgs) -> Result<Store> {
    match &args.store {
        Some(dir) => Store::open_at(dir).with_context(|| format!("opening {}", dir.display())),
        None => Store::open().context("opening the index directory"),
    }
}

/// Write JSON to stdout as one line.
///
/// One line, so that a caller can read a result per line and `jq` does not
/// have to hold the whole document; and to stdout alone, with every human word
/// on stderr, so `nooma … --json > file` yields a file that parses.
fn print_json(value: &serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

fn parse_kind(text: &str) -> Result<SymbolKind, String> {
    match text {
        "function" => Ok(SymbolKind::Function),
        "type" => Ok(SymbolKind::Type),
        "module" => Ok(SymbolKind::Module),
        "constant" => Ok(SymbolKind::Constant),
        other => Err(format!("unknown kind `{other}` — one of: function, type, module, constant")),
    }
}
