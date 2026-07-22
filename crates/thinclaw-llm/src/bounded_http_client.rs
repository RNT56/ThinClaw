//! Bounded HTTP transport for `rig` provider clients.
//!
//! `rig`'s stock reqwest adapter buffers provider responses and error bodies
//! without a limit. This wrapper keeps the same interface while imposing
//! explicit request, response, error, and streaming budgets.

use bytes::Bytes;
use futures::StreamExt as _;
use rig::http_client::{
    Error, HttpClientExt, LazyBody, MultipartForm, Request, Response, StreamingResponse,
};
use rig::wasm_compat::WasmCompatSend;

const MAX_LLM_REQUEST_BYTES: usize = 64 * 1024 * 1024;
const MAX_LLM_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const MAX_LLM_ERROR_BYTES: usize = 64 * 1024;
const MAX_LLM_STREAM_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub(crate) struct BoundedHttpClient {
    inner: Option<reqwest::Client>,
}

impl BoundedHttpClient {
    pub(crate) fn new(inner: reqwest::Client) -> Self {
        Self { inner: Some(inner) }
    }
}

fn instance_error(error: impl std::error::Error + Send + Sync + 'static) -> Error {
    Error::Instance(Box::new(error))
}

fn limit_error(kind: &str, limit: usize) -> Error {
    instance_error(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("{kind} exceeds the {limit}-byte limit"),
    ))
}

async fn bounded_bytes(response: reqwest::Response, limit: usize) -> Result<Bytes, Error> {
    thinclaw_types::http_response::bounded_bytes(response, limit)
        .await
        .map(Bytes::from)
        .map_err(instance_error)
}

async fn ensure_success(response: reqwest::Response) -> Result<reqwest::Response, Error> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let message =
        match thinclaw_types::http_response::bounded_text(response, MAX_LLM_ERROR_BYTES).await {
            Ok(message) => message,
            Err(error) => format!("unreadable provider error response: {error}"),
        };
    Err(Error::InvalidStatusCodeWithMessage(status, message))
}

impl HttpClientExt for BoundedHttpClient {
    fn send<T, U>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = Result<Response<LazyBody<U>>, Error>> + WasmCompatSend + 'static
    where
        T: Into<Bytes>,
        T: WasmCompatSend,
        U: From<Bytes>,
        U: WasmCompatSend + 'static,
    {
        let client = self.inner.clone();
        let (parts, body) = request.into_parts();
        let body = body.into();
        async move {
            let client = client.ok_or_else(|| {
                instance_error(std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "LLM HTTP client was not initialized",
                ))
            })?;
            if body.len() > MAX_LLM_REQUEST_BYTES {
                return Err(limit_error("LLM request", MAX_LLM_REQUEST_BYTES));
            }
            let response = client
                .request(parts.method, parts.uri.to_string())
                .headers(parts.headers)
                .body(body)
                .send()
                .await
                .map_err(|error| instance_error(error.without_url()))?;
            let response = ensure_success(response).await?;
            let status = response.status();
            let version = response.version();
            let headers = response.headers().clone();
            let body: LazyBody<U> = Box::pin(async move {
                Ok(U::from(
                    bounded_bytes(response, MAX_LLM_RESPONSE_BYTES).await?,
                ))
            });
            let mut result = Response::builder().status(status).version(version);
            if let Some(target) = result.headers_mut() {
                *target = headers;
            }
            result.body(body).map_err(Error::Protocol)
        }
    }

    fn send_multipart<U>(
        &self,
        _request: Request<MultipartForm>,
    ) -> impl Future<Output = Result<Response<LazyBody<U>>, Error>> + WasmCompatSend + 'static
    where
        U: From<Bytes>,
        U: WasmCompatSend + 'static,
    {
        std::future::ready(Err(instance_error(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "multipart requests are not supported by the LLM transport",
        ))))
    }

    fn send_streaming<T>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = Result<StreamingResponse, Error>> + WasmCompatSend
    where
        T: Into<Bytes>,
    {
        let client = self.inner.clone();
        let (parts, body) = request.into_parts();
        let body = body.into();
        async move {
            let client = client.ok_or_else(|| {
                instance_error(std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "LLM HTTP client was not initialized",
                ))
            })?;
            if body.len() > MAX_LLM_REQUEST_BYTES {
                return Err(limit_error("LLM request", MAX_LLM_REQUEST_BYTES));
            }
            let request = client
                .request(parts.method, parts.uri.to_string())
                .headers(parts.headers)
                .body(body)
                .build()
                .map_err(|error| instance_error(error.without_url()))?;
            let response = client
                .execute(request)
                .await
                .map_err(|error| instance_error(error.without_url()))?;
            let response = ensure_success(response).await?;
            let status = response.status();
            let version = response.version();
            let headers = response.headers().clone();
            let stream = response
                .bytes_stream()
                .scan((0usize, false), |(total, failed), item| {
                    let output = if *failed {
                        None
                    } else {
                        match item {
                            Ok(chunk) => match total.checked_add(chunk.len()) {
                                Some(next) if next <= MAX_LLM_STREAM_BYTES => {
                                    *total = next;
                                    Some(Ok(chunk))
                                }
                                _ => {
                                    *failed = true;
                                    Some(Err(limit_error(
                                        "LLM streaming response",
                                        MAX_LLM_STREAM_BYTES,
                                    )))
                                }
                            },
                            Err(error) => {
                                *failed = true;
                                Some(Err(instance_error(error.without_url())))
                            }
                        }
                    };
                    std::future::ready(output)
                });
            let mut result = Response::builder().status(status).version(version);
            if let Some(target) = result.headers_mut() {
                *target = headers;
            }
            result
                .body(Box::pin(stream) as rig::http_client::sse::BoxedStream)
                .map_err(Error::Protocol)
        }
    }
}
