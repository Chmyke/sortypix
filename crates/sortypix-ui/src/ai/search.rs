//! ROLE: Sortypix AI only. The search field's state: typed words, the text embedding computed
//! off the interface thread, and the query the grid applies.
//! OWNER: `Ai` holds one `Search`; the toolbar edits `text`, `Ai::poll_search` drives it.
//! INVARIANTS: Never waits on the model: an embedding is asked once per settled text and read
//! with `try_recv`. An answer for words no longer in the field is dropped.
use sortypix_ai::{AiError, Embedding};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// Typing pause after which the words are embedded (Enter embeds at once).
pub(crate) const SETTLE: Duration = Duration::from_millis(300);

type Answer = Receiver<Result<Embedding, AiError>>;

#[derive(Default)]
pub(crate) struct Search {
    /// What the field shows.
    pub text: String,
    /// Last edit not embedded yet.
    edited: Option<Instant>,
    /// Embed at once, without waiting for the typing pause (Enter, clearing).
    due: bool,
    /// Words being embedded.
    pending: Option<(String, Answer)>,
    /// Words and embedding the grid applies.
    query: Option<(String, Box<Embedding>)>,
    /// Ranked by similarity ("Best match") rather than by the order menu.
    pub best_match: bool,
    /// The last embedding failed (shown on the field's hover).
    pub error: Option<String>,
}

impl Search {
    fn words(&self) -> &str {
        self.text.trim()
    }

    /// The field changed (typed, pasted, cleared).
    pub(crate) fn edited(&mut self, now: Instant) {
        self.edited = Some(now);
    }

    /// Embed the words now (Enter), without waiting for the typing pause.
    pub(crate) fn submit(&mut self) {
        self.edited.get_or_insert_with(Instant::now);
        self.due = true;
    }

    pub(crate) fn clear(&mut self) {
        self.text.clear();
        self.submit();
    }

    /// The query the grid applies, if any.
    pub(crate) fn query(&self) -> Option<&Embedding> {
        self.query.as_ref().map(|(_, e)| e.as_ref())
    }

    /// Words of the applied query.
    pub(crate) fn applied(&self) -> Option<&str> {
        self.query.as_ref().map(|(w, _)| w.as_str())
    }

    /// Waiting for an embedding (or for the typing pause to end).
    pub(crate) fn waiting(&self) -> bool {
        self.pending.is_some() || (self.edited.is_some() && !self.words().is_empty())
    }

    /// Advances the search; `embed` asks the model for the words' embedding and returns
    /// `None` while the model is not loaded (asked again on a later frame). True when the
    /// applied query changed (the grid must be arranged again).
    pub(crate) fn poll(
        &mut self,
        now: Instant,
        embed: impl FnOnce(&str) -> Option<Answer>,
    ) -> bool {
        let mut changed = false;
        if self.words().is_empty() {
            self.edited = None;
            self.due = false;
            self.pending = None;
            self.error = None;
            changed = self.query.take().is_some();
        } else if self
            .edited
            .is_some_and(|at| self.due || now.saturating_duration_since(at) >= SETTLE)
        {
            let words = self.words().to_string();
            let known = self.applied() == Some(words.as_str())
                || self.pending.as_ref().is_some_and(|(w, _)| *w == words);
            if known {
                self.edited = None;
                self.due = false;
            } else if let Some(answer) = embed(&words) {
                self.edited = None;
                self.due = false;
                self.pending = Some((words, answer));
            }
        }
        if let Some((words, answer)) = &self.pending {
            match answer.try_recv() {
                Ok(result) => {
                    let current = words.as_str() == self.words();
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

    #[test]
    fn words_are_embedded_once_after_the_typing_pause() {
        let start = Instant::now();
        let mut search = Search {
            text: "ca".into(),
            ..Search::default()
        };
        let mut replies = Vec::new();

        search.edited(start);
        assert!(!search.poll(start, asker(&mut replies)));
        assert!(replies.is_empty(), "embedded while typing");
        search.text = " cat ".into();
        search.edited(start + Duration::from_millis(100));
        assert!(!search.poll(start + SETTLE, asker(&mut replies)));
        assert!(replies.is_empty());
        let later = start + Duration::from_millis(100) + SETTLE;
        assert!(!search.poll(later, asker(&mut replies)));
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].0, "cat");
        assert!(search.waiting() && search.query().is_none());
        replies[0].1.send(Ok(embedding(1.))).unwrap();
        assert!(search.poll(later, asker(&mut replies)));
        assert_eq!(search.applied(), Some("cat"));
        assert!(search.best_match && !search.waiting());
        // The same words again: nothing asked, nothing changed.
        search.edited(later);
        assert!(!search.poll(later + SETTLE, asker(&mut replies)));
        assert_eq!(replies.len(), 1);
    }

    #[test]
    fn an_answer_for_words_no_longer_typed_is_dropped() {
        let now = Instant::now();
        let mut search = Search {
            text: "dog".into(),
            ..Search::default()
        };
        let mut replies = Vec::new();

        search.submit();
        search.poll(now, asker(&mut replies));
        search.text = "dogs".into();
        replies[0].1.send(Ok(embedding(1.))).unwrap();
        assert!(!search.poll(now, asker(&mut replies)));
        assert!(search.query().is_none());
    }

    #[test]
    fn clearing_removes_the_query_and_a_missing_model_retries_later() {
        let now = Instant::now();
        let mut search = Search {
            text: "tree".into(),
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
        assert!(search.query().is_none() && search.text.is_empty());
        assert!(!search.poll(now, asker(&mut replies)));
    }

    #[test]
    fn a_failed_embedding_is_reported_and_keeps_the_grid() {
        let now = Instant::now();
        let mut search = Search {
            text: "boat".into(),
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
}
