//! Auth profiles — multiple authentication strategies per provider.
//!
//! Supports managing multiple API keys/tokens per provider with
//! rotation and fallback capabilities.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A single auth profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthProfile {
    /// Profile name (e.g., "production", "development", "backup").
    pub name: String,
    /// Provider this profile is for (e.g., "openai", "anthropic").
    pub provider: String,
    /// API key (masked in display).
    #[serde(skip_serializing)]
    pub api_key: String,
    /// Base URL override (if different from default).
    pub base_url: Option<String>,
    /// Organization/project ID.
    pub org_id: Option<String>,
    /// Whether this profile is the active/default one.
    pub is_default: bool,
    /// Whether this profile is currently healthy (passes connectivity check).
    pub healthy: bool,
    /// Usage count (for round-robin rotation).
    pub usage_count: u64,
}

impl AuthProfile {
    /// Create a new auth profile.
    pub fn new(
        name: impl Into<String>,
        provider: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            provider: provider.into(),
            api_key: api_key.into(),
            base_url: None,
            org_id: None,
            is_default: false,
            healthy: true,
            usage_count: 0,
        }
    }

    /// Masked version of the API key for display.
    pub fn masked_key(&self) -> String {
        let chars: Vec<char> = self.api_key.chars().collect();
        if chars.len() <= 8 {
            "****".to_string()
        } else {
            let prefix: String = chars[..4].iter().collect();
            let suffix: String = chars[chars.len() - 4..].iter().collect();
            format!("{}...{}", prefix, suffix)
        }
    }
}

/// Auth profile manager.
pub struct AuthProfileManager {
    /// Profiles grouped by provider.
    profiles: HashMap<String, Vec<AuthProfile>>,
}

impl AuthProfileManager {
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
        }
    }

    /// Add a profile.
    pub fn add(&mut self, profile: AuthProfile) {
        let provider = profile.provider.clone();
        self.profiles.entry(provider).or_default().push(profile);
    }

    /// Get the default profile for a provider.
    pub fn get_default(&self, provider: &str) -> Option<&AuthProfile> {
        self.profiles
            .get(provider)
            .and_then(|profiles| profiles.iter().find(|p| p.is_default).or(profiles.first()))
    }

    /// Get a specific profile by name.
    pub fn get_by_name(&self, provider: &str, name: &str) -> Option<&AuthProfile> {
        self.profiles
            .get(provider)
            .and_then(|profiles| profiles.iter().find(|p| p.name == name))
    }

    /// Set a profile as the default for its provider.
    pub fn set_default(&mut self, provider: &str, name: &str) -> bool {
        if let Some(profiles) = self.profiles.get_mut(provider) {
            for p in profiles.iter_mut() {
                p.is_default = p.name == name;
            }
            true
        } else {
            false
        }
    }

    /// Get the next healthy profile (round-robin rotation).
    pub fn next_healthy(&mut self, provider: &str) -> Option<&AuthProfile> {
        if let Some(profiles) = self.profiles.get_mut(provider) {
            // Find the healthy profile with the lowest usage count
            let min_usage = profiles
                .iter()
                .filter(|p| p.healthy)
                .map(|p| p.usage_count)
                .min();

            if let Some(min) = min_usage
                && let Some(profile) = profiles
                    .iter_mut()
                    .find(|p| p.healthy && p.usage_count == min)
            {
                profile.usage_count += 1;
                return Some(profile);
            }
        }
        None
    }

    /// Mark a profile as unhealthy.
    pub fn mark_unhealthy(&mut self, provider: &str, name: &str) {
        if let Some(profiles) = self.profiles.get_mut(provider)
            && let Some(p) = profiles.iter_mut().find(|p| p.name == name)
        {
            p.healthy = false;
        }
    }

    /// Mark a profile as healthy.
    pub fn mark_healthy(&mut self, provider: &str, name: &str) {
        if let Some(profiles) = self.profiles.get_mut(provider)
            && let Some(p) = profiles.iter_mut().find(|p| p.name == name)
        {
            p.healthy = true;
        }
    }

    /// List all profiles for a provider.
    pub fn list(&self, provider: &str) -> Vec<&AuthProfile> {
        self.profiles
            .get(provider)
            .map(|ps| ps.iter().collect())
            .unwrap_or_default()
    }

    /// List all providers.
    pub fn providers(&self) -> Vec<&str> {
        self.profiles.keys().map(|k| k.as_str()).collect()
    }

    /// Remove a profile.
    pub fn remove(&mut self, provider: &str, name: &str) -> bool {
        if let Some(profiles) = self.profiles.get_mut(provider) {
            let before = profiles.len();
            profiles.retain(|p| p.name != name);
            profiles.len() < before
        } else {
            false
        }
    }

    /// Total number of profiles.
    pub fn total_profiles(&self) -> usize {
        self.profiles.values().map(|v| v.len()).sum()
    }
}

impl Default for AuthProfileManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_profile(name: &str, provider: &str) -> AuthProfile {
        AuthProfile::new(name, provider, format!("sk-test-key-{}", name))
    }

    #[test]
    fn test_add_and_get_default() {
        let mut mgr = AuthProfileManager::new();
        let mut p = make_profile("prod", "openai");
        p.is_default = true;
        mgr.add(p);

        let default = mgr.get_default("openai").unwrap();
        assert_eq!(default.name, "prod");
    }

    #[test]
    fn test_fallback_to_first() {
        let mut mgr = AuthProfileManager::new();
        mgr.add(make_profile("dev", "openai"));

        let default = mgr.get_default("openai").unwrap();
        assert_eq!(default.name, "dev"); // Falls back to first
    }

    #[test]
    fn test_set_default() {
        let mut mgr = AuthProfileManager::new();
        mgr.add(make_profile("prod", "openai"));
        mgr.add(make_profile("dev", "openai"));
        mgr.set_default("openai", "dev");

        let default = mgr.get_default("openai").unwrap();
        assert_eq!(default.name, "dev");
    }

    #[test]
    fn test_masked_key() {
        let profile = AuthProfile::new("test", "openai", "sk-1234567890abcdef");
        assert_eq!(profile.masked_key(), "sk-1...cdef");
    }

    #[test]
    fn test_short_key_masked() {
        let profile = AuthProfile::new("test", "openai", "short");
        assert_eq!(profile.masked_key(), "****");
    }

    #[test]
    fn test_round_robin() {
        let mut mgr = AuthProfileManager::new();
        mgr.add(make_profile("a", "openai"));
        mgr.add(make_profile("b", "openai"));

        let first = mgr.next_healthy("openai").unwrap().name.clone();
        let second = mgr.next_healthy("openai").unwrap().name.clone();
        assert_ne!(first, second); // Should alternate
    }

    #[test]
    fn test_mark_unhealthy() {
        let mut mgr = AuthProfileManager::new();
        mgr.add(make_profile("a", "openai"));
        mgr.add(make_profile("b", "openai"));
        mgr.mark_unhealthy("openai", "a");

        let next = mgr.next_healthy("openai").unwrap();
        assert_eq!(next.name, "b"); // Skips unhealthy "a"
    }

    #[test]
    fn test_remove() {
        let mut mgr = AuthProfileManager::new();
        mgr.add(make_profile("a", "openai"));
        assert!(mgr.remove("openai", "a"));
        assert_eq!(mgr.total_profiles(), 0);
    }

    #[test]
    fn test_providers() {
        let mut mgr = AuthProfileManager::new();
        mgr.add(make_profile("a", "openai"));
        mgr.add(make_profile("b", "anthropic"));
        let providers = mgr.providers();
        assert_eq!(providers.len(), 2);
    }
}
