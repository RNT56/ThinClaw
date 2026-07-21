pub(crate) use thinclaw_types::http_response::{BoundedResponseError, bounded_bytes, bounded_json};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_messages_never_include_response_contents() {
        let error = BoundedResponseError::TooLarge { limit: 123 };
        assert_eq!(
            error.to_string(),
            "HTTP response body exceeds the 123-byte limit"
        );
    }
}
