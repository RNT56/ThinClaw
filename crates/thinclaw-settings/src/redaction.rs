use std::fmt;

/// Debug adapter that reveals only whether an optional secret is configured.
///
/// Settings remain serializable for persistence, but credentials must never be
/// copied into logs, panic reports, or diagnostics through derived `Debug`.
pub(crate) struct RedactedOption<'a>(pub(crate) &'a Option<String>);

impl fmt::Debug for RedactedOption<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.as_ref().is_some_and(|value| !value.is_empty()) {
            formatter.write_str("Some(\"[REDACTED]\")")
        } else {
            formatter.write_str("None")
        }
    }
}

/// Debug adapter for a required secret value.
pub(crate) struct Redacted;

impl fmt::Debug for Redacted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("\"[REDACTED]\"")
    }
}
