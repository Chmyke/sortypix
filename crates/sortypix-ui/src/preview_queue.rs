//! ROLE: Owns cancellable, reprioritizable preview requests shared by image workers.
//! OWNER: One queue belongs to `Textures`; workers only pop requests.
//! INVARIANTS: A key has at most one current request; a generation change cancels queued work.
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, BinaryHeap},
    path::PathBuf,
    sync::{Condvar, Mutex},
};

pub(crate) type Key = (PathBuf, u32);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Priority {
    Refine,
    Ahead,
    Visible,
    Immediate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Request {
    pub key: Key,
    pub generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Queued {
    key: Key,
    priority: Priority,
    generation: u64,
    sequence: u64,
}

impl Ord for Queued {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.sequence.cmp(&self.sequence))
            .then_with(|| self.key.cmp(&other.key))
    }
}

impl PartialOrd for Queued {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Copy)]
struct Desired {
    priority: Priority,
    generation: u64,
    sequence: u64,
}

#[derive(Default)]
struct State {
    generation: u64,
    sequence: u64,
    desired: BTreeMap<Key, Desired>,
    heap: BinaryHeap<Queued>,
    closed: bool,
}

pub(crate) struct PreviewQueue {
    state: Mutex<State>,
    wake: Condvar,
}

impl PreviewQueue {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
        }
    }

    pub(crate) fn request(&self, key: Key, priority: Priority, generation: u64) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if generation < state.generation || state.closed {
            return;
        }
        if generation > state.generation {
            state.generation = generation;
            state.desired.clear();
            state.heap.clear();
        }
        if state
            .desired
            .get(&key)
            .is_some_and(|desired| desired.priority >= priority)
        {
            return;
        }
        state.sequence += 1;
        let sequence = state.sequence;
        state.desired.insert(
            key.clone(),
            Desired {
                priority,
                generation,
                sequence,
            },
        );
        state.heap.push(Queued {
            key,
            priority,
            generation,
            sequence,
        });
        self.wake.notify_one();
    }

    pub(crate) fn retain(&self, generation: u64, desired: &BTreeSet<Key>) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state
            .desired
            .retain(|key, value| value.generation == generation && desired.contains(key));
    }

    pub(crate) fn set_generation(&self, generation: u64) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.generation = generation;
        state.desired.clear();
        state.heap.clear();
    }

    pub(crate) fn pop(&self) -> Option<Request> {
        let mut state = self.state.lock().ok()?;
        loop {
            if let Some(request) = Self::take_current(&mut state) {
                return Some(request);
            }
            if state.closed {
                return None;
            }
            state = self.wake.wait(state).ok()?;
        }
    }

    #[cfg(test)]
    fn try_pop(&self) -> Option<Request> {
        let mut state = self.state.lock().ok()?;
        Self::take_current(&mut state)
    }

    fn take_current(state: &mut State) -> Option<Request> {
        while let Some(queued) = state.heap.pop() {
            let is_current = state.desired.get(&queued.key).is_some_and(|desired| {
                desired.priority == queued.priority
                    && desired.generation == queued.generation
                    && desired.sequence == queued.sequence
            });
            if !is_current {
                continue;
            }
            state.desired.remove(&queued.key);
            return Some(Request {
                key: queued.key,
                generation: queued.generation,
            });
        }
        None
    }

    pub(crate) fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            state.desired.clear();
            state.heap.clear();
        }
        self.wake.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn key(name: &str, edge: u32) -> Key {
        (PathBuf::from(name), edge)
    }

    #[test]
    fn visible_work_overtakes_queued_refinement() {
        let queue = PreviewQueue::new();
        queue.request(key("old", 1600), Priority::Refine, 1);
        queue.request(key("visible", 256), Priority::Visible, 1);

        assert_eq!(queue.try_pop().unwrap().key, key("visible", 256));
        assert_eq!(queue.try_pop().unwrap().key, key("old", 1600));
    }

    #[test]
    fn a_new_generation_cancels_every_old_request() {
        let queue = PreviewQueue::new();
        queue.request(key("old", 256), Priority::Visible, 1);
        queue.set_generation(2);
        queue.request(key("new", 256), Priority::Visible, 2);

        assert_eq!(queue.try_pop().unwrap().key, key("new", 256));
        assert!(queue.try_pop().is_none());
    }

    #[test]
    fn reprioritizing_a_key_decodes_it_only_once() {
        let queue = PreviewQueue::new();
        queue.request(key("photo", 256), Priority::Ahead, 1);
        queue.request(key("photo", 256), Priority::Visible, 1);

        assert_eq!(queue.try_pop().unwrap().key, key("photo", 256));
        assert!(queue.try_pop().is_none());
    }
}
