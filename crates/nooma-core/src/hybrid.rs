//! The two halves as one list.
//!
//! Neither half is the product alone. The words find the invoice by its
//! number and miss the note written in other words; the meaning finds that
//! note and blurs the number. So each is asked, and what they found is ranked
//! together: one list, one document per row, best first.
//!
//! # By rank, not by score
//!
//! A full-text score and a cosine are not on one scale - the first grows with
//! the length of the query and the rarity of its words, the second sits in a
//! narrow band that differs from model to model. What both halves do agree on
//! is order. A document earns from each half that found it a share that falls
//! with its place in that half's list - reciprocal rank fusion - and the list
//! is ordered by the sum. A document both halves found high is the strongest
//! answer there is.
//!
//! # A word in common is not an answer
//!
//! When no document holds every word of a question, the full-text half takes
//! documents holding any of them, and for a question asked in words the
//! answer does not use, those are the documents that share a word with the
//! question and nothing else - a tower crane for a dripping tap, *кран* both.
//! Ranked like the rest, they push the document the meaning found down and
//! out of sight. So the words' share of a document is weighed by how much of
//! the question its chunk holds, squared: all of it counts fully, half of it a
//! quarter, one word in six next to nothing. Measured on both query sets, this
//! is what kept the answer among the first ten where the questions cross
//! languages, and it cost nothing where they do not.
//!
//! # Fresh, a little
//!
//! A note edited this week is a little more likely to be the one meant than
//! one untouched for a year, and only a little: a newer note wins a near tie,
//! never against a document that answers better. The weight falls by half
//! every thirty days.
//!
//! The numbers are measured, not chosen: `nooma eval` over both query sets, a
//! flat region of settings that score alike, and the middle of it. There is no
//! dial for the balance between words and meaning - a dial would hand the
//! measurement to the person searching.

use std::collections::HashMap;

use serde::Serialize;

#[cfg(feature = "semantic")]
use crate::library::unix_now;
use crate::library::{Hit, Matched};

/// Where in a list a document stops counting for much: the `k` of
/// reciprocal rank fusion. Small, so the first places of each half matter far
/// more than the tenth.
const K: f32 = 8.0;

/// How much a place in the words' list weighs against the same place in the
/// meaning's, for a chunk that holds the whole question.
const WORDS: f32 = 1.5;

/// How much more a document edited just now weighs than an old one.
const FRESH: f64 = 0.1;

/// Days over which the weight of freshness halves.
const HALF_LIFE_DAYS: f64 = 30.0;

/// Documents asked of each half at the least: a document the words put
/// twentieth and the meaning third belongs near the top, and is found only
/// when both lists reach that far. Thirty measured the same as fifty.
pub const CANDIDATES: usize = 30;

/// How many documents to ask each half for, to rank `limit` of them.
pub fn candidates(limit: usize) -> usize {
    limit.max(CANDIDATES)
}

/// Where a document stood in one half's list.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Place {
    /// Its rank there, from 1.
    pub rank: usize,
    /// That half's own score: a full-text score, comparable only within one
    /// search; or a cosine.
    pub score: f32,
    /// For the words: the share of the query's distinctive words its chunk
    /// holds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share: Option<f32>,
}

/// A document in the one list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Ranked {
    /// The chunk shown for it - from the half that ranked it higher - with
    /// the fused score: comparable within one search, not across two.
    #[serde(flatten)]
    pub hit: Hit,
    /// Where the words put it, if they found it.
    pub words: Option<Place>,
    /// Where the meaning put it, if it found it.
    pub meaning: Option<Place>,
}

/// What a search answers.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Answer {
    /// The documents, best first.
    pub hits: Vec<Ranked>,
    /// Whether the words' hits hold every word of the query; see
    /// [`Matched::all_words`].
    pub all_words: bool,
}

/// One document while the halves are summed.
struct Entry {
    words: Option<(Place, Hit)>,
    meaning: Option<(Place, Hit)>,
    from_words: f32,
    from_meaning: f32,
}

/// Rank what the two halves found as one list of at most `limit`.
///
/// `meaning` is the meaning's hits, closest first, scored by cosine; empty
/// when there is no model or no vectors, and the words alone answer. `now` is
/// the time freshness is measured from, in seconds since the epoch.
pub fn fuse(words: Matched, meaning: Vec<Hit>, now: i64, limit: usize) -> Answer {
    let mut entries: HashMap<std::path::PathBuf, Entry> = HashMap::new();
    let blank = || Entry {
        words: None,
        meaning: None,
        from_words: 0.0,
        from_meaning: 0.0,
    };
    for (i, found) in words.hits.into_iter().enumerate() {
        let place = Place {
            rank: i + 1,
            score: found.hit.score,
            share: Some(found.share),
        };
        let entry = entries.entry(found.hit.path.clone()).or_insert_with(blank);
        entry.from_words = WORDS * found.share * found.share / (K + place.rank as f32);
        entry.words = Some((place, found.hit));
    }
    for (i, hit) in meaning.into_iter().enumerate() {
        let place = Place {
            rank: i + 1,
            score: hit.score,
            share: None,
        };
        let entry = entries.entry(hit.path.clone()).or_insert_with(blank);
        entry.from_meaning = 1.0 / (K + place.rank as f32);
        entry.meaning = Some((place, hit));
    }

    let mut ranked: Vec<Ranked> = entries
        .into_values()
        .map(|entry| {
            let total = entry.from_words + entry.from_meaning;
            let words = entry.words.as_ref().map(|(place, _)| *place);
            let meaning = entry.meaning.as_ref().map(|(place, _)| *place);
            // The half that carried the document shows it: its chunk is the
            // one that answered, and the words' carries highlights.
            let shown = if entry.from_words >= entry.from_meaning {
                entry.words.or(entry.meaning)
            } else {
                entry.meaning.or(entry.words)
            };
            let (_, mut hit) = shown.expect("every entry was found by one half");
            hit.score = total * freshness(hit.modified, now);
            Ranked { hit, words, meaning }
        })
        .collect();
    // Ties go to the path, so two runs over the same library agree.
    ranked.sort_by(|a, b| b.hit.score.total_cmp(&a.hit.score).then_with(|| a.hit.path.cmp(&b.hit.path)));
    ranked.truncate(limit);
    Answer {
        hits: ranked,
        all_words: words.all_words,
    }
}

/// The weight of being recent: one and a tenth for a file edited now, falling
/// by half of the tenth every thirty days. A file stamped in the future - a
/// clock set wrong - counts as edited now.
fn freshness(modified: i64, now: i64) -> f32 {
    let days = (now - modified).max(0) as f64 / 86_400.0;
    (1.0 + FRESH * (-days / HALF_LIFE_DAYS).exp2()) as f32
}

/// Ask both halves and rank what they found as one list.
///
/// `meaning` is the vector index and the query as a vector, when there are
/// both; without them the words answer alone.
#[cfg(feature = "semantic")]
pub fn search(finder: &crate::Finder, meaning: Option<(&crate::SemanticIndex, &[f32])>, query: &str, limit: usize) -> crate::Result<Answer> {
    let wanted = candidates(limit);
    let words = finder.search(query, wanted)?;
    let close = match meaning {
        Some((index, vector)) => index.hits(finder, vector, wanted, None)?,
        None => Vec::new(),
    };
    Ok(fuse(words, close, unix_now(), limit))
}

/// The documents that say what a passage says, ranked as [`search`] ranks:
/// only the meaning answers - a paragraph's words are not a query - and
/// freshness weighs as it does there. `skip` leaves out the file the passage
/// came from.
#[cfg(feature = "semantic")]
pub fn similar(finder: &crate::Finder, index: &crate::SemanticIndex, vector: &[f32], limit: usize, skip: Option<&std::path::Path>) -> crate::Result<Answer> {
    let close = index.hits(finder, vector, candidates(limit), skip)?;
    Ok(fuse(Matched::default(), close, unix_now(), limit))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::library::WordHit;

    const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;

    fn hit(name: &str, score: f32, modified: i64) -> Hit {
        Hit {
            path: PathBuf::from(format!("/notes/{name}.md")),
            kind: "markdown".to_string(),
            title: name.to_string(),
            headings: Vec::new(),
            line: 1,
            fragment: format!("{name} fragment"),
            highlights: Vec::new(),
            tags: Vec::new(),
            modified,
            score,
        }
    }

    /// Notes edited a year ago: old enough that freshness says nothing.
    fn old(name: &str) -> Hit {
        hit(name, 1.0, NOW - 365 * DAY)
    }

    fn words(hits: &[(Hit, f32)], all_words: bool) -> Matched {
        Matched {
            hits: hits
                .iter()
                .map(|(hit, share)| WordHit {
                    hit: hit.clone(),
                    share: *share,
                })
                .collect(),
            all_words,
        }
    }

    fn order(answer: &Answer) -> Vec<&str> {
        answer.hits.iter().map(|ranked| ranked.hit.title.as_str()).collect()
    }

    #[test]
    fn a_document_both_halves_found_comes_first_once() {
        let answer = fuse(
            words(&[(old("receipt"), 1.0), (old("both"), 1.0)], true),
            vec![old("note"), old("both")],
            NOW,
            10,
        );
        assert_eq!(order(&answer)[0], "both");
        assert_eq!(answer.hits.len(), 3, "one row per document");
        let both = &answer.hits[0];
        assert_eq!(both.words.map(|p| p.rank), Some(2));
        assert_eq!(both.meaning.map(|p| p.rank), Some(2));
        assert!(answer.all_words);
    }

    /// The tower crane for the dripping tap: no note holds the whole
    /// question, the words' first is a note that shares one word of six with
    /// it, and the meaning's first is the answer. The answer leads.
    #[test]
    fn a_word_in_common_counts_for_little_against_the_meaning() {
        let answer = fuse(
            words(&[(old("crane"), 1.0 / 6.0), (old("tap"), 2.0 / 6.0)], false),
            vec![old("tap"), old("crane")],
            NOW,
            10,
        );
        assert_eq!(order(&answer), ["tap", "crane"]);
        assert!(!answer.all_words);
    }

    /// And a chunk that holds the whole question - a code, a name - wins
    /// over what the meaning merely put first.
    #[test]
    fn a_chunk_holding_every_word_leads_the_meaning_s_first() {
        // Named to sort first, so a tie would put it first.
        let answer = fuse(words(&[(old("invoice"), 1.0)], true), vec![old("agenda")], NOW, 10);
        assert_eq!(order(&answer), ["invoice", "agenda"]);
    }

    /// Two documents found as well as each other, crossed: the one edited
    /// this week goes first, the year-old one second.
    #[test]
    fn a_fresh_note_wins_a_near_tie() {
        let fresh = hit("fresh", 1.0, NOW - 2 * DAY);
        let answer = fuse(words(&[(old("stale"), 1.0), (fresh.clone(), 1.0)], true), vec![fresh, old("stale")], NOW, 10);
        assert_eq!(order(&answer), ["fresh", "stale"]);
    }

    /// And never more than a near tie: a note edited a minute ago that only
    /// the meaning found, fifth, passes the fourth and no more.
    #[test]
    fn freshness_does_not_outweigh_an_answer() {
        let answer = fuse(
            words(&[(old("answer"), 1.0)], true),
            vec![old("answer"), old("b"), old("c"), old("d"), hit("new", 1.0, NOW - 60)],
            NOW,
            10,
        );
        assert_eq!(order(&answer), ["answer", "b", "c", "new", "d"]);
    }

    /// The chunk shown is the one the stronger half found: the words' with
    /// its highlights when they carried the document, the meaning's when it
    /// did.
    #[test]
    fn the_half_that_carried_a_document_shows_it() {
        let mut by_words = old("doc");
        by_words.line = 3;
        by_words.highlights.push(0..3);
        let mut by_meaning = old("doc");
        by_meaning.line = 40;

        let carried_by_words = fuse(words(&[(by_words.clone(), 1.0)], true), vec![old("x"), by_meaning.clone()], NOW, 10);
        let doc = carried_by_words.hits.iter().find(|r| r.hit.title == "doc").unwrap();
        assert_eq!((doc.hit.line, doc.hit.highlights.len()), (3, 1));

        let carried_by_meaning = fuse(words(&[(old("y"), 1.0), (old("z"), 1.0), (by_words, 0.5)], false), vec![by_meaning], NOW, 10);
        let doc = carried_by_meaning.hits.iter().find(|r| r.hit.title == "doc").unwrap();
        assert_eq!(doc.hit.line, 40);
    }

    #[test]
    fn the_limit_holds_and_ties_go_to_the_path() {
        let answer = fuse(Matched::default(), vec![old("b"), old("a")], NOW, 1);
        assert_eq!(order(&answer), ["b"], "different ranks are not a tie");
        // Chunks holding none of the distinctive words weigh nothing alike.
        let tied = fuse(words(&[(old("b"), 0.0), (old("a"), 0.0), (old("c"), 0.0)], false), Vec::new(), NOW, 10);
        assert_eq!(order(&tied), ["a", "b", "c"]);
    }

    #[test]
    fn freshness_halves_over_its_half_life_and_never_exceeds_a_tenth() {
        assert!((freshness(NOW, NOW) - 1.1).abs() < 1e-6);
        assert!((freshness(NOW - 30 * DAY, NOW) - 1.05).abs() < 1e-6);
        assert!((freshness(NOW + DAY, NOW) - 1.1).abs() < 1e-6, "a clock set wrong is not a bonus");
        assert!(freshness(0, NOW) < 1.0001);
    }
}
