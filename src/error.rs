//! Errors this crate returns.
//!
//! Every message says what will happen to the caller, not that a value is
//! invalid. A gateway parameter that is wrong is not a style problem: the
//! request usually still succeeds, on settings nobody asked for.
//!
//! The variants line up one for one with the Python SDK's `ParamError`,
//! `CredentialsError`, `ProviderError` and `CheckError`, and the enum itself
//! stands in for its `NodeMavenError` base. The error taxonomy is part of the
//! cross-language contract - a golden vector says which error an invalid input
//! must produce - so the names have to agree even where the shape does not.
//!
//! Python has four more: `ApiError`, `AuthError`, `NotFoundError` and
//! `RateLimitError`, all of them raised by its dashboard API client. There is no
//! client here yet, so they are absent rather than unreachable - a variant
//! nothing ever returns is worse than a missing one, because a caller writes a
//! `match` arm for it and cannot tell that arm is dead. They arrive with the
//! module that raises them.

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
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Error::Param(message) => message,
            Error::Credentials(message) => message,
            Error::Provider(message) => message,
            Error::Check(message) => message,
        };
        f.write_str(message)
    }
}

impl std::error::Error for Error {}

/// The result type every fallible call in this crate returns.
pub type Result<T> = std::result::Result<T, Error>;
