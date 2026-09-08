//! Build and validate proxy connection strings.
//!
//! This crate builds the username a proxy gateway expects, refuses the input
//! that gateway would mishandle, and hands the result to whatever HTTP client
//! you already use. It holds no session and retries nothing.
//!
//! <!--
//! That sentence read "It opens no socket, holds no session and retries
//! nothing" until 2026-09-07, and the first clause stopped being true the same
//! day: `Proxy::check` opens one CONNECT. The correction is kept here rather
//! than made silently, because the identical sentence was the Python SDK's
//! opening line and shipped false to PyPI for exactly as long as nobody
//! re-read it. `Proxy` itself still opens nothing - the socket is one explicit
//! call, never on construction and never on `url()`.
//! -->
//!
//! ```
//! # fn main() -> Result<(), nodemaven::Error> {
//! use nodemaven::Proxy;
//!
//! let proxy = Proxy::builder()
//!     .login("your-login")
//!     .password("your-password")
//!     .param("country", "us")
//!     .param("filter", "medium")
//!     .build()?;
//!
//! // reqwest::Proxy::all(proxy.url())?
//! assert_eq!(proxy.username(), "your-login-country-us-filter-medium");
//! # Ok(())
//! # }
//! ```
//!
//! There is no retry policy here on purpose. Retrying a refused request is the
//! one thing that reliably makes the next one worse: measured over 1464
//! attempts, the chance the next attempt succeeds falls from 75% with no prior
//! failure to 5.8% after five and 0.5% after seven, and 294 attempts spent past
//! six consecutive failures returned three pages - 98 attempts per delivered
//! page against 1.7 in a healthy session. A library that hid that behind a
//! default would be spending a shared pool's reputation on your behalf.

#![forbid(unsafe_code)]

mod check;
mod error;
mod provider;
mod proxy;

pub use crate::check::{Check, Connect, DEFAULT_TARGET, DEFAULT_TIMEOUT};
pub use crate::error::{Error, Result};
pub use crate::provider::{
    available, load, load_file, load_file_as, load_str, Provider, ProviderBuilder,
    ASCII_WHITESPACE, DEFAULT_PROVIDER,
};
pub use crate::proxy::{BrowserProxy, Proxy, ProxyBuilder};
