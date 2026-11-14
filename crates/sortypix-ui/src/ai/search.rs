//! ROLE: Sortypix AI only. The search field's state: typed words split into tags and free words,
//! the free words' text embedding computed off the interface thread, and what the grid applies.
//! OWNER: `Ai` holds one `Search`; the toolbar edits `text`, `Ai::poll_search` drives it.
//! INVARIANTS: Never waits on the model: an embedding is asked once per settled text and read
//! with `try_recv`. An answer for words no longer in the field is dropped. The field reads like a
//! prompt: comma-separated terms; a term that is a word of the vocabulary filters by that tag
//! ("dog, beach": dogs on a beach), the other terms together are one free search.
use sortypix_ai::vocabulary;
use sortypix_ai::{AiError, Embedding};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// Typing pause after which the words are embedded (Enter embeds at once).
pub(crate) const SETTLE: Duration = Duration::from_millis(300);

type Answer = Receiver<Result<Embedding, AiError>>;

/// The terms of `text`: tags (vocabulary indices, each once, in typed order) and the free words
/// left (joined by ", ", empty when every term is a tag).
pub(crate) fn terms(text: &str) -> (Vec<u16>, String) {
    let mut tags = Vec::new();
    let mut free = Vec::new();
    for term in text.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        match vocabulary::find(term) {
            Some(i) => {
                let i = i as u16;
                if !tags.contains(&i) {
                    tags.push(i);
                }
            }
            None => free.push(term),
        }
    }
    (tags, free.join(", "))
}

/// The term being typed: what follows the last comma, trimmed at the start.
pub(crate) fn last_term(text: &str) -> &str {
    text.rsplit(',').next().unwrap_or("").trim_start()
}

/// `text` with its last term replaced by the word `name`, ready for the next term.
pub(crate) fn complete(text: &str, name: &str) -> String {
    let head = match text.rfind(',') {
        Some(at) => format!("{}, ", text[..at].trim_end()),
        None => String::new(),
    };
    format!("{head}{name}, ")
}

/// Words to offer for the `term` being typed: present in the folder (`counts`, photos per word),
/// not already `asked`; those whose name starts with the term, then those with a later word that
/// does ("hair" offers "long hair"); most frequent first, then vocabulary order. Every present
/// word when the term is empty. At most `limit`.
pub(crate) fn completions(
    term: &str,
    counts: &[usize],
    asked: &[u16],
    limit: usize,
) -> Vec<(u16, usize)> {
    let term = term.trim().to_lowercase();
    let mut out: Vec<(bool, u16, usize)> = vocabulary::VOCABULARY
        .iter()
        .enumerate()
        .filter_map(|(i, word)| {
            let count = counts.get(i).copied().unwrap_or(0);
            let i = i as u16;
            if count == 0 || asked.contains(&i) {
                return None;
            }
            let first = word.name.starts_with(&term);
            let later = word
                .name
                .split(' ')
                .skip(1)
                .any(|part| part.starts_with(&term));
            (first || later).then_some((!first, i, count))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0).then(b.2.cmp(&a.2)).then(a.1.cmp(&b.1)));
    out.into_iter()
        .take(limit)
        .map(|(_, i, count)| (i, count))
        .collect()
}

/// The row of a list of `len` words chosen after an arrow key (`down`), from `cursor` (`None`:
/// no row chosen yet, the first is the default). Up from the first row leaves the list.
pub(crate) fn step(cursor: Option<usize>, len: usize, down: bool) -> Option<usize> {
    if len == 0 {
        return None;
    }
    match (cursor, down) {
        (None, true) => Some(0),
        (None, false) => None,
        (Some(c), true) => Some((c + 1).min(len - 1)),
        (Some(c), false) => c.min(len - 1).checked_sub(1),
    }
}

#[derive(Default)]
pub(crate) struct Search {
    /// What the field shows.
    pub text: String,
    /// Last edit not applied yet.
    edited: Option<Instant>,
    /// Apply at once, without waiting for the typing pause (Enter, clearing, a completion).
    due: bool,
    /// Free words being embedded.
    pending: Option<(String, Answer)>,
    /// Free words and embedding the grid applies.
    query: Option<(String, Box<Embedding>)>,
    /// Tags the grid applies (vocabulary indices).
    tags: Vec<u16>,
    /// Ranked by similarity ("Best match") rather than by the order menu.
    pub best_match: bool,
    /// The last embedding failed (shown on the field's hover).
    pub error: Option<String>,
}

impl Search {
    fn free(&self) -> String {
        terms(&self.text).1
    }

    /// The field changed (typed, pasted, cleared).
    pub(crate) fn edited(&mut self, now: Instant) {
        self.edited = Some(now);
    }

    /// Apply the words now (Enter, a completion), without waiting for the typing pause.
    pub(crate) fn submit(&mut self) {
        self.edited.get_or_insert_with(Instant::now);
        self.due = true;
    }

    pub(crate) fn clear(&mut self) {
        self.text.clear();
        self.submit();
    }

    /// The free-words query the grid applies, if any.
    pub(crate) fn query(&self) -> Option<&Embedding> {
        self.query.as_ref().map(|(_, e)| e.as_ref())
    }

    /// Free words of the applied query.
    pub(crate) fn applied(&self) -> Option<&str> {
        self.query.as_ref().map(|(w, _)| w.as_str())
    }

    /// Tags the grid applies.
    pub(crate) fn tags(&self) -> &[u16] {
        &self.tags
    }

    /// A query or tags narrow the grid.
    pub(crate) fn active(&self) -> bool {
        self.query.is_some() || !self.tags.is_empty()
    }

    /// Waiting for an embedding (or for the typing pause to end).
    pub(crate) fn waiting(&self) -> bool {
        self.pending.is_some() || (self.edited.is_some() && !self.text.trim().is_empty())
    }

    /// Advances the search; `embed` asks the model for the free words' embedding and returns
    /// `None` while the model is not loaded (asked again on a later frame). True when what the
    /// grid applies changed (it must be arranged again).
    pub(crate) fn poll(
        &mut self,
        now: Instant,
        embed: impl FnOnce(&str) -> Option<Answer>,
    ) -> bool {
        let mut changed = false;
        let settled = self
            .edited
            .is_some_and(|at| self.due || now.saturating_duration_since(at) >= SETTLE);
        if settled {
            let (tags, free) = terms(&self.text);
            if tags != self.tags {
                if !tags.is_empty() {
                    self.best_match = true;
                }
                self.tags = tags;
                changed = true;
            }
            if free.is_empty() {
                self.edited = None;
                self.due = false;
                self.pending = None;
                self.error = None;
                changed |= self.query.take().is_some();
            } else {
                let known = self.applied() == Some(free.as_str())
                    || self.pending.as_ref().is_some_and(|(w, _)| *w == free);
                if known {
                    self.edited = None;
                    self.due = false;
                } else if let Some(answer) = embed(&free) {
                    self.edited = None;
                    self.due = false;
                    self.pending = Some((free, answer));
                }
            }
        }
        if let Some((words, answer)) = &self.pending {
            match answer.try_recv() {
                Ok(result) => {
                    let current = *words == self.free();
                    match result {
                        Ok(embedding) if current => {
                            self.query = Some((words.clone(), Box::new(embedding)));
                            self.best_match = true;
                            self.error = None;
                            changed = true;
                        }
                        Ok(_) => {}
                        Err(e) => self.error = Some(e.to_string()),
                    }
                    self.pending = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.pending = None,
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sortypix_ai::EMBED_DIM;
    use std::sync::mpsc::{self, Sender};

    type Reply = Sender<Result<Embedding, AiError>>;

    fn asker(replies: &mut Vec<(String, Reply)>) -> impl FnOnce(&str) -> Option<Answer> + '_ {
        |words: &str| {
            let (send, receive) = mpsc::channel();
            replies.push((words.to_string(), send));
            Some(receive)
        }
    }

    fn embedding(v: f32) -> Embedding {
        let mut e = [0.; EMBED_DIM];
        e[0] = v;
        e
    }

    fn word(name: &str) -> u16 {
        vocabulary::find(name).unwrap() as u16
    }

    #[test]
    fn words_are_embedded_once_after_the_typing_pause() {
        let start = Instant::now();
        let mut search = Search {
            text: "re".into(),
            ..Search::default()
        };
        let mut replies = Vec::new();

        search.edited(start);
        assert!(!search.poll(start, asker(&mut replies)));
        assert!(replies.is_empty(), "embedded while typing");
        search.text = " red kite ".into();
        search.edited(start + Duration::from_millis(100));
        assert!(!search.poll(start + SETTLE, asker(&mut replies)));
        assert!(replies.is_empty());
        let later = start + Duration::from_millis(100) + SETTLE;
        assert!(!search.poll(later, asker(&mut replies)));
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].0, "red kite");
        assert!(search.waiting() && search.query().is_none());
        replies[0].1.send(Ok(embedding(1.))).unwrap();
        assert!(search.poll(later, asker(&mut replies)));
        assert_eq!(search.applied(), Some("red kite"));
        assert!(search.best_match && !search.waiting() && search.active());
        // The same words again: nothing asked, nothing changed.
        search.edited(later);
        assert!(!search.poll(later + SETTLE, asker(&mut replies)));
        assert_eq!(replies.len(), 1);
    }

    #[test]
    fn an_answer_for_words_no_longer_typed_is_dropped() {
        let now = Instant::now();
        let mut search = Search {
            text: "teapot".into(),
            ..Search::default()
        };
        let mut replies = Vec::new();

        search.submit();
        search.poll(now, asker(&mut replies));
        search.text = "teapots".into();
        replies[0].1.send(Ok(embedding(1.))).unwrap();
        assert!(!search.poll(now, asker(&mut replies)));
        assert!(search.query().is_none());
    }

    #[test]
    fn clearing_removes_the_query_and_a_missing_model_retries_later() {
        let now = Instant::now();
        let mut search = Search {
            text: "old oak".into(),
            ..Search::default()
        };

        search.submit();
        // Model not loaded yet: asked again on the next poll.
        assert!(!search.poll(now, |_| None));
        let mut replies = Vec::new();
        search.poll(now, asker(&mut replies));
        assert_eq!(replies.len(), 1);
        replies[0].1.send(Ok(embedding(1.))).unwrap();
        assert!(search.poll(now, asker(&mut replies)));
        search.clear();
        assert!(search.poll(now, asker(&mut replies)));
        assert!(search.query().is_none() && search.text.is_empty() && !search.active());
        assert!(!search.poll(now, asker(&mut replies)));
    }

    #[test]
    fn a_failed_embedding_is_reported_and_keeps_the_grid() {
        let now = Instant::now();
        let mut search = Search {
            text: "rowing".into(),
            ..Search::default()
        };
        let mut replies = Vec::new();

        search.submit();
        search.poll(now, asker(&mut replies));
        replies[0]
            .1
            .send(Err(AiError::Model("broken".into())))
            .unwrap();
        assert!(!search.poll(now, asker(&mut replies)));
        assert!(search.error.is_some() && search.query().is_none());
        drop(replies);
        assert!(!search.waiting());
    }

    #[test]
    fn vocabulary_words_are_tags_and_need_no_model() {
        let now = Instant::now();
        let mut search = Search {
            text: "Dog, beach, dog, ".into(),
            ..Search::default()
        };
        search.submit();
        // No embedding asked: every term is a tag.
        assert!(search.poll(now, |_| panic!("embedded a tag")));
        assert_eq!(search.tags(), &[word("dog"), word("beach")]);
        assert!(search.active() && search.best_match && search.query().is_none());
        // Tags and free words together.
        search.text = "dog, red frisbee".into();
        search.submit();
        let mut replies = Vec::new();
        assert!(search.poll(now, asker(&mut replies)));
        assert_eq!(replies[0].0, "red frisbee");
        assert_eq!(search.tags(), &[word("dog")]);
        search.clear();
        assert!(search.poll(now, asker(&mut replies)));
        assert!(search.tags().is_empty() && !search.active());
    }

    #[test]
    fn completions_offer_present_words_by_name_then_by_count() {
        let mut counts = vec![0; vocabulary::VOCABULARY.len()];
        counts[word("beach") as usize] = 3;
        counts[word("bear") as usize] = 9;
        counts[word("bed") as usize] = 1;
        counts[word("long hair") as usize] = 4;
        counts[word("hat") as usize] = 2;
        // Empty term: every present word, most frequent first.
        let all = completions("", &counts, &[], 10);
        assert_eq!(all[0], (word("bear"), 9));
        assert_eq!(all.len(), 5);
        // "be": names starting so, by count; "bench" absent from the folder is not offered.
        assert_eq!(
            completions("Be", &counts, &[], 10),
            [(word("bear"), 9), (word("beach"), 3), (word("bed"), 1)]
        );
        // A later word matches after the names that start with the term.
        assert_eq!(
            completions("ha", &counts, &[], 10),
            [(word("hat"), 2), (word("long hair"), 4)]
        );
        // Already asked, or beyond the limit: left out.
        assert_eq!(
            completions("be", &counts, &[word("bear")], 1),
            [(word("beach"), 3)]
        );
    }

    #[test]
    fn arrows_walk_the_list_within_its_ends() {
        assert_eq!(step(None, 0, true), None);
        assert_eq!(step(None, 3, true), Some(0));
        assert_eq!(step(Some(0), 3, true), Some(1));
        assert_eq!(step(Some(2), 3, true), Some(2));
        assert_eq!(step(Some(2), 3, false), Some(1));
        assert_eq!(step(Some(0), 3, false), None);
        assert_eq!(step(None, 3, false), None);
        // A shorter list since: back inside it.
        assert_eq!(step(Some(9), 3, false), Some(1));
        assert_eq!(step(Some(9), 3, true), Some(2));
        assert_eq!(step(Some(5), 1, false), None);
    }

    #[test]
    fn terms_and_completion_read_like_a_prompt() {
        assert_eq!(terms(""), (vec![], String::new()));
        assert_eq!(
            terms(" long hair ,red car, , sky"),
            (vec![word("long hair"), word("sky")], "red car".into())
        );
        assert_eq!(last_term("dog, be"), "be");
        assert_eq!(last_term("be"), "be");
        assert_eq!(last_term("dog, "), "");
        assert_eq!(complete("dog, be", "beach"), "dog, beach, ");
        assert_eq!(complete("be", "beach"), "beach, ");
        assert_eq!(complete("", "beach"), "beach, ");
    }
}
