use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheOutcome {
    Hit(String),
    Miss,
}

#[derive(Debug)]
struct CacheEntry {
    payload: String,
    inserted_at: Instant,
}

#[derive(Debug)]
pub struct ResponseCache {
    ttl: Duration,
    entries: Mutex<HashMap<String, CacheEntry>>,
}

impl ResponseCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, key: &str, now: Instant) -> CacheOutcome {
        let mut entries = self
            .entries
            .lock()
            .expect("cache mutex should not be poisoned");

        match entries.get(key) {
            Some(entry) if now.duration_since(entry.inserted_at) <= self.ttl => {
                CacheOutcome::Hit(entry.payload.clone())
            }
            Some(_) => {
                entries.remove(key);
                CacheOutcome::Miss
            }
            None => CacheOutcome::Miss,
        }
    }

    pub fn insert(&self, key: &str, payload: String, inserted_at: Instant) {
        let mut entries = self
            .entries
            .lock()
            .expect("cache mutex should not be poisoned");
        entries.insert(
            key.to_string(),
            CacheEntry {
                payload,
                inserted_at,
            },
        );
    }

    pub fn invalidate(&self, key: &str) {
        self.entries
            .lock()
            .expect("cache mutex should not be poisoned")
            .remove(key);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{CacheOutcome, ResponseCache};

    #[test]
    fn returns_miss_before_any_value_is_stored() {
        let cache = ResponseCache::new(Duration::from_secs(30));

        let outcome = cache.get("work-items", Instant::now());

        assert_eq!(outcome, CacheOutcome::Miss);
    }

    #[test]
    fn returns_hit_with_cached_value_before_ttl_expires() {
        let cache = ResponseCache::new(Duration::from_secs(30));
        let inserted_at = Instant::now();

        cache.insert("work-items", "payload".to_string(), inserted_at);

        let outcome = cache.get("work-items", inserted_at + Duration::from_secs(5));

        assert_eq!(outcome, CacheOutcome::Hit("payload".to_string()));
    }

    #[test]
    fn invalidate_removes_a_cached_entry() {
        let cache = ResponseCache::new(Duration::from_secs(30));
        let inserted_at = Instant::now();
        cache.insert("work-items", "payload".to_string(), inserted_at);
        cache.invalidate("work-items");
        assert_eq!(cache.get("work-items", inserted_at), CacheOutcome::Miss);
    }

    #[test]
    fn returns_miss_after_ttl_expires() {
        let cache = ResponseCache::new(Duration::from_secs(30));
        let inserted_at = Instant::now();

        cache.insert("work-items", "payload".to_string(), inserted_at);

        let outcome = cache.get("work-items", inserted_at + Duration::from_secs(31));

        assert_eq!(outcome, CacheOutcome::Miss);
    }
}
