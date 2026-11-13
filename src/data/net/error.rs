//! One error model for every API: a kind callers can branch on, a stable
//! code, and a message that is safe to show.

use std::{fmt, time::Duration};

/// What went wrong, in terms a caller can act on. The [`ApiError::code`]
/// says exactly what; the kind says what to do about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// Refused before anything was sent: bad input, or something the API
    /// does not support.
    InvalidRequest,
    /// The API answered, but not with what its contract promises.
    InvalidResponse,
    /// The API answered with an error: an error status or an error body.
    Api,
    /// No answer at all: offline, DNS, TLS, a refused or broken connection.
    Network,
    /// The deadline passed before the whole answer arrived.
    Timeout,
    /// Lattice's own code failed while making or handling the call (a
    /// fetch that panicked): a bug, reported with the panic's message.
    Internal,
}

/// A failed API call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiError {
    pub kind: ErrorKind,
    /// A stable machine code: the API's own (`INVALID_EXCHANGE`) or one of
    /// Lattice's (`NETWORK_ERROR`, `REQUEST_TIMEOUT`, ...).
    pub code: String,
    /// What happened, for people: control characters removed and length
    /// bounded, so it is safe to log and show.
    pub message: String,
    /// The HTTP status, when the API answered.
    pub status: Option<u16>,
}

/// The longest message kept, in characters.
const MESSAGE_LIMIT: usize = 240;
/// The longest code kept, in characters.
const CODE_LIMIT: usize = 80;

impl ApiError {
    pub fn new(kind: ErrorKind, code: impl AsRef<str>, message: impl AsRef<str>) -> Self {
        Self {
            kind,
            code: sanitize(code.as_ref(), CODE_LIMIT),
            message: sanitize(message.as_ref(), MESSAGE_LIMIT),
            status: None,
        }
    }

    pub fn invalid_request(message: impl AsRef<str>) -> Self {
        Self::new(ErrorKind::InvalidRequest, "INVALID_REQUEST", message)
    }

    pub fn invalid_response(message: impl AsRef<str>) -> Self {
        Self::new(ErrorKind::InvalidResponse, "INVALID_RESPONSE", message)
    }

    pub fn network(message: impl AsRef<str>) -> Self {
        Self::new(ErrorKind::Network, "NETWORK_ERROR", message)
    }

    pub fn timeout(limit: Duration) -> Self {
        Self::new(
            ErrorKind::Timeout,
            "REQUEST_TIMEOUT",
            format!("no response within {:.1}s", limit.as_secs_f64()),
        )
    }

    /// A call whose code panicked, with the panic's message.
    pub fn panicked(message: impl AsRef<str>) -> Self {
        Self::new(ErrorKind::Internal, "PANICKED", message)
    }

    /// An error the API reported, with its own code and message.
    pub fn api(status: Option<u16>, code: impl AsRef<str>, message: impl AsRef<str>) -> Self {
        Self {
            status,
            ..Self::new(ErrorKind::Api, code, message)
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)?;
        if let Some(status) = self.status {
            write!(f, " (HTTP {status})")?;
        }
        Ok(())
    }
}

impl std::error::Error for ApiError {}

/// Text from outside the app, made safe to keep and show: control
/// characters become spaces and it is cut to `limit` characters.
pub(crate) fn sanitize(text: &str, limit: usize) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(limit)
        .collect::<String>()
        .trim()
        .to_owned()
}
