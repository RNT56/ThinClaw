use std::fmt;

/// Debug adapter for optional credentials. It deliberately reveals only
/// presence so derived diagnostics cannot disclose the underlying value.
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

pub(crate) struct Redacted;

impl fmt::Debug for Redacted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("\"[REDACTED]\"")
    }
}
