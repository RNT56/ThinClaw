use crate::config::helpers::{parse_bool_env, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Routines configuration.
#[derive(Debug, Clone)]
pub struct RoutineConfig {
    /// Whether the routines system is enabled.
    pub enabled: bool,
    /// How often (seconds) to poll for cron routines that need firing.
    pub cron_check_interval_secs: u64,
    /// Max routines executing concurrently across all users.
    pub max_concurrent_routines: usize,
    /// Default cooldown between fires (seconds).
    pub default_cooldown_secs: u64,
    /// Max output tokens for lightweight routine LLM calls.
    pub max_lightweight_tokens: u32,
    /// How long compiled event cache entries may stay stale before a forced refresh.
    pub event_cache_ttl_secs: u64,
    /// Default max age for replayed durable events before they expire.
    pub default_event_max_age_secs: u64,
    /// Lease duration for claimed routine triggers/events before another node may reclaim them.
    pub claim_lease_secs: u64,
}

impl Default for RoutineConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cron_check_interval_secs: 15,
            max_concurrent_routines: 10,
            default_cooldown_secs: 300,
            max_lightweight_tokens: 4096,
            event_cache_ttl_secs: 30,
            default_event_max_age_secs: 86_400,
            claim_lease_secs: 120,
        }
    }
}

impl RoutineConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let resolved = Self {
            enabled: parse_bool_env("ROUTINES_ENABLED", settings.routines_enabled)?,
            cron_check_interval_secs: parse_optional_env("ROUTINES_CRON_INTERVAL", 15)?,
            max_concurrent_routines: parse_optional_env("ROUTINES_MAX_CONCURRENT", 10)?,
            default_cooldown_secs: parse_optional_env("ROUTINES_DEFAULT_COOLDOWN", 300)?,
            max_lightweight_tokens: parse_optional_env("ROUTINES_MAX_TOKENS", 4096)?,
            event_cache_ttl_secs: parse_optional_env("ROUTINES_EVENT_CACHE_TTL", 30)?,
            default_event_max_age_secs: parse_optional_env("ROUTINES_EVENT_MAX_AGE", 86_400)?,
            claim_lease_secs: parse_optional_env("ROUTINES_CLAIM_LEASE_SECS", 120)?,
        };

        if resolved.cron_check_interval_secs == 0 {
            return Err(ConfigError::InvalidValue {
                key: "ROUTINES_CRON_INTERVAL".to_string(),
                message: "must be greater than 0 seconds".to_string(),
            });
        }
        if resolved.max_concurrent_routines == 0 {
            return Err(ConfigError::InvalidValue {
                key: "ROUTINES_MAX_CONCURRENT".to_string(),
                message: "must be greater than 0".to_string(),
            });
        }
        if resolved.max_lightweight_tokens == 0 {
            return Err(ConfigError::InvalidValue {
                key: "ROUTINES_MAX_TOKENS".to_string(),
                message: "must be greater than 0".to_string(),
            });
        }
        if resolved.event_cache_ttl_secs == 0 {
            return Err(ConfigError::InvalidValue {
                key: "ROUTINES_EVENT_CACHE_TTL".to_string(),
                message: "must be greater than 0 seconds".to_string(),
            });
        }
        if resolved.default_event_max_age_secs == 0 {
            return Err(ConfigError::InvalidValue {
                key: "ROUTINES_EVENT_MAX_AGE".to_string(),
                message: "must be greater than 0 seconds".to_string(),
            });
        }
        if resolved.claim_lease_secs == 0 {
            return Err(ConfigError::InvalidValue {
                key: "ROUTINES_CLAIM_LEASE_SECS".to_string(),
                message: "must be greater than 0 seconds".to_string(),
            });
        }

        Ok(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;

    #[test]
    fn resolve_rejects_zero_cron_interval() {
        let _guard = lock_env();
        unsafe {
            std::env::set_var("ROUTINES_CRON_INTERVAL", "0");
        }

        let err = RoutineConfig::resolve(&Settings::default())
            .expect_err("zero cron interval must be rejected");
        assert!(err.to_string().contains("ROUTINES_CRON_INTERVAL"));

        unsafe {
            std::env::remove_var("ROUTINES_CRON_INTERVAL");
        }
    }
}
