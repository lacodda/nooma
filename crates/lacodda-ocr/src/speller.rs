//! Which words a reading got right, as far as the languages' own spell
//! checkers can tell.
//!
//! The recognizers say nothing about how sure they are. Each reads every
//! picture, in its own alphabet, and a recognizer for the wrong language
//! returns its best guess: the English one reads Russian as `rapaHTHiiHblii`,
//! the Russian one reads `on` as the Cyrillic `оп`, and the English one
//! reads Spanish `mañanas` as `mafianas` - every one of them a word in the
//! right alphabet. What tells the readings apart is whether their words are
//! words: Windows has a spell checker for every language it has a
//! recognizer for, with the same dictionaries it underlines typing with.

use std::sync::OnceLock;

use windows::Win32::Globalization::{CORRECTIVE_ACTION_DELETE, ISpellChecker, ISpellCheckerFactory, SpellCheckerFactory};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoIncrementMTAUsage};
use windows::core::HSTRING;

/// The spell checkers for some languages, for one thread.
pub(crate) struct Spellers(Vec<ISpellChecker>);

/// Keep a multithreaded apartment open for the process, so the spell
/// checker factory, a COM object, can be made on any thread.
fn apartment() -> bool {
    static OPEN: OnceLock<bool> = OnceLock::new();
    // SAFETY: CoIncrementMTAUsage takes no pointers we own; the cookie it
    // returns is kept for the life of the process, never released.
    *OPEN.get_or_init(|| unsafe { CoIncrementMTAUsage() }.is_ok())
}

impl Spellers {
    /// The spell checkers for these languages - all of them, or none.
    ///
    /// A language without one would lose to every language with one: its
    /// right reading would have no known words, and another language's wrong
    /// reading of the same picture a few. With none, a reading is judged by
    /// its alphabet alone, which is fair to every language.
    pub(crate) fn for_languages(tags: &[String]) -> Self {
        if !apartment() {
            return Self(Vec::new());
        }
        // SAFETY: a COM object created in the process's apartment, used on
        // this thread only.
        let factory: Option<ISpellCheckerFactory> = unsafe { CoCreateInstance(&SpellCheckerFactory, None, CLSCTX_INPROC_SERVER) }.ok();
        let Some(factory) = factory else { return Self(Vec::new()) };
        let checkers: Option<Vec<ISpellChecker>> = tags
            .iter()
            .map(|tag| {
                let tag = HSTRING::from(tag.as_str());
                // SAFETY: as above.
                unsafe {
                    if !factory.IsSupported(&tag).is_ok_and(|supported| supported.as_bool()) {
                        return None;
                    }
                    factory.CreateSpellChecker(&tag).ok()
                }
            })
            .collect();
        Self(checkers.unwrap_or_default())
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The letters of `text` in words some language knows: a word no spell
    /// checker objects to counts with all its letters, any other with none.
    pub(crate) fn known_letters(&self, text: &str) -> usize {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let words = words(text);
        let mut known = vec![false; words.len()];
        for checker in &self.0 {
            let mut flagged = vec![false; wide.len()];
            // SAFETY: a COM call on this thread with a string it copies.
            let Ok(errors) = (unsafe { checker.Check(&HSTRING::from_wide(&wide)) }) else {
                continue;
            };
            loop {
                let mut error = None;
                // SAFETY: Next writes one error or none into `error`.
                if unsafe { errors.Next(&mut error) }.is_err() {
                    break;
                }
                let Some(error) = error else { break };
                // SAFETY: plain getters on the error object.
                let (start, length, action) = unsafe { (error.StartIndex(), error.Length(), error.CorrectiveAction()) };
                let (Ok(start), Ok(length)) = (start, length) else { continue };
                // A word twice in a row is reported to be deleted, and both
                // are words.
                if action.is_ok_and(|action| action == CORRECTIVE_ACTION_DELETE) {
                    continue;
                }
                let end = (start as usize + length as usize).min(flagged.len());
                for unit in flagged.iter_mut().take(end).skip(start as usize) {
                    *unit = true;
                }
            }
            for (word, known) in words.iter().zip(known.iter_mut()) {
                if !flagged[word.start..word.end].iter().any(|flag| *flag) {
                    *known = true;
                }
            }
        }
        words.iter().zip(&known).filter(|(_, known)| **known).map(|(word, _)| word.letters).sum()
    }
}

/// A word of a text: where it is, in UTF-16 units as the spell checkers
/// count, and how many letters it has.
struct WordAt {
    start: usize,
    end: usize,
    letters: usize,
}

/// The runs of letters in a text, an apostrophe or a hyphen between letters
/// kept inside one.
fn words(text: &str) -> Vec<WordAt> {
    let mut words = Vec::new();
    let mut unit = 0;
    let mut current: Option<WordAt> = None;
    let chars: Vec<char> = text.chars().collect();
    for (index, &c) in chars.iter().enumerate() {
        let joins = matches!(c, '\'' | '’' | '-') && current.is_some() && chars.get(index + 1).is_some_and(|next| next.is_alphabetic());
        if c.is_alphabetic() || joins {
            let word = current.get_or_insert(WordAt {
                start: unit,
                end: unit,
                letters: 0,
            });
            word.end = unit + c.len_utf16();
            word.letters += usize::from(c.is_alphabetic());
        } else if let Some(word) = current.take() {
            words.push(word);
        }
        unit += c.len_utf16();
    }
    words.extend(current);
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_runs_of_letters() {
        let found: Vec<(usize, usize, usize)> = words("don't  re-read 12 Ёж!").iter().map(|w| (w.start, w.end, w.letters)).collect();
        assert_eq!(found, vec![(0, 5, 4), (7, 14, 6), (18, 20, 2)]);
    }
}
