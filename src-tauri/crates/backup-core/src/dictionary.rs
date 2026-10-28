//! The transcription dictionary: canonical spellings and the aliases that should become them.
//!
//! It follows WakeNote's shared Dictionary so the two apps treat the same entries the same way.
//! Canonical terms reach whisper.cpp as a prompt hint, and after transcription every alias is
//! replaced by its canonical term deterministically — ASCII aliases case-insensitively at
//! alphanumeric boundaries, other aliases exactly. Nothing is fuzzy-matched.

use std::cmp::Reverse;
use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// The most entries kept, matching WakeNote's active-line limit for `dictionary.txt`.
pub const MAX_DICTIONARY_ENTRIES: usize = 1_000;
pub const MAX_DICTIONARY_ALIASES: usize = 32;
/// The longest term, alias, or id, in characters.
pub const MAX_DICTIONARY_TEXT_CHARS: usize = 128;
/// The longest prompt built from canonical terms; later terms that would overflow are skipped.
pub const DICTIONARY_PROMPT_MAX_CHARS: usize = 800;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictionaryEntry {
    pub id: String,
    pub term: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

const fn enabled_by_default() -> bool {
    true
}

/// Trims every value, drops entries without a term, removes aliases that repeat the term or
/// each other, and gives every entry a unique id — keeping the user's order.
pub fn normalize_dictionary(entries: Vec<DictionaryEntry>) -> Vec<DictionaryEntry> {
    let mut used_ids = HashSet::new();
    entries
        .into_iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let term = entry.term.trim().to_owned();
            if term.is_empty() {
                return None;
            }
            let requested_id = entry.id.trim();
            let base_id = if requested_id.is_empty() {
                format!("dictionary-{}", index + 1)
            } else {
                requested_id.to_owned()
            };
            let mut id = base_id.clone();
            let mut suffix = 2;
            while !used_ids.insert(id.clone()) {
                id = format!("{base_id}-{suffix}");
                suffix += 1;
            }
            let term_key = comparison_key(&term);
            let mut seen_aliases = HashSet::new();
            let aliases = entry
                .aliases
                .into_iter()
                .filter_map(|alias| {
                    let alias = alias.trim().to_owned();
                    let key = comparison_key(&alias);
                    (!alias.is_empty() && key != term_key && seen_aliases.insert(key))
                        .then_some(alias)
                })
                .collect();
            Some(DictionaryEntry {
                id,
                term,
                aliases,
                enabled: entry.enabled,
            })
        })
        .collect()
}

/// Refuses a dictionary that normalization would change or that exceeds the limits, so the
/// stored list is always exactly what the window shows.
pub fn validate_dictionary(entries: &[DictionaryEntry]) -> Result<(), CoreError> {
    let text_valid = |value: &str| {
        !value.is_empty()
            && value == value.trim()
            && value.chars().count() <= MAX_DICTIONARY_TEXT_CHARS
            && !value.chars().any(char::is_control)
    };
    let mut ids = HashSet::new();
    let mut terms = HashSet::new();
    let valid = entries.len() <= MAX_DICTIONARY_ENTRIES
        && entries.iter().all(|entry| {
            text_valid(&entry.id)
                && text_valid(&entry.term)
                && ids.insert(entry.id.as_str())
                && terms.insert(comparison_key(&entry.term))
                && entry.aliases.len() <= MAX_DICTIONARY_ALIASES
                && entry.aliases.iter().all(|alias| text_valid(alias))
        });
    if valid {
        Ok(())
    } else {
        Err(CoreError::InvalidRequest)
    }
}

/// The dictionary compiled for one transcription: the prompt hint and the alias replacements.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DictionaryContext {
    prompt: Option<String>,
    replacements: Vec<(String, String)>,
}

impl DictionaryContext {
    pub fn compile(enabled: bool, entries: &[DictionaryEntry]) -> Self {
        if !enabled {
            return Self::default();
        }
        let active = entries
            .iter()
            .filter(|entry| entry.enabled && !entry.term.trim().is_empty())
            .collect::<Vec<_>>();
        let mut prompt = String::new();
        for entry in &active {
            let term = entry.term.trim();
            let delimiter = if prompt.is_empty() { 0 } else { 2 };
            if prompt.chars().count() + delimiter + term.chars().count()
                > DICTIONARY_PROMPT_MAX_CHARS
            {
                continue;
            }
            if !prompt.is_empty() {
                prompt.push_str(", ");
            }
            prompt.push_str(term);
        }
        let mut seen_aliases = HashSet::new();
        let mut replacements = active
            .iter()
            .flat_map(|entry| {
                entry
                    .aliases
                    .iter()
                    .map(|alias| (alias.trim().to_owned(), entry.term.trim().to_owned()))
            })
            .filter(|(alias, term)| {
                !alias.is_empty()
                    && comparison_key(alias) != comparison_key(term)
                    && seen_aliases.insert(comparison_key(alias))
            })
            .collect::<Vec<_>>();
        replacements.sort_by_key(|(alias, _)| Reverse(alias.chars().count()));
        Self {
            prompt: (!prompt.is_empty()).then_some(prompt),
            replacements,
        }
    }

    /// Canonical terms joined for whisper.cpp's initial prompt, if any are active.
    pub fn prompt(&self) -> Option<&str> {
        self.prompt.as_deref()
    }

    /// Replaces every alias with its canonical term, longest alias first.
    pub fn correct(&self, text: &str) -> String {
        if text.is_empty() || self.replacements.is_empty() {
            return text.to_owned();
        }
        let mut corrected = String::with_capacity(text.len());
        let mut index = 0;
        while index < text.len() {
            if let Some((alias, canonical)) = self
                .replacements
                .iter()
                .find(|(alias, _)| alias_matches_at(text, index, alias))
            {
                corrected.push_str(canonical);
                index += alias.len();
                continue;
            }
            let Some(character) = text[index..].chars().next() else {
                break;
            };
            corrected.push(character);
            index += character.len_utf8();
        }
        corrected
    }
}

/// ASCII values compare case-insensitively; anything else must match exactly.
fn comparison_key(value: &str) -> String {
    if value.is_ascii() {
        value.to_ascii_lowercase()
    } else {
        value.to_owned()
    }
}

fn alias_matches_at(text: &str, index: usize, alias: &str) -> bool {
    let Some(end) = index.checked_add(alias.len()) else {
        return false;
    };
    let Some(candidate) = text.get(index..end) else {
        return false;
    };
    if !alias.is_ascii() {
        return candidate == alias;
    }
    if !candidate.eq_ignore_ascii_case(alias) {
        return false;
    }
    let left_is_alphanumeric = text[..index]
        .chars()
        .next_back()
        .is_some_and(|character| character.is_ascii_alphanumeric());
    let right_is_alphanumeric = text[end..]
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric());
    !left_is_alphanumeric && !right_is_alphanumeric
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(term: &str, aliases: &[&str]) -> DictionaryEntry {
        DictionaryEntry {
            id: term.to_ascii_lowercase(),
            term: term.to_owned(),
            aliases: aliases.iter().map(|alias| (*alias).to_owned()).collect(),
            enabled: true,
        }
    }

    #[test]
    fn corrects_ascii_aliases_at_word_boundaries_longest_first() {
        let context = DictionaryContext::compile(
            true,
            &[
                entry("WakeNote", &["wake note", "wake"]),
                entry("Qwen3-ASR", &["qwen 3 asr"]),
            ],
        );
        assert_eq!(
            context.correct("Wake Note and QWEN 3 ASR, but not awake or wakeful"),
            "WakeNote and Qwen3-ASR, but not awake or wakeful"
        );
    }

    #[test]
    fn corrects_non_ascii_aliases_exactly() {
        let context = DictionaryContext::compile(true, &[entry("백업 마이크", &["백업마이크"])]);
        assert_eq!(
            context.correct("백업마이크로 녹음했고 백업마이크가 좋아요"),
            "백업 마이크로 녹음했고 백업 마이크가 좋아요"
        );
    }

    #[test]
    fn the_prompt_lists_enabled_terms_within_the_budget() {
        let mut disabled = entry("Hidden", &[]);
        disabled.enabled = false;
        let long = "가".repeat(DICTIONARY_PROMPT_MAX_CHARS);
        let context = DictionaryContext::compile(
            true,
            &[
                entry("WakeNote", &[]),
                disabled,
                entry(&long, &[]),
                entry("DJI", &[]),
            ],
        );
        assert_eq!(context.prompt(), Some("WakeNote, DJI"));
        assert_eq!(
            DictionaryContext::compile(false, &[entry("WakeNote", &[])]).prompt(),
            None
        );
    }

    #[test]
    fn normalization_trims_dedupes_and_assigns_unique_ids_in_order() {
        let normalized = normalize_dictionary(vec![
            DictionaryEntry {
                id: String::new(),
                term: "  WakeNote ".to_owned(),
                aliases: vec![
                    " wake note ".to_owned(),
                    "WAKE NOTE".to_owned(),
                    "wakenote".to_owned(),
                    " ".to_owned(),
                ],
                enabled: true,
            },
            entry("   ", &["dropped"]),
            DictionaryEntry {
                id: "dictionary-1".to_owned(),
                term: "DJI".to_owned(),
                aliases: Vec::new(),
                enabled: false,
            },
        ]);
        assert_eq!(
            normalized,
            vec![
                DictionaryEntry {
                    id: "dictionary-1".to_owned(),
                    term: "WakeNote".to_owned(),
                    aliases: vec!["wake note".to_owned()],
                    enabled: true,
                },
                DictionaryEntry {
                    id: "dictionary-1-2".to_owned(),
                    term: "DJI".to_owned(),
                    aliases: Vec::new(),
                    enabled: false,
                },
            ]
        );
        assert!(validate_dictionary(&normalized).is_ok());
    }

    #[test]
    fn validation_refuses_what_normalization_would_change_or_the_limits_exceed() {
        for refused in [
            vec![entry(" padded", &[])],
            vec![
                entry("dup", &[]),
                DictionaryEntry {
                    id: "other".to_owned(),
                    ..entry("DUP", &[])
                },
            ],
            vec![entry("control\u{7}", &[])],
            vec![entry(&"a".repeat(MAX_DICTIONARY_TEXT_CHARS + 1), &[])],
            vec![entry("many", &["x"; MAX_DICTIONARY_ALIASES + 1])],
        ] {
            assert!(validate_dictionary(&refused).is_err(), "{refused:?}");
        }
        let too_many = (0..=MAX_DICTIONARY_ENTRIES)
            .map(|index| entry(&format!("term{index}"), &[]))
            .collect::<Vec<_>>();
        assert!(validate_dictionary(&too_many).is_err());
    }
}
