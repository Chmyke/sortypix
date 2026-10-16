use std::collections::BTreeMap;
pub struct Cache<K: Ord, V> {
    entries: BTreeMap<K, (V, usize, u64)>,
    budget: usize,
    used: usize,
    tick: u64,
}
impl<K: Ord + Clone, V> Cache<K, V> {
    pub fn new(budget: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            budget,
            used: 0,
            tick: 0,
        }
    }
    pub fn insert(&mut self, key: K, value: V, bytes: usize) {
        if bytes > self.budget {
            return;
        }
        if let Some((_, size, _)) = self.entries.remove(&key) {
            self.used -= size;
        }
        while self.used + bytes > self.budget {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, (_, _, tick))| tick)
                .map(|(key, _)| key.clone());
            if let Some(key) = oldest
                && let Some((_, size, _)) = self.entries.remove(&key)
            {
                self.used -= size;
            }
        }
        self.tick += 1;
        self.used += bytes;
        self.entries.insert(key, (value, bytes, self.tick));
    }
    pub fn get(&mut self, key: &K) -> Option<&V> {
        self.tick += 1;
        self.entries.get_mut(key).map(|(value, _, tick)| {
            *tick = self.tick;
            &*value
        })
    }
    pub fn used_bytes(&self) -> usize {
        self.used
    }
}
pub fn accept_result(current: u64, received: u64) -> bool {
    current == received
}
