//! Pure channel configuration helpers.

/// Canonical Telegram transport mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramTransportMode {
    /// Let the host pick the best transport for the current gateway/tunnel state.
    Auto,
    /// Force long polling.
    Polling,
}

impl TelegramTransportMode {
    /// Return the persisted/runtime string for this mode.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Polling => "polling",
        }
    }
}

/// Result of resolving a user-provided Telegram transport mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TelegramTransportModeResolution {
    mode: TelegramTransportMode,
    used_fallback: bool,
}

impl TelegramTransportModeResolution {
    /// Canonical mode selected by the resolver.
    pub const fn mode(self) -> TelegramTransportMode {
        self.mode
    }

    /// Canonical string selected by the resolver.
    pub const fn as_str(self) -> &'static str {
        self.mode.as_str()
    }

    /// Whether the input was unknown and the resolver fell back to the default.
    pub const fn used_fallback(self) -> bool {
        self.used_fallback
    }

    /// Convert the canonical mode to an owned string.
    pub fn into_string(self) -> String {
        self.as_str().to_string()
    }
}

/// Resolve a Telegram transport mode alias to its canonical mode.
///
/// Empty input defaults to `auto`. Unknown input also falls back to `auto`, with
/// `used_fallback` set so callers can decide whether to log or surface it.
pub fn resolve_telegram_transport_mode(value: impl AsRef<str>) -> TelegramTransportModeResolution {
    let (mode, used_fallback) = match value.as_ref().trim().to_ascii_lowercase().as_str() {
        "" | "auto" | "automatic" | "webhook" => (TelegramTransportMode::Auto, false),
        "polling" | "poll" | "off" | "disabled" => (TelegramTransportMode::Polling, false),
        _ => (TelegramTransportMode::Auto, true),
    };

    TelegramTransportModeResolution {
        mode,
        used_fallback,
    }
}

/// Normalize a Telegram transport mode alias to its canonical string.
pub fn normalize_telegram_transport_mode(value: impl AsRef<str>) -> String {
    resolve_telegram_transport_mode(value).into_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_transport_mode_normalizes_auto_aliases() {
        assert_eq!(normalize_telegram_transport_mode(""), "auto");
        assert_eq!(normalize_telegram_transport_mode("auto"), "auto");
        assert_eq!(normalize_telegram_transport_mode(" automatic "), "auto");
        assert_eq!(normalize_telegram_transport_mode("WEBHOOK"), "auto");
    }

    #[test]
    fn telegram_transport_mode_normalizes_polling_aliases() {
        assert_eq!(normalize_telegram_transport_mode("polling"), "polling");
        assert_eq!(normalize_telegram_transport_mode("poll"), "polling");
        assert_eq!(normalize_telegram_transport_mode(" off "), "polling");
        assert_eq!(normalize_telegram_transport_mode("DISABLED"), "polling");
    }

    #[test]
    fn telegram_transport_mode_defaults_unknown_values_to_auto() {
        let resolution = resolve_telegram_transport_mode("mystery");

        assert_eq!(resolution.mode(), TelegramTransportMode::Auto);
        assert_eq!(resolution.as_str(), "auto");
        assert!(resolution.used_fallback());
        assert_eq!(resolution.into_string(), "auto");
    }

    #[test]
    fn telegram_transport_mode_empty_default_is_not_unknown_fallback() {
        let resolution = resolve_telegram_transport_mode("  ");

        assert_eq!(resolution.mode(), TelegramTransportMode::Auto);
        assert!(!resolution.used_fallback());
    }
}
