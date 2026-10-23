//! CLIP byte-level BPE tokenizer, read from the `tokenizer.json` shipped with the models.
//!
//! Written by hand to avoid a production dependency: it needs only the `vocab` and `merges`
//! of the file. Text normalisation covers whitespace collapse and lowercase; Unicode NFC and
//! HTML/ftfy cleanup are out of scope.

use crate::error::AiError;
use std::collections::HashMap;
use std::path::Path;

/// Length of the model input, start and end tokens included.
pub const CONTEXT: usize = 77;
const START: &str = "<|startoftext|>";
const END: &str = "<|endoftext|>";
const WORD_END: &str = "</w>";

/// Tokenizer for the CLIP text model.
pub struct ClipTokenizer {
    vocab: HashMap<String, i32>,
    /// Merge rank of a pair, looked up by borrowed strings (left symbol, then right symbol).
    ranks: HashMap<String, HashMap<String, usize>>,
    /// Bytes of a word that can still influence the first 75 tokens (see `from_parts`).
    max_word_bytes: usize,
    byte_chars: [char; 256],
    start: i32,
    end: i32,
}

fn bad(message: impl std::fmt::Display) -> AiError {
    AiError::Model(format!("tokenizer.json: {message}"))
}

/// GPT-2 `bytes_to_unicode`: printable bytes map to themselves, the others to U+0100 and up.
fn byte_chars() -> [char; 256] {
    let mut table = ['\0'; 256];
    let mut extra = 0u32;
    for b in 0..=255u32 {
        let printable = (0x21..=0x7e).contains(&b) || (0xa1..=0xac).contains(&b) || b >= 0xae;
        let code = if printable {
            b
        } else {
            extra += 1;
            255 + extra
        };
        table[b as usize] = char::from_u32(code).unwrap_or('\u{fffd}');
    }
    table
}

/// Splits like CLIP's pattern `'s|'t|'re|'ve|'m|'ll|'d|\p{L}+|\p{N}|[^\s\p{L}\p{N}]+`.
fn pre_tokenize(text: &str) -> Vec<&str> {
    const CONTRACTIONS: [&str; 7] = ["'s", "'t", "'re", "'ve", "'m", "'ll", "'d"];
    let mut pieces = Vec::new();
    let mut rest = text;
    while let Some(first) = rest.chars().next() {
        let len = if first.is_whitespace() {
            first.len_utf8()
        } else if let Some(c) = CONTRACTIONS.iter().find(|c| rest.starts_with(**c)) {
            c.len()
        } else if first.is_alphabetic() {
            run(rest, char::is_alphabetic)
        } else if first.is_numeric() {
            first.len_utf8()
        } else {
            run(rest, |c| {
                !c.is_whitespace() && !c.is_alphabetic() && !c.is_numeric()
            })
        };
        if !first.is_whitespace() {
            pieces.push(&rest[..len]);
        }
        rest = &rest[len..];
    }
    pieces
}

/// The longest prefix of `text` of at most `max` bytes, cut on a character boundary.
fn truncate_to_boundary(text: &str, max: usize) -> &str {
    let mut end = max.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Byte length of the leading run of characters satisfying `keep`.
fn run(text: &str, keep: impl Fn(char) -> bool) -> usize {
    text.char_indices()
        .find(|&(_, c)| !keep(c))
        .map_or(text.len(), |(i, _)| i)
}

impl ClipTokenizer {
    /// Reads the vocabulary and merge rules; a missing or malformed file is a typed error.
    pub fn load(path: &Path) -> Result<Self, AiError> {
        let bytes = std::fs::read(path).map_err(|e| bad(format!("{}: {e}", path.display())))?;
        let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(bad)?;
        let model = &json["model"];

        let mut vocab = HashMap::new();
        for (token, id) in model["vocab"].as_object().ok_or_else(|| bad("no vocab"))? {
            let id = id
                .as_i64()
                .and_then(|i| i32::try_from(i).ok())
                .ok_or_else(|| bad("vocab id is not an integer"))?;
            vocab.insert(token.clone(), id);
        }

        let mut merges = Vec::new();
        for merge in model["merges"]
            .as_array()
            .ok_or_else(|| bad("no merges"))?
            .iter()
        {
            let pair = match merge {
                serde_json::Value::String(s) => s
                    .split_once(' ')
                    .map(|(a, b)| (a.to_string(), b.to_string())),
                serde_json::Value::Array(p) => match (p.first(), p.get(1)) {
                    (Some(a), Some(b)) => a
                        .as_str()
                        .zip(b.as_str())
                        .map(|(a, b)| (a.to_string(), b.to_string())),
                    _ => None,
                },
                _ => None,
            };
            merges.push(pair.ok_or_else(|| bad("malformed merge"))?);
        }

        Self::from_parts(vocab, merges)
    }

    /// Builds a tokenizer from a vocabulary and ordered merge rules.
    ///
    /// A token of the vocabulary stands for at most `longest` bytes, so the first 75 tokens
    /// of a word never reach past `75 * longest` bytes: `encode` truncates each word there
    /// before merging, which bounds the work whatever the input size.
    fn from_parts(
        vocab: HashMap<String, i32>,
        merges: Vec<(String, String)>,
    ) -> Result<Self, AiError> {
        let special = |name: &str| {
            vocab
                .get(name)
                .copied()
                .ok_or_else(|| bad(format!("{name} missing from vocab")))
        };
        let (start, end) = (special(START)?, special(END)?);
        let longest = vocab
            .keys()
            .filter(|k| k.as_str() != START && k.as_str() != END)
            .map(|k| k.strip_suffix(WORD_END).unwrap_or(k).chars().count())
            .max()
            .unwrap_or(1)
            .max(1);
        let mut ranks: HashMap<String, HashMap<String, usize>> = HashMap::new();
        for (rank, (left, right)) in merges.into_iter().enumerate() {
            ranks.entry(left).or_default().entry(right).or_insert(rank);
        }
        Ok(ClipTokenizer {
            vocab,
            ranks,
            max_word_bytes: longest * (CONTEXT - 2),
            byte_chars: byte_chars(),
            start,
            end,
        })
    }

    /// Token ids of `text`: start token, up to 75 content tokens, end token, then zeros.
    pub fn encode(&self, text: &str) -> [i32; CONTEXT] {
        let normalized = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let mut content: Vec<i32> = Vec::new();
        'words: for piece in pre_tokenize(&normalized) {
            for token in self.bpe(truncate_to_boundary(piece, self.max_word_bytes)) {
                if content.len() == CONTEXT - 2 {
                    break 'words;
                }
                if let Some(&id) = self.vocab.get(&token) {
                    content.push(id);
                }
            }
        }
        let mut ids = [0i32; CONTEXT];
        ids[0] = self.start;
        ids[1..=content.len()].copy_from_slice(&content);
        ids[content.len() + 1] = self.end;
        ids
    }

    /// Merges the symbols of one word by ascending rule rank.
    fn bpe(&self, word: &str) -> Vec<String> {
        let mut symbols: Vec<String> = word
            .bytes()
            .map(|b| self.byte_chars[b as usize].to_string())
            .collect();
        if let Some(last) = symbols.last_mut() {
            last.push_str(WORD_END);
        }
        while symbols.len() > 1 {
            let best = symbols
                .windows(2)
                .enumerate()
                .filter_map(|(i, w)| {
                    self.ranks
                        .get(w[0].as_str())
                        .and_then(|row| row.get(w[1].as_str()))
                        .map(|&rank| (rank, i))
                })
                .min();
            let Some((_, i)) = best else { break };
            let right = symbols.remove(i + 1);
            symbols[i].push_str(&right);
        }
        symbols
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_table_matches_gpt2() {
        let t = byte_chars();
        assert_eq!(t[b'a' as usize], 'a');
        assert_eq!(t[b' ' as usize], '\u{120}');
        assert_eq!(t[b'\n' as usize], '\u{10a}');
        assert_eq!(t[0xad], '\u{143}');
        let unique: std::collections::HashSet<_> = t.iter().collect();
        assert_eq!(unique.len(), 256);
    }

    #[test]
    fn splits_like_clip() {
        assert_eq!(pre_tokenize("don't  go!!"), ["don", "'t", "go", "!!"]);
        assert_eq!(pre_tokenize("ab12"), ["ab", "1", "2"]);
        assert_eq!(
            pre_tokenize("caf\u{e9} \u{1f34e}"),
            ["caf\u{e9}", "\u{1f34e}"]
        );
        assert!(pre_tokenize("  \t").is_empty());
    }

    fn tiny() -> ClipTokenizer {
        let vocab = [
            ("a", 1),
            ("b</w>", 2),
            ("ab</w>", 3),
            (START, 10),
            (END, 11),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        ClipTokenizer::from_parts(vocab, vec![("a".into(), "b</w>".into())]).unwrap()
    }

    #[test]
    fn merges_by_rank_and_tolerates_unknown_symbols() {
        let tokenizer = tiny();
        assert_eq!(tokenizer.bpe("ab"), ["ab</w>"]);
        let ids = tokenizer.encode("AB zz");
        assert_eq!(&ids[..3], [10, 3, 11]);
    }

    #[test]
    fn truncation_cuts_on_a_char_boundary() {
        assert_eq!(truncate_to_boundary("abc", 10), "abc");
        assert_eq!(truncate_to_boundary("ab\u{e9}", 3), "ab");
        assert_eq!(truncate_to_boundary("\u{1f34e}", 3), "");
    }

    #[test]
    fn huge_words_are_bounded() {
        let tokenizer = tiny();
        assert_eq!(tokenizer.max_word_bytes, 2 * 75);
        for text in [
            "ab".repeat(500_000),
            "a".repeat(10_000),
            "\u{e9}".repeat(100_000),
        ] {
            let start = std::time::Instant::now();
            let ids = tokenizer.encode(&text);
            assert!(start.elapsed().as_secs_f32() < 2.0, "{:?}", start.elapsed());
            assert_eq!(ids[0], 10);
            assert!(ids.contains(&11));
        }
    }
}
