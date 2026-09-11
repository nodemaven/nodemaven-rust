//! One proxy identity, and the strings a client needs to use it.
//!
//! This module opens no socket. It builds a username in the gateway's dialect,
//! refuses input the gateway would mishandle, and hands the result to whatever
//! HTTP client you already have.

use std::fmt;

use crate::check;
use crate::check::{Check, Connect};
use crate::error::{Error, Result};
use crate::provider::{Provider, ASCII_WHITESPACE, DEFAULT_PROVIDER};

const REDACTED: &str = "***";

/// A set of gateway parameters plus the credentials to use them.
///
/// ```
/// # fn main() -> Result<(), nodemaven::Error> {
/// use nodemaven::Proxy;
///
/// let proxy = Proxy::builder()
///     .login("your-login")
///     .password("your-password")
///     .param("country", "us")
///     .param("filter", "medium")
///     .build()?;
///
/// assert_eq!(proxy.username(), "your-login-country-us-filter-medium");
/// # Ok(())
/// # }
/// ```
///
/// Credentials fall back to the environment when not passed, under the
/// provider's id in upper case: `NODEMAVEN_LOGIN`, `NODEMAVEN_PASSWORD`,
/// `NODEMAVEN_HOST`, `NODEMAVEN_PORT`.
///
/// **One instance is one identity.** On the gateway this crate ships a
/// definition for, the sticky session key is the whole recognised parameter set
/// and not the session id alone: `country=us, sid=A` and
/// `country=us, sid=A, filter=medium` are two different sessions, and adding or
/// removing any parameter moves you to a different exit address without saying
/// so. Build the object once and reuse it; use [`Proxy::with_param`] and
/// [`Proxy::without_param`] when you intend to move, so that the move is
/// written down.
#[derive(Clone, PartialEq, Eq)]
pub struct Proxy {
    provider: Provider,
    login: String,
    password: String,
    host: String,
    port: u16,
    // An ordered list of pairs rather than a map, because order is part of the
    // answer: `country` then `sid` builds a different username from `sid` then
    // `country`, and `HashMap` would reorder them while `BTreeMap` would sort
    // them. The golden vectors the other language SDKs run carry parameters as
    // an ordered array of pairs for the same reason.
    params: Vec<(String, String)>,
}

impl Proxy {
    /// Start building an identity.
    pub fn builder() -> ProxyBuilder {
        ProxyBuilder {
            provider: None,
            login: None,
            password: None,
            host: None,
            port: None,
            params: Vec::new(),
        }
    }

    /// The gateway definition this identity was built against.
    ///
    /// Exposed because it answers the two questions a caller cannot otherwise
    /// ask: [`Provider::known_params`] is what this object will accept, and
    /// [`Provider::is_measured`] is whether anyone has confirmed the dialect
    /// against the gateway or only read it out of documentation.
    pub fn provider(&self) -> &Provider {
        &self.provider
    }

    /// The gateway parameters, in the order they will be sent.
    pub fn params(&self) -> &[(String, String)] {
        &self.params
    }

    /// `host:port`, with no credentials in it.
    pub fn server(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// The proxy username carrying every parameter, in the gateway's dialect.
    pub fn username(&self) -> String {
        let mut parts = Vec::with_capacity(self.params.len() + 1);
        parts.push(self.provider.render_prefix(&self.login));
        for (name, value) in &self.params {
            parts.push(format!(
                "{}{}{}",
                self.provider.spell(name),
                self.provider.pair_separator(),
                value
            ));
        }
        parts.join(self.provider.separator())
    }

    /// `http://user:pass@host:port`, for reqwest, ureq, curl and aiohttp.
    ///
    /// Both credentials are percent-encoded. A URL has structure, and the
    /// structure is made of the characters `: @ / ? #`; a password containing
    /// any of them cuts the string in the wrong place and the client connects
    /// somewhere else entirely. `pa/ss` unencoded ends the authority at the
    /// slash, so the host becomes `pa`.
    ///
    /// That failure is worse than an error, because at least one browser engine
    /// answers a proxy URL it cannot parse by connecting **directly** without
    /// reporting it - the request then leaves from your own address while
    /// everything downstream believes it went through the pool.
    ///
    /// Note the asymmetry with [`Proxy::browser`], which must not encode.
    pub fn url(&self) -> String {
        self.url_with_scheme("http")
    }

    /// [`Proxy::url`] with a scheme other than `http`.
    pub fn url_with_scheme(&self, scheme: &str) -> String {
        format!(
            "{scheme}://{}:{}@{}",
            percent_encode(&self.username()),
            percent_encode(&self.password),
            self.server()
        )
    }

    /// The three fields a browser driver wants, unencoded.
    ///
    /// Playwright, Patchright, Puppeteer and chromiumoxide all take the server,
    /// username and password separately and encode them themselves, so encoding
    /// here as well turns `pa/ss` into `pa%252Fss` and authentication fails
    /// while blaming the credentials.
    pub fn browser(&self) -> BrowserProxy {
        BrowserProxy {
            server: format!("http://{}", self.server()),
            username: self.username(),
            password: self.password.clone(),
        }
    }

    /// Open one CONNECT through this proxy and report what came back.
    ///
    /// The one call in this crate that touches the network. It sends the
    /// **built** username, so every parameter is under test and not just the
    /// credentials, and it opens a tunnel to [`crate::DEFAULT_TARGET`] - a real
    /// host, which will see a TCP connection from the exit address, because
    /// there is no null CONNECT.
    ///
    /// A refusal is a value and not an error: a 407 comes back as a [`Check`]
    /// carrying the status, the reason phrase, every header and this gateway's
    /// own explanation of what a 407 means. [`Error::Check`] is returned only
    /// when nothing came back at all.
    ///
    /// Use [`Proxy::connect`] to change the target or the timeout.
    ///
    /// # Errors
    ///
    /// [`Error::Check`] when the gateway was never reached: DNS failure, refused
    /// connection, timeout, or a first line that is not a status line.
    pub fn check(&self) -> Result<Check> {
        self.connect().send()
    }

    /// The same CONNECT, configured but not yet sent.
    ///
    /// Python spells this `proxy.check(target=..., timeout=...)`. Rust has no
    /// keyword arguments, so the two-method split is the faithful port and not a
    /// second API - the same reasoning that made [`Proxy::builder`] a builder.
    /// The returned [`Connect`] already carries this proxy's server, built
    /// username, password, exit-address header and whole reaction table, so
    /// nothing has to be re-derived and nothing about the gateway can be got
    /// wrong by hand.
    pub fn connect(&self) -> Connect {
        let mut connect = Connect::new(self.server(), self.username(), self.password.clone())
            .reactions(self.provider.connect_reactions().clone());
        if let Some(header) = self.provider.exit_ip_header() {
            connect = connect.exit_ip_header(header);
        }
        connect
    }

    /// A new identity with one parameter added or changed.
    ///
    /// This returns a new object rather than mutating, because on a gateway
    /// whose session key is the whole parameter set, changing a parameter is not
    /// an adjustment to one identity - it is a different identity on a different
    /// exit address. A method that returned `&mut self` would hide that.
    ///
    /// A parameter that is already set keeps its position in the username; a new
    /// one is appended.
    pub fn with_param(&self, name: &str, value: impl fmt::Display) -> Result<Proxy> {
        let mut params = self.params.clone();
        let value = value.to_string();
        match params.iter_mut().find(|(key, _)| key == name) {
            Some(pair) => pair.1 = value,
            None => params.push((name.to_string(), value)),
        }
        self.rebuilt(params)
    }

    /// A new identity with one parameter removed. A no-op if it was not set.
    pub fn without_param(&self, name: &str) -> Result<Proxy> {
        let mut params = self.params.clone();
        params.retain(|(key, _)| key != name);
        self.rebuilt(params)
    }

    /// The same parameters, pinned to one sticky session.
    ///
    /// A provider that declares no session parameter gets none, and the id is
    /// refused rather than sent under a guessed name: a name this gateway does
    /// not know is answered with 200 and ignored, so every request would draw a
    /// fresh exit while your code believed it was holding one.
    pub fn session(&self, session_id: &str) -> Result<Proxy> {
        let Some(name) = self.provider.session_param() else {
            return Err(Error::Param(format!(
                "{} declares no session parameter, so {session_id:?} cannot be sent and \
                 every request will leave from whichever exit the gateway picks.",
                self.provider.label()
            )));
        };
        self.with_param(name, session_id)
    }

    fn rebuilt(&self, params: Vec<(String, String)>) -> Result<Proxy> {
        Ok(Proxy {
            provider: self.provider.clone(),
            login: self.login.clone(),
            password: self.password.clone(),
            host: self.host.clone(),
            port: self.port,
            params: validate(&self.provider, params)?,
        })
    }
}

/// Never carries the password.
///
/// `{:?}` is what a panic message, a failing assertion and most logging setups
/// print, so a password in here reaches CI logs and pasted bug reports without
/// anyone ever choosing to print it. [`Proxy::url`] is the one place the secret
/// appears, and it is named so that it is obvious. There is deliberately no
/// `Display` impl: `{}` on a proxy would be the url, which is the leak.
impl fmt::Debug for Proxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut shown = f.debug_struct("Proxy");
        shown
            .field("provider", &self.provider.id())
            .field("server", &self.server())
            .field("login", &self.login)
            .field("password", &REDACTED);
        for (name, value) in &self.params {
            shown.field(name, value);
        }
        shown.finish()
    }
}

/// The proxy fields a browser driver takes, unencoded. See [`Proxy::browser`].
#[derive(Clone, PartialEq, Eq)]
pub struct BrowserProxy {
    /// `http://host:port`.
    pub server: String,
    /// The full username, parameters and all.
    pub username: String,
    /// The proxy password, verbatim.
    pub password: String,
}

impl fmt::Debug for BrowserProxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrowserProxy")
            .field("server", &self.server)
            .field("username", &self.username)
            .field("password", &REDACTED)
            .finish()
    }
}

/// Collects the parts of an identity. See [`Proxy::builder`].
pub struct ProxyBuilder {
    provider: Option<Provider>,
    login: Option<String>,
    password: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    params: Vec<(String, String)>,
}

impl ProxyBuilder {
    /// Build against a gateway other than the shipped default.
    pub fn provider(mut self, provider: Provider) -> Self {
        self.provider = Some(provider);
        self
    }

    /// The proxy username assigned by the gateway, before any parameters.
    pub fn login(mut self, login: impl Into<String>) -> Self {
        self.login = Some(login.into());
        self
    }

    /// The proxy password.
    pub fn password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// The gateway host, when it differs from the definition's default.
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = Some(host.into());
        self
    }

    /// The gateway port, when it differs from the definition's default.
    pub fn port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// One gateway parameter. Setting the same name twice replaces the value.
    ///
    /// The name is not checked here but at [`ProxyBuilder::build`], so a
    /// half-built chain never raises and the error names the whole identity.
    pub fn param(mut self, name: impl Into<String>, value: impl fmt::Display) -> Self {
        let name = name.into();
        let value = value.to_string();
        match self.params.iter_mut().find(|(key, _)| *key == name) {
            Some(pair) => pair.1 = value,
            None => self.params.push((name, value)),
        }
        self
    }

    /// Several gateway parameters, in the order given.
    pub fn params<I, K, V>(mut self, params: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: fmt::Display,
    {
        for (name, value) in params {
            self = self.param(name, value);
        }
        self
    }

    /// Resolve the credentials, check every parameter, and hand over the identity.
    pub fn build(self) -> Result<Proxy> {
        let provider = match self.provider {
            Some(provider) => provider,
            None => crate::load(DEFAULT_PROVIDER)?,
        };

        let env = provider.id().to_uppercase().replace('-', "_");
        let login = self
            .login
            .filter(not_empty)
            .or_else(|| from_env(&env, "LOGIN"));
        let password = self
            .password
            .filter(not_empty)
            .or_else(|| from_env(&env, "PASSWORD"));
        let host = self
            .host
            .filter(not_empty)
            .or_else(|| from_env(&env, "HOST"))
            .or_else(|| provider.host().map(str::to_string));
        // Through `check::port_number` and not `str::parse`, and the import
        // across modules is the point rather than a shortcut. That function
        // already holds the rule - ASCII digits only, at most five of them, 1 to
        // 65535 - with the three defects that produced it written down beside
        // it. A second `parse` here is a second implementation of the same rule,
        // and a rule with two implementations is what let this file ship the
        // missing `HTTP/` check that `check.py` also had.
        //
        // `parse::<u16>` is not that rule: measured 2026-09-08, it accepts a
        // leading `+`, so `NODEMAVEN_PORT=+8080` was taken as 8080, and it
        // accepts `0`, which then fell through to the address arm below and was
        // reported as a missing port.
        //
        // The sharp part is that **this crate already had the rule and already
        // had a test for it**. `check.rs` refuses `"+80"` in `split_address` and
        // `tests/check.rs` asserts it by name. So the defect was not a rule
        // nobody had thought about - it was one rule, correct, tested, with a
        // second uninstrumented copy sixty lines away. The `HTTP/` hole this
        // file shipped is the same failure with a language boundary in the
        // middle instead of a module boundary.
        let port = match self.port {
            Some(port) => Some(port),
            None => match from_env(&env, "PORT") {
                Some(text) => Some(check::port_number(&text).ok_or_else(|| {
                    Error::Credentials(format!(
                        "{env}_PORT is {text:?}, which is not a TCP port: it has to be \
                         a whole number from 1 to 65535, so nothing can connect. Unset \
                         it or pass port() to the builder."
                    ))
                })?),
                None => provider.port(),
            },
        };

        let (login, password) = match (login, password) {
            (Some(login), Some(password)) => (login, password),
            (login, password) => {
                let mut missing: Vec<&str> = Vec::with_capacity(2);
                if login.is_none() {
                    missing.push("login");
                }
                if password.is_none() {
                    missing.push("password");
                }
                return Err(Error::Credentials(format!(
                    "no {} for {}: pass it to the builder or set {env}_LOGIN and \
                     {env}_PASSWORD in the environment. Nothing was built, so nothing \
                     will connect.",
                    missing.join(" and no "),
                    provider.label()
                )));
            }
        };

        // Zero is checked on its own and before the address arm, because it is
        // the one value that arrives here *because the caller passed a port* and
        // the address arm's sentence is "pass port()". Found 2026-09-08 in an
        // external review of the Python SDK and present here in the same shape:
        // `port(0)` was filtered out by `port.filter(|port| *port != 0)` and then
        // reported as "no gateway address ... pass host() and port()", which
        // sends a caller to look at the one thing they got right. `port()` takes
        // a `u16`, so this is the only value the builder path can be wrong
        // about; the environment path is checked above.
        //
        // A third path reached this check until 2026-09-11 and the comment above
        // listed two, which is how it got here: a definition carrying `port = 0`
        // loaded, because `provider.rs` used `u16::try_from` and `try_from`
        // accepts zero, and then failed *here*, where every word is addressed to
        // somebody who called `port(0)`. The message read "The gateway's own
        // port is 0" - true, and it says the file is wrong while pointing the
        // reader at the builder. The definition's port now goes through
        // `check::port_number` at load, so by the time control is here the only
        // zero left is one the caller passed. Found in an external review of
        // this crate; the reachable conclusion is the general one, that a
        // comment enumerating the paths into a check goes stale the moment a
        // path is added and nothing makes it fail when it does.
        if port == Some(0) {
            return Err(Error::Credentials(format!(
                "port 0 is not a TCP port: it has to be a whole number from 1 to \
                 65535. Nothing was built. The gateway's own port is {}, and 0 is not \
                 a port at all - it means \"any free port\" when binding and is \
                 meaningless when connecting.",
                provider.port().map_or_else(
                    || "not in this definition".to_string(),
                    |port| port.to_string()
                )
            )));
        }

        let (Some(host), Some(port)) = (host, port) else {
            return Err(Error::Credentials(format!(
                "no gateway address for {}: pass host() and port() to the builder or \
                 set {env}_HOST and {env}_PORT.",
                provider.label()
            )));
        };

        let params = validate(&provider, self.params)?;
        Ok(Proxy {
            provider,
            login,
            password,
            host,
            port,
            params,
        })
    }
}

impl fmt::Debug for ProxyBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyBuilder")
            .field("provider", &self.provider.as_ref().map(Provider::id))
            .field("login", &self.login)
            .field("password", &self.password.as_ref().map(|_| REDACTED))
            .field("host", &self.host)
            .field("port", &self.port)
            .field("params", &self.params)
            .finish()
    }
}

/// Refuse client-side what the gateway will not report.
///
/// This is not politeness, it is the only check available. The gateway this
/// crate ships a definition for answers a value it will not take five different
/// ways and none of them names the parameter: a bad region gives 406, a bad city
/// 500, a bad isp 410, and a bad country, filter, ttl, type or speed value gives
/// 407 - which sends you to check credentials that are fine. An empty value
/// hangs the connection for about twenty seconds, and an unknown parameter name
/// is answered with **200 and the setting silently dropped**. That last one is
/// why this function exists: the request succeeds, and nothing that comes back
/// can tell you the setting was never applied. See `connect_reactions` in the
/// gateway definition for the sentence a caller is shown next to each code.
fn validate(provider: &Provider, params: Vec<(String, String)>) -> Result<Vec<(String, String)>> {
    let mut separators: Vec<&str> = Vec::with_capacity(2);
    for separator in [provider.separator(), provider.pair_separator()] {
        if !separator.is_empty() && !separators.contains(&separator) {
            separators.push(separator);
        }
    }
    separators.sort_unstable();

    let mut checked = Vec::with_capacity(params.len());
    for (name, value) in params {
        if !provider.known_params().contains(&name) {
            let known: Vec<&str> = provider.known_params().iter().map(String::as_str).collect();
            return Err(Error::Param(format!(
                "{} does not know the parameter {name:?}: it is answered with 200 and \
                 dropped, so the connection would succeed and your setting would NOT be \
                 applied. Known: {known:?}",
                provider.label()
            )));
        }

        // Fold before every remaining check, so that a refusal quotes the string
        // that would actually have gone on the wire rather than the one that was
        // typed. The folded value is what gets stored, so `params()`,
        // `username()` and the sticky-session identity all agree - a `Proxy` that
        // reported `New York` while sending `new_york` would make two callers
        // with the same visible configuration land on different exits.
        let value = provider.normalized(&name, &value);

        if value.is_empty() {
            return Err(Error::Param(format!(
                "empty value for {name:?}: the gateway does not reply to this, the \
                 connection hangs for about 20 s and then fails. Drop the parameter \
                 instead of passing an empty value."
            )));
        }

        // Whitespace inside a value, for a parameter nothing folds. There is no
        // form of this that can be right: a username is one token on the CONNECT
        // line, so the space either malforms the line or cuts the value short,
        // and `url()` would percent-encode it to `%20` while a browser driver
        // taking the fields separately would not - three spellings of one value,
        // at most one of which any gateway accepts. Refusing it is loud, and loud
        // beats a connection that succeeds with the setting quietly wrong.
        if let Some(bad) = value.chars().find(|c| ASCII_WHITESPACE.contains(c)) {
            let folded = provider
                .known_params()
                .iter()
                .filter(|known| provider.normalizes(known))
                .map(String::as_str)
                .collect::<Vec<&str>>();
            return Err(Error::Param(format!(
                "the value of {name:?} is {value:?} and contains whitespace ({bad:?}), \
                 which cannot be sent: a proxy username is a single token, so the value \
                 would be malformed or cut short. {} folds a space to an underscore for \
                 {folded:?} and for nothing else, so pass {name:?} without whitespace.",
                provider.label()
            )));
        }

        let bad: Vec<&str> = separators
            .iter()
            .copied()
            .filter(|separator| value.contains(separator))
            .collect();
        if let Some(first) = bad.first() {
            return Err(Error::Param(format!(
                "the value of {name:?} is {value:?} and contains {bad:?}, which {} uses to \
                 separate parameters. The username would be split into different settings \
                 than you asked for. Session ids in particular cannot carry a {first:?} - \
                 use {:?} or another character.",
                provider.label(),
                value.replace(first, "")
            )));
        }
        if let Some(allowed) = provider.allowed(&name) {
            if !allowed.contains(&value) {
                return Err(Error::Param(format!(
                    "{value:?} is not a value {} accepts for {name:?}. Allowed: {allowed:?}. \
                     This one is worth catching here because the gateway answers a bad value \
                     for some parameters with 407 Proxy Authentication Required, which reads \
                     as a credentials problem and is not one.",
                    provider.label()
                )));
            }
        }

        checked.push((name, value));
    }
    Ok(checked)
}

/// Percent-encode everything outside RFC 3986's unreserved set.
///
/// Hand-rolled rather than taken from `percent-encoding`, whose
/// `NON_ALPHANUMERIC` set also encodes `-`, `_`, `.` and `~`. The encoded form
/// is part of the cross-language contract - a golden vector pins the url a set
/// of parameters produces - so this set has to be the one Python's
/// `quote(text, safe="")` uses, byte for byte, rather than whichever set a
/// dependency happens to default to.
fn percent_encode(text: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            _ => {
                out.push('%');
                out.push(char::from(HEX[usize::from(byte >> 4)]));
                out.push(char::from(HEX[usize::from(byte & 0x0f)]));
            }
        }
    }
    out
}

fn from_env(env: &str, suffix: &str) -> Option<String> {
    std::env::var(format!("{env}_{suffix}"))
        .ok()
        .filter(not_empty)
}

// `&String` and not `&str`, against clippy's advice: this is only ever passed to
// `Option::<String>::filter`, which hands its predicate a `&String`, and the
// suggested signature does not compile there.
#[allow(clippy::ptr_arg)]
fn not_empty(value: &String) -> bool {
    !value.is_empty()
}
