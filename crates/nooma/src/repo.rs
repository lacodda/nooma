//! `nooma repo` — the repository index, on the command line.
//!
//! `--json` exists because `rigger` reads this before nooma speaks MCP, and a
//! CLI with machine output is the line's standard way for one product to ask
//! another a question. What `--json` prints is a contract: fields are added,
//! never renamed or dropped without a format bump.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use clap::{Args, Subcommand};
use nooma_core::{FileIndex, RepoIndex, Store, Symbol, SymbolKind, index, repo::Repo, symbols};

/// What is in a repository.
#[derive(Debug, Args)]
pub struct RepoArgs {
    #[command(subcommand)]
    command: RepoCommand,
}

#[derive(Debug, Subcommand)]
enum RepoCommand {
    /// Read the repository at the current commit and store the index
    Index(IndexArgs),
    /// List the symbols the stored index holds
    Symbols(SymbolsArgs),
    /// Show which files import which, within this repository
    Deps(CommonArgs),
    /// Say whether the stored index is current for the checked-out commit
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

#[derive(Debug, Args)]
struct IndexArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// Rebuild even if the stored index already describes this commit
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
struct SymbolsArgs {
    #[command(flatten)]
    common: CommonArgs,
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

pub fn run(args: RepoArgs) -> Result<()> {
    match args.command {
        RepoCommand::Index(args) => index_command(args),
        RepoCommand::Symbols(args) => symbols_command(args),
        RepoCommand::Deps(args) => deps_command(args),
        RepoCommand::Status(args) => status_command(args),
    }
}

/// Build the index and store it.
fn index_command(args: IndexArgs) -> Result<()> {
    let repo = Repo::discover(&args.common.path).with_context(|| format!("reading {}", args.common.path.display()))?;
    let store = open_store(&args.common)?;

    // Already current: the whole point of storing it. Say so rather than doing
    // the work again in silence, so a caller scripting this can tell the two
    // apart.
    // A stored index that cannot be read is not a failure here: `index` is the
    // command that fixes that, so it goes on and rebuilds.
    if !args.force
        && let Ok(Some(stored)) = store.load(repo.root())
        && stored.commit == repo.commit()
    {
        let path = store.path_for(repo.root());
        return report_index(&args.common, &stored, false, &path);
    }

    let files = repo.source_files().context("walking the work tree")?;
    let indexed = symbols::index_files(&files);
    let index = RepoIndex {
        format_version: index::FORMAT_VERSION,
        commit: repo.commit().to_string(),
        root: repo.root().to_path_buf(),
        files: indexed,
    };
    let path = store.save(&index).context("storing the index")?;
    report_index(&args.common, &index, true, &path)
}

/// List symbols, filtered.
fn symbols_command(args: SymbolsArgs) -> Result<()> {
    let index = load_current(&args.common)?;
    let name = args.name.map(|n| n.to_lowercase());
    let matched: Vec<(&FileIndex, &Symbol)> = index
        .files
        .iter()
        .filter(|file| args.under.as_ref().is_none_or(|prefix| file.path.starts_with(prefix)))
        .flat_map(|file| file.symbols.iter().map(move |symbol| (file, symbol)))
        .filter(|(_, symbol)| args.kind.is_none_or(|kind| symbol.kind == kind))
        .filter(|(_, symbol)| name.as_ref().is_none_or(|text| symbol.name.to_lowercase().contains(text)))
        .collect();

    if args.common.json {
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
        print_json(&serde_json::json!({ "commit": index.commit, "symbols": rows }))
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
fn deps_command(args: CommonArgs) -> Result<()> {
    let index = load_current(&args)?;
    let graph = index.module_dependencies();
    if args.json {
        print_json(&serde_json::json!({ "commit": index.commit, "dependencies": graph }))
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

/// Say whether the stored index describes the checked-out commit.
///
/// The question `rigger` asks before deciding whether to trust what it holds.
fn status_command(args: CommonArgs) -> Result<()> {
    let repo = Repo::discover(&args.path).with_context(|| format!("reading {}", args.path.display()))?;
    let store = open_store(&args)?;
    let stored = match store.load(repo.root()) {
        Ok(stored) => stored,
        // A stale format is not a failure to report — it is the same answer as
        // "not indexed", with a different reason. Both mean: run `index`.
        Err(error) if error.is_stale_index() => None,
        Err(error) => return Err(error.into()),
    };
    let current = stored.as_ref().is_some_and(|index| index.commit == repo.commit());

    if args.json {
        print_json(&serde_json::json!({
            "root": repo.root(),
            "commit": repo.commit(),
            "indexed": stored.is_some(),
            "current": current,
            "indexed_commit": stored.as_ref().map(|index| &index.commit),
            "files": stored.as_ref().map(|index| index.files.len()),
            "symbols": stored.as_ref().map(RepoIndex::symbol_count),
        }))
    } else {
        match (&stored, current) {
            (None, _) => println!("not indexed — run `nooma repo index`"),
            (Some(index), true) => println!(
                "current at {}: {} files, {} symbols",
                short(&index.commit),
                index.files.len(),
                index.symbol_count()
            ),
            (Some(index), false) => println!(
                "stale: indexed at {}, checked out {} — run `nooma repo index`",
                short(&index.commit),
                short(repo.commit())
            ),
        }
        Ok(())
    }
}

/// Load the index, insisting it describes the checked-out commit.
///
/// Reading a stale index and saying nothing would answer the user's question
/// with the previous commit's truth, which is the kind of quiet wrongness that
/// takes an afternoon to notice.
fn load_current(args: &CommonArgs) -> Result<RepoIndex> {
    let repo = Repo::discover(&args.path).with_context(|| format!("reading {}", args.path.display()))?;
    let store = open_store(args)?;
    match store.load(repo.root()) {
        Ok(Some(index)) if index.commit == repo.commit() => Ok(index),
        Ok(Some(index)) => anyhow::bail!(
            "the stored index is for {}, but {} is checked out — run `nooma repo index`",
            short(&index.commit),
            short(repo.commit())
        ),
        Ok(None) => {
            anyhow::bail!("{} is not indexed — run `nooma repo index`", repo.root().display())
        }
        Err(error) if error.is_stale_index() => anyhow::bail!("{error} — run `nooma repo index`"),
        Err(error) => Err(error.into()),
    }
}

fn open_store(args: &CommonArgs) -> Result<Store> {
    match &args.store {
        Some(dir) => Store::open_at(dir).with_context(|| format!("opening {}", dir.display())),
        None => Store::open().context("opening the index directory"),
    }
}

fn report_index(args: &CommonArgs, index: &RepoIndex, rebuilt: bool, path: &Path) -> Result<()> {
    if args.json {
        print_json(&serde_json::json!({
            "root": index.root,
            "commit": index.commit,
            "files": index.files.len(),
            "symbols": index.symbol_count(),
            "rebuilt": rebuilt,
            "stored_at": path,
        }))
    } else {
        let what = if rebuilt { "indexed" } else { "already current at" };
        println!("{what} {}: {} files, {} symbols", short(&index.commit), index.files.len(), index.symbol_count());
        println!("stored at {}", path.display());
        Ok(())
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

/// A commit hash as people quote it.
fn short(commit: &str) -> &str {
    &commit[..commit.len().min(8)]
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
