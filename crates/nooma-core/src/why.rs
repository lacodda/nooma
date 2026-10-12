//! Why a document was found, when its words were not why.
//!
//! A row the words found shows the words, marked. A row only the meaning
//! found has nothing to mark - and when the question was asked in one
//! language and the document is written in the other, nothing in it looks
//! like the question at all. What it can show is the place that answered:
//! the sentence of the chunk closest in meaning to the question, compared as
//! the index compares - without the language. The fragment starts at that
//! sentence, and [`crate::Hit::closest`] says where it ends.
//!
//! Finding it costs the model a read of every sentence of the chunk - up to
//! a quarter of a second for a long one - so it is done for the rows that have
//! nothing else to show: the ones the meaning alone found in the other
//! language ([`crate::Ranked::across`]). Asked of every row the meaning found,
//! it made a search on the command line take seconds longer; a row in the
//! question's own language shows the opening of its chunk, as before. The
//! window asks row by row after the list is shown, the command line for the
//! rows it prints.

use std::ops::Range;

use crate::library::FRAGMENT_CHARS;

/// The most sentences of one chunk read to find the closest: one batch of
/// the runtime. A chunk is at most a screen of text, which this covers.
pub const MOST_SENTENCES: usize = 32;

/// The sentences of a text, as byte ranges, in order: a text is cut at each
/// line break and after `.`, `!`, `?` or `…` followed by a space. A piece
/// with no letter or digit in it - a rule, a fence - is not a sentence.
pub fn sentences(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut push = |range: Range<usize>| {
        let piece = &text[range.clone()];
        let start = range.start + (piece.len() - piece.trim_start().len());
        let end = range.end - (piece.len() - piece.trim_end().len());
        if start < end && text[start..end].chars().any(char::is_alphanumeric) {
            out.push(start..end);
        }
    };
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let mut start = line_start;
        let mut chars = line.char_indices().peekable();
        while let Some((at, c)) = chars.next() {
            if matches!(c, '.' | '!' | '?' | '…') && chars.peek().is_none_or(|(_, next)| next.is_whitespace()) {
                let end = line_start + at + c.len_utf8();
                push(start..end);
                start = end;
            }
        }
        push(start..line_start + line.len());
        line_start += line.len();
    }
    out
}

/// The fragment that shows one sentence of a text: from the sentence on, as
/// long as a fragment is, with `…` where text was left out on either side;
/// and where the sentence is in it.
pub fn window(text: &str, sentence: Range<usize>) -> (String, Range<usize>) {
    let mut fragment = String::new();
    if !text[..sentence.start].trim().is_empty() {
        fragment.push('…');
    }
    let starts = fragment.len();
    let rest = &text[sentence.start..];
    let cut = rest.char_indices().nth(FRAGMENT_CHARS).map(|(end, _)| end);
    match cut {
        Some(end) => {
            fragment.push_str(rest[..end].trim_end());
            fragment.push('…');
        }
        None => fragment.push_str(rest.trim_end()),
    }
    let shown = cut.unwrap_or(rest.len()).min(sentence.len());
    let shown = rest[..shown].trim_end().len();
    (fragment, starts..starts + shown)
}

/// Whether a row needs to be told why it was found: the meaning alone found
/// it in another language, and it has not been told yet.
pub fn wants_why(ranked: &crate::Ranked) -> bool {
    ranked.across && ranked.hit.highlights.is_empty() && ranked.hit.closest.is_none()
}

/// Point a hit at the sentence of its chunk closest to `query` - a vector as
/// the model made it, for a question or a passage: the fragment becomes that
/// sentence and what follows, and [`crate::Hit::closest`] marks it. Returns whether
/// it did; a chunk the index no longer holds is left as it was.
#[cfg(feature = "semantic")]
pub fn explain(
    finder: &crate::Finder,
    index: &crate::SemanticIndex,
    embedder: &mut dyn crate::Embedder,
    query: &[f32],
    hit: &mut crate::Hit,
) -> crate::Result<bool> {
    let Some(text) = finder.chunk_text(&hit.path, hit.line)? else {
        return Ok(false);
    };
    let found: Vec<Range<usize>> = sentences(&text).into_iter().take(MOST_SENTENCES).collect();
    let best = match found.len() {
        0 => return Ok(false),
        // One sentence is the answer without asking the model.
        1 => found[0].clone(),
        _ => {
            let texts: Vec<&str> = found.iter().map(|range| &text[range.clone()]).collect();
            let vectors = embedder.embed(&texts, crate::Role::Passage)?;
            let close = index.closeness(query, &vectors);
            let at = close
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1).then(b.0.cmp(&a.0)))
                .map_or(0, |(at, _)| at);
            found[at].clone()
        }
    };
    let (fragment, closest) = window(&text, best);
    hit.fragment = fragment;
    hit.closest = Some(closest);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pieces(text: &str) -> Vec<&str> {
        sentences(text).into_iter().map(|range| &text[range]).collect()
    }

    #[test]
    fn a_text_is_cut_into_sentences_at_stops_and_lines() {
        assert_eq!(
            pieces("Descale it monthly. Use citric acid!\n- Rinse twice\n---\nКипятить 10 мин… Затем слить."),
            ["Descale it monthly.", "Use citric acid!", "- Rinse twice", "Кипятить 10 мин…", "Затем слить."]
        );
        assert_eq!(pieces("version 1.2.3 is out"), ["version 1.2.3 is out"], "a stop inside a word is not an end");
        assert!(pieces("  \n```\n").is_empty());
    }

    #[test]
    fn a_window_starts_at_its_sentence_and_marks_it() {
        let text = "First one. Второе предложение здесь. Third.";
        let second = sentences(text)[1].clone();
        let (fragment, at) = window(text, second);
        assert_eq!(fragment, "…Второе предложение здесь. Third.");
        assert_eq!(&fragment[at], "Второе предложение здесь.");

        let (fragment, at) = window(text, sentences(text)[0].clone());
        assert!(fragment.starts_with("First one."), "nothing left out before the first: {fragment}");
        assert_eq!(&fragment[at], "First one.");
    }

    /// A sentence longer than a fragment is shown as far as it fits, and the
    /// mark ends where the fragment does.
    #[test]
    fn a_long_sentence_is_cut_where_the_fragment_ends() {
        let long = "слово ".repeat(80);
        let text = format!("Intro. {long}end.");
        let sentence = sentences(&text)[1].clone();
        let (fragment, at) = window(&text, sentence);
        assert!(fragment.starts_with('…') && fragment.ends_with('…'), "{fragment}");
        assert_eq!(fragment[at.clone()].chars().count(), FRAGMENT_CHARS - 1, "trailing space trimmed");
        assert!(fragment.is_char_boundary(at.end));
    }
}
