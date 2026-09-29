//! Errors this crate returns.
//!
//! Every message says what will happen to the caller, not that a value is
//! invalid. A gateway parameter that is wrong is not a style problem: the
//! request usually still succeeds, on settings nobody asked for.
//!
//! The variants line up one for one with the Python SDK's error classes and
//! with the names in the shared specification (`sdk-spec`): `param`,
//! `credentials`, `provider`, `check`, `api`, `auth`, `not_found`,
//! `rate_limit`. The enum itself stands in for Python's `NodeMavenError` base.
//! The taxonomy is part of the cross-language contract, so the names have to
//! agree even where the shape does not.
//!
//! In Python `AuthError`, `NotFoundError` and `RateLimitError` are subclasses of
//! `ApiError`, so `except ApiError` catches all four. An enum has no
//! subclasses; [`Error::is_api`] is the same question asked of a value.
//!
//! The four API variants arrived on 2026-09-29 with the account API client.
//! Until then they were absent rather than unreachable, on the argument that a
//! variant nothing returns is worse than a missing one.
//!
//! No variant carries a response body. The Python classes keep one on `.body`;
//! here it is left out on purpose, because two of this API's endpoints return
//! live proxy passwords and an error's `Debug` output goes wherever the
//! caller's logs go.

use std::fmt;

/// Everything this crate returns.
#[derive(Debug, Clone, PartialEq, Eq)]
// The taxonomy can still gain a variant: splitting an unknown parameter name
// from a separator inside a value is an open question the vectors will settle.
#[non_exhaustive]
pub enum Error {
    /// Input the gateway would silently ignore, misreport, or hang on.
    Param(String),
    /// No login or password was given and none was found in the environment.
    Credentials(String),
    /// A provider definition is missing or does not describe a gateway.
    Provider(String),
    /// A CONNECT produced no answer at all: no route, no reply, no status line.
    ///
    /// Deliberately **not** what a refused tunnel returns. A 407 is the gateway
    /// answering the question that was asked, so it comes back as a
    /// [`crate::Check`] carrying the status, the reason phrase, the headers and
    /// the gateway's own explanation. This variant means there is nothing to
    /// report - DNS did not resolve, the connection was refused, the deadline
    /// passed, or whatever answered was not a proxy.
    Check(String),
    /// The account API answered with an error, answered with something that is
    /// not JSON, or did not answer at all. `status` is `None` when no status
    /// line came back - a timeout, a dropped connection, a refused redirect.
    ///
    /// A `2xx` whose body is not JSON is this variant: the dashboard host
    /// answers a path it does not serve with `200` and its web page.
    Api {
        /// The HTTP status, when there was one.
        status: Option<u16>,
        /// What happened, with the server's own reason where it gave one.
        message: String,
    },
    /// The account API refused the key: `401` or `403`.
    Auth {
        /// `401` or `403`.
        status: u16,
        /// What happened, with the server's own reason where it gave one.
        message: String,
    },
    /// `404` where a `404` is not the end of a paged walk.
    NotFound(String),
    /// `429`. Nothing here retries.
    RateLimit(String),
}

impl Error {
    /// Whether this came from the account API: [`Error::Api`], [`Error::Auth`],
    /// [`Error::NotFound`] or [`Error::RateLimit`]. The same question Python's
    /// `except ApiError` asks, since there the last three subclass the first.
    pub fn is_api(&self) -> bool {
        matches!(
            self,
            Error::Api { .. } | Error::Auth { .. } | Error::NotFound(_) | Error::RateLimit(_)
        )
    }

    /// The HTTP status behind an account API error, when there was one.
    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Api { status, .. } => *status,
            Error::Auth { status, .. } => Some(*status),
            Error::NotFound(_) => Some(404),
            Error::RateLimit(_) => Some(429),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Error::Param(message) => message,
            Error::Credentials(message) => message,
            Error::Provider(message) => message,
            Error::Check(message) => message,
            Error::Api { message, .. } => message,
            Error::Auth { message, .. } => message,
            Error::NotFound(message) => message,
            Error::RateLimit(message) => message,
        };
        f.write_str(message)
    }
}

impl std::error::Error for Error {}

/// The result type every fallible call in this crate returns.
pub type Result<T> = std::result::Result<T, Error>;
