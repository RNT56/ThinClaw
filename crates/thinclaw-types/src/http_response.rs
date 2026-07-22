//! Bounded HTTP response decoding shared by ThinClaw integrations.
//!
//! `reqwest::Response::{bytes,text,json}` buffer the complete peer-controlled
//! body. Every external integration should instead choose an explicit limit
//! and use these helpers, which enforce both declared and streamed sizes.

use serde::de::DeserializeOwned;

#[derive(Debug, thiserror::Error)]
pub enum BoundedResponseError {
    #[error("HTTP response body exceeds the {limit}-byte limit")]
    TooLarge { limit: usize },

    #[error("failed to read HTTP response body: {0}")]
    Http(#[from] reqwest::Error),

    #[error("HTTP response body is not valid UTF-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),

    #[error("HTTP response body is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

pub async fn bounded_bytes(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, BoundedResponseError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(BoundedResponseError::TooLarge { limit });
    }

    let mut body = Vec::with_capacity(
        response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or_default()
            .min(limit),
    );
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(BoundedResponseError::TooLarge { limit });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub async fn bounded_text(
    response: reqwest::Response,
    limit: usize,
) -> Result<String, BoundedResponseError> {
    let body = bounded_bytes(response, limit).await?;
    String::from_utf8(body).map_err(|error| BoundedResponseError::Utf8(error.utf8_error()))
}

pub async fn bounded_json<T: DeserializeOwned>(
    response: reqwest::Response,
    limit: usize,
) -> Result<T, BoundedResponseError> {
    let body = bounded_bytes(response, limit).await?;
    Ok(serde_json::from_slice(&body)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_never_embed_response_contents() {
        assert_eq!(
            BoundedResponseError::TooLarge { limit: 123 }.to_string(),
            "HTTP response body exceeds the 123-byte limit"
        );
    }
}
