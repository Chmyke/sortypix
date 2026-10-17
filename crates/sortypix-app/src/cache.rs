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
        let _ = self.insert_protecting(key, value, bytes, |_| false);
    }
    /// Inserts only if the oldest unprotected entries can make room; protected ones stay.
    pub fn insert_protecting(
        &mut self,
        key: K,
        value: V,
        bytes: usize,
        protected: impl Fn(&K) -> bool,
    ) -> bool {
        let evictable: usize = self
            .entries
            .iter()
            .filter(|(k, _)| **k == key || !protected(k))
            .map(|(_, (_, size, _))| *size)
            .sum();
        if bytes > self.budget || self.used - evictable + bytes > self.budget {
            return false;
        }
        if let Some((_, size, _)) = self.entries.remove(&key) {
            self.used -= size;
        }
        while self.used + bytes > self.budget {
            let oldest = self
                .entries
                .iter()
                .filter(|(k, _)| !protected(k))
                .min_by_key(|(_, (_, _, tick))| tick)
                .map(|(key, _)| key.clone());
            let Some((_, size, _)) = oldest.and_then(|key| self.entries.remove(&key)) else {
                return false;
            };
            self.used -= size;
        }
        self.tick += 1;
        self.used += bytes;
        self.entries.insert(key, (value, bytes, self.tick));
        true
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
