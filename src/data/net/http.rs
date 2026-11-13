//! HTTP for every integration, over NSH's `fetch`: the platform's network
//! stack on desktop and the browser's on the web (which applies CORS: the
//! API must allow the page's origin).

use std::{rc::Rc, time::Duration};

use nsh_core::{platform::HttpRequest, prelude::*};
use serde::{Serialize, de::DeserializeOwned};

use super::{error::ApiError, task::deadline};

/// Reads an API's error body: given the status and the raw body, its
/// `(code, message)` when the body is one of the API's error replies.
pub type ErrorDecoder = fn(status: u16, body: &[u8]) -> Option<(String, String)>;

/// How a client talks to its API.
#[derive(Clone)]
pub struct HttpOptions {
    /// The deadline for a whole request, headers and body, unless the
    /// request sets its own.
    pub timeout: Duration,
    /// Sent with every request (an API key, a client id).
    pub headers: Vec<(String, String)>,
    /// Recognizes the API's error replies; see [`ErrorDecoder`].
    pub decode_error: ErrorDecoder,
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(15),
            headers: Vec::new(),
            decode_error: |_, _| None,
        }
    }
}

/// One request to a client's API.
#[derive(Clone, Debug)]
pub struct Request {
    method: &'static str,
    path: String,
    body: Option<Vec<u8>>,
    timeout: Option<Duration>,
}

impl Request {
    /// A `GET` of `path` (from `/`, under the client's origin).
    pub fn get(path: impl Into<String>) -> Self {
        Self {
            method: "GET",
            path: path.into(),
            body: None,
            timeout: None,
        }
    }

    /// A `POST` of `body` as JSON.
    pub fn post_json(
        path: impl Into<String>,
        body: &(impl Serialize + ?Sized),
    ) -> Result<Self, ApiError> {
        let body = serde_json::to_vec(body)
            .map_err(|error| ApiError::invalid_request(format!("unserializable body: {error}")))?;
        Ok(Self {
            method: "POST",
            path: path.into(),
            body: Some(body),
            timeout: None,
        })
    }

    /// Overrides the client's deadline for this request.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

/// A JSON client for one API origin. Cloning is cheap and shares it.
#[derive(Clone)]
pub struct HttpClient(Rc<Inner>);

struct Inner {
    /// `scheme://host[:port]`, without a trailing slash.
    origin: String,
    options: HttpOptions,
}

impl HttpClient {
    /// A client for the API at `base`. Only its scheme, host and port are
    /// kept; each request names its whole path.
    pub fn new(base: &str, options: HttpOptions) -> Result<Self, ApiError> {
        let url = url::Url::parse(base).map_err(|error| {
            ApiError::invalid_request(format!("invalid API URL `{base}`: {error}"))
        })?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(ApiError::invalid_request(format!(
                "API URL `{base}` is not an http(s) origin"
            )));
        }
        let origin = url.origin().ascii_serialization();
        Ok(Self(Rc::new(Inner { origin, options })))
    }

    /// `scheme://host[:port]`.
    pub fn origin(&self) -> &str {
        &self.0.origin
    }

    /// `GET`s `path` and decodes the JSON reply.
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        self.send(Request::get(path)).await
    }

    /// `POST`s `body` as JSON to `path` and decodes the JSON reply.
    pub async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &(impl Serialize + ?Sized),
    ) -> Result<T, ApiError> {
        self.send(Request::post_json(path, body)?).await
    }

    /// Sends `request` and decodes the JSON reply into `T`.
    ///
    /// A success status whose body does not decode is `INVALID_RESPONSE`,
    /// unless the API's [`ErrorDecoder`] recognizes it as an error reply.
    /// An error status is the API's own code and message when the decoder
    /// reads them, else `HTTP_ERROR`. Dropping the future abandons the
    /// request.
    pub async fn send<T: DeserializeOwned>(&self, request: Request) -> Result<T, ApiError> {
        let Inner { origin, options } = &*self.0;
        if !request.path.starts_with('/') {
            return Err(ApiError::invalid_request(format!(
                "request path `{}` must start with `/`",
                request.path
            )));
        }
        let mut http = HttpRequest::new(request.method, format!("{origin}{}", request.path))
            .header("accept", "application/json");
        for (name, value) in &options.headers {
            http = http.header(name.clone(), value.clone());
        }
        if let Some(body) = request.body {
            http = http.header("content-type", "application/json").body(body);
        }

        let limit = request.timeout.unwrap_or(options.timeout);
        let response = match deadline(limit, fetch(http)).await {
            None => return Err(ApiError::timeout(limit)),
            Some(Err(error)) => return Err(ApiError::network(error)),
            Some(Ok(response)) => response,
        };
        let status = response.status();

        if response.ok() {
            return serde_json::from_slice(&response.body).map_err(|error| {
                match (options.decode_error)(status, &response.body) {
                    Some((code, message)) => ApiError::api(Some(status), code, message),
                    None => ApiError {
                        status: Some(status),
                        ..ApiError::invalid_response(format!("unexpected reply: {error}"))
                    },
                }
            });
        }
        let (code, message) = (options.decode_error)(status, &response.body).unwrap_or_else(|| {
            let text = format!("HTTP {status} {}", response.head.status_text);
            ("HTTP_ERROR".to_owned(), text.trim().to_owned())
        });
        Err(ApiError::api(Some(status), code, message))
    }
}
