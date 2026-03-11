//! Generic API key rotation.
//!
//! Rotates API keys across providers to distribute usage and handle
//! rate limits gracefully.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Key rotation strategy.
#[derive(Debug, Clone, PartialEq)]
pub enum RotationStrategy {
    /// Round-robin across keys.
    RoundRobin,
    /// Use primary until rate-limited, then fall back.
    PrimaryFallback,
    /// Random selection.
    Random,
}

/// A rotatable API key.
#[derive(Debug, Clone)]
pub struct RotatableKey {
    /// The API key value.
    pub key: String,
    /// Label for this key (e.g., "production-1").
    pub label: String,
    /// Whether this key is currently healthy.
    pub healthy: bool,
    /// Number of times this key has been used.
    pub usage_count: u64,
    /// Number of rate-limit hits on this key.
    pub rate_limit_hits: u64,
}

impl RotatableKey {
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            healthy: true,
            usage_count: 0,
            rate_limit_hits: 0,
        }
    }

    /// Masked key for display.
    pub fn masked(&self) -> String {
        if self.key.len() <= 8 {
            "****".to_string()
        } else {
            format!("{}...{}", &self.key[..4], &self.key[self.key.len() - 4..])
        }
    }
}

/// Key rotator for a single provider.
pub struct KeyRotator {
    keys: Vec<RotatableKey>,
    strategy: RotationStrategy,
    cursor: AtomicUsize,
}

impl KeyRotator {
    pub fn new(strategy: RotationStrategy) -> Self {
        Self {
            keys: Vec::new(),
            strategy,
            cursor: AtomicUsize::new(0),
        }
    }

    /// Add a key to the rotation pool.
    pub fn add_key(&mut self, key: RotatableKey) {
        self.keys.push(key);
    }

    /// Get the next key according to the rotation strategy.
    pub fn next_key(&mut self) -> Option<&mut RotatableKey> {
        if self.keys.is_empty() {
            return None;
        }

        match self.strategy {
            RotationStrategy::RoundRobin => {
                let idx = self.cursor.fetch_add(1, Ordering::Relaxed) % self.keys.len();
                // Find next healthy key starting from idx
                for offset in 0..self.keys.len() {
                    let i = (idx + offset) % self.keys.len();
                    if self.keys[i].healthy {
                        self.keys[i].usage_count += 1;
                        return Some(&mut self.keys[i]);
                    }
                }
                None // All unhealthy
            }
            RotationStrategy::PrimaryFallback => {
                // Try primary (first), then fall back
                for key in self.keys.iter_mut() {
                    if key.healthy {
                        key.usage_count += 1;
                        return Some(key);
                    }
                }
                None
            }
            RotationStrategy::Random => {
                let healthy: Vec<usize> = self
                    .keys
                    .iter()
                    .enumerate()
                    .filter(|(_, k)| k.healthy)
                    .map(|(i, _)| i)
                    .collect();

                if healthy.is_empty() {
                    return None;
                }

                // Simple pseudo-random using cursor
                let idx = self.cursor.fetch_add(7, Ordering::Relaxed) % healthy.len();
                let key_idx = healthy[idx];
                self.keys[key_idx].usage_count += 1;
                Some(&mut self.keys[key_idx])
            }
        }
    }

    /// Mark a key as rate-limited (unhealthy).
    pub fn mark_rate_limited(&mut self, label: &str) {
        if let Some(key) = self.keys.iter_mut().find(|k| k.label == label) {
            key.healthy = false;
            key.rate_limit_hits += 1;
        }
    }

    /// Restore a key to healthy status.
    pub fn restore(&mut self, label: &str) {
        if let Some(key) = self.keys.iter_mut().find(|k| k.label == label) {
            key.healthy = true;
        }
    }

    /// Restore all keys.
    pub fn restore_all(&mut self) {
        for key in &mut self.keys {
            key.healthy = true;
        }
    }

    /// Number of healthy keys.
    pub fn healthy_count(&self) -> usize {
        self.keys.iter().filter(|k| k.healthy).count()
    }

    /// Total keys.
    pub fn key_count(&self) -> usize {
        self.keys.len()
    }

    /// Get rotation stats.
    pub fn stats(&self) -> Vec<KeyStats> {
        self.keys
            .iter()
            .map(|k| KeyStats {
                label: k.label.clone(),
                masked_key: k.masked(),
                healthy: k.healthy,
                usage_count: k.usage_count,
                rate_limit_hits: k.rate_limit_hits,
            })
            .collect()
    }
}

/// Stats for a single key.
#[derive(Debug, Clone)]
pub struct KeyStats {
    pub label: String,
    pub masked_key: String,
    pub healthy: bool,
    pub usage_count: u64,
    pub rate_limit_hits: u64,
}

/// Multi-provider key rotation manager.
pub struct KeyRotationManager {
    rotators: HashMap<String, KeyRotator>,
}

impl KeyRotationManager {
    pub fn new() -> Self {
        Self {
            rotators: HashMap::new(),
        }
    }

    /// Get or create a rotator for a provider.
    pub fn rotator_mut(&mut self, provider: &str) -> &mut KeyRotator {
        self.rotators
            .entry(provider.to_string())
            .or_insert_with(|| KeyRotator::new(RotationStrategy::RoundRobin))
    }

    /// Get a key for a provider.
    pub fn next_key(&mut self, provider: &str) -> Option<&str> {
        self.rotators
            .get_mut(provider)
            .and_then(|r| r.next_key().map(|k| k.key.as_str()))
    }

    /// List providers.
    pub fn providers(&self) -> Vec<&str> {
        self.rotators.keys().map(|k| k.as_str()).collect()
    }
}

impl Default for KeyRotationManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_round_robin() {
        let mut rotator = KeyRotator::new(RotationStrategy::RoundRobin);
        rotator.add_key(RotatableKey::new("key-a", "a"));
        rotator.add_key(RotatableKey::new("key-b", "b"));

        let first = rotator.next_key().unwrap().label.clone();
        let second = rotator.next_key().unwrap().label.clone();
        assert_ne!(first, second);
    }

    #[test]
    fn test_primary_fallback() {
        let mut rotator = KeyRotator::new(RotationStrategy::PrimaryFallback);
        rotator.add_key(RotatableKey::new("key-primary", "primary"));
        rotator.add_key(RotatableKey::new("key-backup", "backup"));

        // Always uses primary when healthy
        let label = rotator.next_key().unwrap().label.clone();
        assert_eq!(label, "primary");
    }

    #[test]
    fn test_fallback_on_rate_limit() {
        let mut rotator = KeyRotator::new(RotationStrategy::PrimaryFallback);
        rotator.add_key(RotatableKey::new("key-primary", "primary"));
        rotator.add_key(RotatableKey::new("key-backup", "backup"));

        rotator.mark_rate_limited("primary");
        let label = rotator.next_key().unwrap().label.clone();
        assert_eq!(label, "backup");
    }

    #[test]
    fn test_all_unhealthy() {
        let mut rotator = KeyRotator::new(RotationStrategy::RoundRobin);
        rotator.add_key(RotatableKey::new("key-a", "a"));
        rotator.mark_rate_limited("a");
        assert!(rotator.next_key().is_none());
    }

    #[test]
    fn test_restore() {
        let mut rotator = KeyRotator::new(RotationStrategy::RoundRobin);
        rotator.add_key(RotatableKey::new("key-a", "a"));
        rotator.mark_rate_limited("a");
        assert_eq!(rotator.healthy_count(), 0);
        rotator.restore("a");
        assert_eq!(rotator.healthy_count(), 1);
    }

    #[test]
    fn test_masked_key() {
        let key = RotatableKey::new("sk-1234567890abcdef", "test");
        assert_eq!(key.masked(), "sk-1...cdef");
    }

    #[test]
    fn test_stats() {
        let mut rotator = KeyRotator::new(RotationStrategy::RoundRobin);
        rotator.add_key(RotatableKey::new("key-a", "a"));
        rotator.next_key();
        let stats = rotator.stats();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].usage_count, 1);
    }

    #[test]
    fn test_manager() {
        let mut mgr = KeyRotationManager::new();
        mgr.rotator_mut("openai")
            .add_key(RotatableKey::new("key-1", "k1"));
        assert!(mgr.next_key("openai").is_some());
        assert!(mgr.next_key("anthropic").is_none());
    }
}
