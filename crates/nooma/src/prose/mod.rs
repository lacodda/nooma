//! Prose summaries: what a module is for, said in sentences.
//!
//! A summary lifted from source (`nooma repo summary`) is trustworthy because
//! every word in it was written by the module's author. That is also its
//! ceiling: a module whose author wrote no documentation summarizes as a list
//! of signatures, and a list of signatures does not say what the module is
//! *for* — the question the whole product exists to answer.
//!
//! This answers it by asking the Claude Code CLI the user already installed.
//! Why that rather than a key in a config file, and why none of it lives in
//! `nooma-core`, is [ADR 0004]. The short version: the library other products
//! link has no code that starts a process and gains none; this module is
//! behind a Cargo feature that is off by default; and even in a build that has
//! it, nothing happens without `--prose` on the command line.
//!
//! [ADR 0004]: https://github.com/lacodda/nooma/blob/main/docs/adr/0004-prose-summaries-shell-out.md
//!
//! # What is generated and what is not
//!
//! Everything here is generated, and it is kept apart from everything that is
//! not. Prose lives in its own store, never in the index; it is labelled
//! wherever it is printed; and it never replaces a header the author wrote.
//! The promise is not that nothing is ever generated — it is that generated
//! text is visible as generated, and that the part lifted from the source is
//! still there beside it, unchanged.

mod cache;
mod claude;

pub use cache::Cache;
pub use claude::{MISSING, probe};

use anyhow::Result;

/// What a run of prose cost, for the caller to report.
#[derive(Debug, Clone, Copy, Default)]
pub struct Spend {
    /// Modules answered from the cache.
    pub cached: usize,
    /// Modules that needed a call.
    pub asked: usize,
    /// What those calls cost, as the CLI reported it.
    ///
    /// `None` from the CLI is counted as zero rather than refused: a missing
    /// cost field is not a reason to fail a summary that was produced.
    pub cost_usd: f64,
}

impl Spend {
    /// Whether anything was paid for.
    pub fn is_free(&self) -> bool {
        self.asked == 0
    }

    /// What happened, in one line for a person.
    pub fn describe(&self) -> String {
        if self.is_free() {
            return format!("{} modules described from the cache", self.cached);
        }
        format!(
            "described {} modules ({} from the cache); ${:.4}",
            self.asked + self.cached,
            self.cached,
            self.cost_usd
        )
    }
}

/// Produce prose for one module, from the cache when it is there.
///
/// Keyed by the content hash of the file the summary came from, never by its
/// path: the same bytes in two repositories are the same module and deserve
/// one answer rather than two charges.
pub fn describe(cache: &Cache, summary_text: &str, content_hash: &str, spend: &mut Spend) -> Result<String> {
    if let Some(text) = cache.get(content_hash) {
        spend.cached += 1;
        return Ok(text);
    }

    let turn = claude::describe(summary_text)?;
    // Written to the cache before it is returned, not after the run: a crash
    // partway through a repository must leave every paragraph already bought
    // still bought.
    cache.put(content_hash, &turn.text)?;
    spend.asked += 1;
    spend.cost_usd += turn.cost_usd.unwrap_or_default();
    Ok(turn.text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cached module must not reach the CLI. If it did, the cache would be
    /// decoration and every second run would be charged again — so this asks
    /// for prose the cache holds and proves nothing was spent.
    ///
    /// It also needs no CLI to run, which is what makes it a test rather than
    /// something only the author can check.
    #[test]
    fn a_cached_module_costs_nothing_and_never_reaches_the_cli() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path()).unwrap();
        cache.put("hash-of-the-file", "What it is for.").unwrap();

        let mut spend = Spend::default();
        let prose = describe(&cache, "ignored, because the cache answers first", "hash-of-the-file", &mut spend).unwrap();

        assert_eq!(prose, "What it is for.");
        assert_eq!(spend.asked, 0, "a cached module must never reach the CLI");
        assert_eq!(spend.cached, 1);
        assert_eq!(spend.cost_usd, 0.0);
        assert!(spend.is_free());
    }

    #[test]
    fn a_free_run_says_so_without_a_price() {
        let spend = Spend {
            cached: 12,
            asked: 0,
            cost_usd: 0.0,
        };
        assert_eq!(spend.describe(), "12 modules described from the cache");
    }

    #[test]
    fn a_paid_run_reports_what_it_cost() {
        let spend = Spend {
            cached: 3,
            asked: 2,
            cost_usd: 0.0134,
        };
        assert_eq!(spend.describe(), "described 5 modules (3 from the cache); $0.0134");
    }
}
