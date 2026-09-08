//! One CONNECT, and what the gateway said about it.
//!
//! This is the only module in the crate that opens a socket, and it does it in
//! the smallest form there is: a raw `CONNECT` and the status line that comes
//! back. No TLS, no HTTP client, no target traffic, no dependency.
//!
//! The reason it is written by hand rather than delegated to an HTTP client is
//! that **the diagnosis is in the status line and clients throw it away.** The
//! Python SDK records the shape with `requests`, which reports a failed tunnel
//! as `ProxyError('Unable to connect to proxy', OSError('Tunnel connection
//! failed: 407 Proxy Authentication Required'))` - the code survives as text
//! inside a nested error and the reason phrase and every response header do
//! not. On this gateway the reason phrase identifies which back end answered
//! and one of the headers carries the exit address, so both are worth more than
//! the convenience of not writing this file.
//!
//! Like its Python original, this module depends on nothing in the crate except
//! [`Error`]. The reaction table is passed in as plain data rather than looked
//! up through a [`crate::Provider`], which is what makes it testable against a
//! socket on loopback and usable by anyone assembling a username by hand.
//!
//! # Where this diverges from the Python module, and why
//!
//! Everything here is below the golden vectors - they are keyed on the username
//! and the connection string - so none of it can invalidate a vector case. It is
//! written down because the next port will meet the same four questions.
//!
//! - **`headers` is a `Vec<(String, String)>` and not a map.** Python uses a
//!   `dict`, so a repeated header keeps its first position and its last value.
//!   Here every occurrence is kept, because a duplicated header is itself worth
//!   seeing when the question is which middlebox answered. [`Check::header`]
//!   returns the **last** match, so every lookup agrees with Python.
//! - **`elapsed` is a [`Duration`], not a float.** Python has one numeric type
//!   for it and Rust has a right one.
//! - **A zero timeout is refused before anything is sent.** Python hands it to
//!   the socket layer, which puts the socket in non-blocking mode and fails with
//!   whatever the platform says. Here it would be an `InvalidInput` from
//!   [`TcpStream::connect_timeout`], which reads as a bug in this crate.
//! - **Header names are lower-cased ASCII-only.** `str::to_lowercase` and
//!   Python's `str.lower()` are Unicode-aware and the head is decoded latin-1,
//!   so a byte like `0xC0` in a field name would produce two different keys in
//!   two SDKs. RFC 9110 says a field name is a token and a token is ASCII, so
//!   ASCII-only is both the portable rule and the correct one. The Python module
//!   was changed to match on 2026-09-07 rather than this one being bent to it.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};

/// What a CONNECT is opened *to*.
///
/// The gateway has to be asked for some target - there is no null CONNECT - so
/// this is a real host that will see a TCP connection from the exit address. It
/// is a parameter on every entry point and this is only the default.
///
/// Port 443 and not 80 because a gateway may treat plaintext differently, and
/// because 443 is what a caller's real traffic will use.
pub const DEFAULT_TARGET: &str = "api.ipify.org:443";

/// How long a [`Connect`] waits, when nothing says otherwise.
///
/// Fifteen seconds, and not something small, because one of this gateway's
/// measured reactions is *no reply for about 20 s* on an empty parameter value
/// (2026-08-10). A 5 s default would report that as a network problem, which is a wrong
/// diagnosis produced by our own default. This crate refuses empty values before
/// sending, so the case should be unreachable through [`crate::Proxy`]; the
/// default is set for the caller who assembles a username by hand.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// What this client *sends*. Reading is deliberately laxer - see [`head_end`] -
/// because being strict about what you send and liberal about what you accept
/// are the same rule, not opposite ones.
const CRLF: &str = "\r\n";

/// The response head is read up to here and no further.
///
/// A head is a few hundred bytes. An unbounded read is a memory exhaustion bug
/// waiting for a gateway that answers with a stream.
const MAX_HEAD: usize = 16384;

/// Stands in for the password wherever a [`Connect`] is printed.
///
/// Deliberately a second copy of the constant in `proxy.rs` rather than a shared
/// one. This module depends on nothing in the crate but [`Error`], which is what
/// lets it be tested and used on its own, and a shared constant would buy
/// nothing for the cost of that.
const REDACTED: &str = "***";

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// What one CONNECT produced.
///
/// A refusal is a **result and not an error**: the status code is the thing the
/// caller came for, and returning `Err` would push the useful part somewhere it
/// has to be unwrapped from. [`Error::Check`] is reserved for the cases where
/// nothing came back at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    status: u16,
    reason: String,
    server: String,
    elapsed: Duration,
    headers: Vec<(String, String)>,
    exit_ip: Option<String>,
    meaning: Option<String>,
}

impl Check {
    /// The CONNECT status, e.g. 200 or 407.
    ///
    /// A `u16` and not a wider type because RFC 9110 calls the status code a
    /// three-digit integer, and this module refuses a first line that does not
    /// carry exactly three ASCII digits. That refusal is what makes the narrow
    /// type safe, and it is also why the Python module refuses the same thing:
    /// without it, `99999` parses there and cannot be represented here, which is
    /// a port that disagrees with its original on a real input.
    pub fn status(&self) -> u16 {
        self.status
    }

    /// The reason phrase, verbatim and not normalised - not even for case.
    ///
    /// On the shipped gateway this identifies which back end answered: measured
    /// 2026-08-13, a 200 carrying `X-Proxy-Exit-IP` arrives as
    /// `Connection established`, while the ones that arrive as `OK` or
    /// `Connection Established` do not carry it. Any per-implementation number
    /// has to be split on this rather than pooled, so it is preserved byte for
    /// byte.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// `host:port` of the gateway, with no credentials in it.
    pub fn server(&self) -> &str {
        &self.server
    }

    /// From the first byte sent to the status line parsed.
    ///
    /// Includes DNS and the TCP handshake, and is not comparable against a
    /// number measured on a different network path.
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Every response header, names lower-cased, in the order they arrived.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// One header by name, case-insensitively, or `None`.
    ///
    /// The **last** occurrence wins, which is what a Python `dict` built by
    /// assignment does and therefore what the Python SDK returns. A gateway that
    /// sends a header twice is unusual enough that both are kept in
    /// [`Check::headers`]; this is the lookup, so it agrees with the original.
    pub fn header(&self, name: &str) -> Option<&str> {
        let wanted = name.to_ascii_lowercase();
        self.headers
            .iter()
            .rev()
            .find(|(header, _)| *header == wanted)
            .map(|(_, value)| value.as_str())
    }

    /// The exit address, when the gateway sent one.
    ///
    /// Read off whatever header the provider definition declares. `None` is
    /// normal rather than an error - on the shipped gateway only one of at least
    /// three back ends sends it, which is the same measurement that made
    /// [`Check::reason`] worth keeping verbatim.
    pub fn exit_ip(&self) -> Option<&str> {
        self.exit_ip.as_deref()
    }

    /// What this status means on this gateway, or `None` if unrecorded.
    ///
    /// This is where a 407 gets told not to go and check its password. The text
    /// comes from the provider definition's `connect_reactions` table, because
    /// it is dialect rather than HTTP.
    pub fn meaning(&self) -> Option<&str> {
        self.meaning.as_deref()
    }

    /// Whether the tunnel opened.
    ///
    /// True means the gateway accepted the credentials and every parameter it
    /// recognised. It does **not** mean every parameter was applied: an
    /// unrecognised name is answered 200 and dropped. That is why this crate
    /// refuses unknown names before sending, and why `ok` cannot be the whole
    /// answer on its own.
    pub fn ok(&self) -> bool {
        self.status == 200
    }
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} via {} in {:.2}s",
            self.status,
            self.reason,
            self.server,
            self.elapsed.as_secs_f64()
        )?;
        // `filter` on emptiness in both clauses, because Python's `if self.exit_ip`
        // and `if self.meaning` are false for an empty string and `Some("")` is
        // not. A blank explanation is refused when a provider is loaded, so this
        // can only arrive from a hand-built `Connect`; a blank `X-Proxy-Exit-IP`
        // needs nothing but a gateway sending the header with no value.
        if let Some(exit_ip) = self.exit_ip.as_deref().filter(|ip| !ip.is_empty()) {
            write!(f, ", exit {exit_ip}")?;
        }
        if let Some(meaning) = self.meaning.as_deref().filter(|text| !text.is_empty()) {
            if !self.ok() {
                write!(f, "\n{meaning}")?;
            }
        }
        Ok(())
    }
}

/// One CONNECT, configured but not yet sent.
///
/// Rust has no keyword arguments, so where Python offers
/// `connect(server, username, password, *, target=..., timeout=...)` this is a
/// builder - the same reasoning that makes [`crate::Proxy::builder`] one. The
/// defaults are identical to Python's and nothing has to be set:
/// [`Connect::new`] followed by [`Connect::send`] is the whole call.
///
/// `send` takes `&self`, so one of these can be sent more than once. That is
/// deliberate: the honest way to ask "did that 407 stick" is to repeat the same
/// request, and rebuilding it by hand invites changing something by accident.
/// Nothing here retries on its own - see the crate documentation for the
/// measurement behind that.
#[derive(Clone)]
pub struct Connect {
    server: String,
    username: String,
    password: String,
    target: String,
    timeout: Duration,
    exit_ip_header: Option<String>,
    reactions: BTreeMap<String, String>,
}

// Hand-written because the derive would print the password. `Proxy` redacts the
// same way and for the same reason: a `Proxy-Authorization` header is base64 and
// not encryption, so anything that prints one has put a working credential into
// a terminal history, a CI log and whatever bug report gets pasted next.
impl fmt::Debug for Connect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connect")
            .field("server", &self.server)
            .field("username", &self.username)
            .field("password", &REDACTED)
            .field("target", &self.target)
            .field("timeout", &self.timeout)
            .field("exit_ip_header", &self.exit_ip_header)
            .field("reactions", &self.reactions.len())
            .finish()
    }
}

impl Connect {
    /// A CONNECT through `server` as `username`, with everything else defaulted.
    ///
    /// `server` is `host:port` and carries no credentials. `username` is the
    /// whole built username, not the bare login - through [`crate::Proxy`] that
    /// is [`crate::Proxy::username`] and the parameters are already in it.
    pub fn new(
        server: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Connect {
            server: server.into(),
            username: username.into(),
            password: password.into(),
            target: DEFAULT_TARGET.to_string(),
            timeout: DEFAULT_TIMEOUT,
            exit_ip_header: None,
            reactions: BTreeMap::new(),
        }
    }

    /// What to open the tunnel to. Defaults to [`DEFAULT_TARGET`].
    ///
    /// Whatever is named here sees a real TCP connection from the exit address,
    /// so it is a parameter rather than a constant.
    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target = target.into();
        self
    }

    /// How long to wait. Defaults to [`DEFAULT_TIMEOUT`].
    ///
    /// It bounds each operation - resolve, connect, write, each read - and not
    /// the call as a whole, which is what Python's `socket` module does and what
    /// this deliberately copies. A total deadline would be the better API and a
    /// worse port; if it is ever wanted, it belongs in both SDKs on the same day.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Which response header carries the exit address on this gateway.
    ///
    /// Per-gateway dialect, so it comes from the provider definition rather than
    /// being guessed here. Unset means [`Check::exit_ip`] is always `None`.
    pub fn exit_ip_header(mut self, header: impl Into<String>) -> Self {
        self.exit_ip_header = Some(header.into());
        self
    }

    /// The provider's whole `connect_reactions` table.
    ///
    /// Passed **whole** and not resolved at the call site, and that is the one
    /// thing about this signature that is easy to get wrong: the meaning depends
    /// on the status, and the status is not known until the answer comes back.
    /// A caller who looked one up and passed a single `Option<&str>` would always
    /// pass `None`, the table would go unread, and nothing else about the call
    /// would change. The Python SDK pins that with a regression test.
    pub fn reactions(mut self, reactions: BTreeMap<String, String>) -> Self {
        self.reactions = reactions;
        self
    }

    /// What one status means, for a caller who has no [`crate::Provider`].
    ///
    /// Keys are stored as strings because that is what the file and the golden
    /// vectors use - TOML has no integer keys and neither does JSON - so the
    /// `u16` is converted here, exactly as [`crate::Provider::reaction`] does it.
    pub fn reaction(mut self, status: u16, meaning: impl Into<String>) -> Self {
        self.reactions.insert(status.to_string(), meaning.into());
        self
    }

    /// Open the tunnel and report what came back.
    ///
    /// `username` and `password` go into a `Proxy-Authorization` header. **That
    /// header is never logged, never put in an error message and never
    /// returned.**
    ///
    /// # Errors
    ///
    /// [`Error::Check`] in two situations. Before anything is sent, when an
    /// argument cannot make a well-formed request - a zero timeout, an address
    /// that is not `host:port`, a target that is not one token - and those
    /// messages end in *Nothing was sent*. After sending, when there was no
    /// usable answer: DNS failure, refused connection, timeout, a head that
    /// never ended, or a first line that is not a status line. A gateway that
    /// refuses the tunnel answered the question, so it comes back as `Ok` with
    /// the status in it.
    pub fn send(&self) -> Result<Check> {
        if self.timeout.is_zero() {
            return Err(Error::Check(format!(
                "the timeout for {} is zero, so there is no time in which to \
                 answer. Nothing was sent.",
                self.server
            )));
        }
        let (host, port) = split_address(&self.server)?;
        if !is_request_target(&self.target) {
            return Err(Error::Check(format!(
                "{:?} cannot go in a request line: a target is visible ASCII \
                 with no spaces, so that it lands in the CONNECT as one token. \
                 Nothing was sent.",
                self.target
            )));
        }

        let token = base64(format!("{}:{}", self.username, self.password).as_bytes());
        let request = format!(
            "CONNECT {target} HTTP/1.1{CRLF}\
             Host: {target}{CRLF}\
             Proxy-Authorization: Basic {token}{CRLF}\
             Proxy-Connection: close{CRLF}\
             {CRLF}",
            target = self.target,
        );

        let started = Instant::now();
        // `request` holds the credential and is in scope for everything below.
        // Every error from here on is built by `no_answer`, which takes the
        // server and the io error and nothing else - the one discipline that
        // keeps a base64 credential out of a traceback.
        let mut stream = open(&self.server, host, port, self.timeout)?;
        stream
            .set_read_timeout(Some(self.timeout))
            .and_then(|()| stream.set_write_timeout(Some(self.timeout)))
            .and_then(|()| stream.write_all(request.as_bytes()))
            .map_err(|error| no_answer(&self.server, &error))?;
        let head = read_head(&mut stream, &self.server)?;
        let elapsed = started.elapsed();
        drop(stream);

        let (status, reason, headers) = parse_head(&head, &self.server)?;
        let exit_ip = self.exit_ip_header.as_deref().and_then(|name| {
            let wanted = name.to_ascii_lowercase();
            headers
                .iter()
                .rev()
                .find(|(header, _)| *header == wanted)
                .map(|(_, value)| value.clone())
        });
        let meaning = self.reactions.get(&status.to_string()).cloned();
        Ok(Check {
            status,
            reason,
            server: self.server.clone(),
            elapsed,
            headers,
            exit_ip,
            meaning,
        })
    }
}

/// Open a TCP connection to every address `host` resolves to, in turn.
///
/// [`TcpStream::connect`] has no timeout and blocks on the platform default,
/// which on Windows is around 21 seconds and on Linux longer - so a caller who
/// asked for 2 s would wait ten times that and be told it timed out. The version
/// that takes a deadline needs a resolved [`std::net::SocketAddr`], which is why
/// resolution is spelled out here rather than left to `connect`.
///
/// Resolution itself is unbounded. `getaddrinfo` takes no deadline in any
/// standard library, Python's `create_connection` has the same hole, and pretending
/// otherwise would need a thread. Said out loud because "the timeout did not
/// hold" is otherwise a puzzling bug report.
fn open(server: &str, host: &str, port: u16, timeout: Duration) -> Result<TcpStream> {
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| no_answer(server, &error))?;
    let mut last = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) => last = Some(error),
        }
    }
    match last {
        Some(error) => Err(no_answer(server, &error)),
        None => Err(Error::Check(format!(
            "{server} resolved to no addresses at all, so there was nothing to \
             connect to. Nothing here can tell you whether the credentials are \
             right, because the gateway was never reached."
        ))),
    }
}

/// Read up to and including the blank line that ends the response head.
///
/// Reads no further, so nothing is consumed from the tunnel body even when the
/// tunnel opened, and any body bytes that arrived in the same segment are cut
/// off rather than parsed as headers.
///
/// **An unterminated head is refused rather than returned.** A peer that sends a
/// status line and hangs up mid-head would otherwise parse as a complete answer:
/// `status=200`, `ok=true` and an empty header list, with the header line in the
/// bytes and [`parse_head`] dropping it, because a header only enters the table
/// when the blank line proves it arrived whole. So the caller is told the tunnel
/// opened and the exit address has silently gone missing.
///
/// Nothing at all is still returned as an empty head, because [`parse_head`] has
/// the sentence for it: an immediate close is a documented reaction of the
/// shipped gateway and a truncated head is not.
fn read_head(stream: &mut TcpStream, server: &str) -> Result<Vec<u8>> {
    let mut head = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(end) = head_end(&head) {
            head.truncate(end);
            return Ok(head);
        }
        if head.len() > MAX_HEAD {
            return Err(Error::Check(format!(
                "{server} sent {} bytes with no end to the response head. A \
                 head is a few hundred bytes, so this is a stream and not a \
                 reply, and reading further is how a client runs out of memory.",
                head.len()
            )));
        }
        match stream.read(&mut chunk) {
            Ok(0) if head.is_empty() => return Ok(head),
            Ok(0) => {
                return Err(Error::Check(format!(
                    "{server} closed the connection after {} bytes, part-way \
                     through the response head. Whatever came before the cut is \
                     not an answer - a status line without its blank line may be \
                     missing headers that had not arrived yet.",
                    head.len()
                )))
            }
            Ok(read) => head.extend_from_slice(&chunk[..read]),
            // Python's `recv` retries this itself - PEP 475 made every blocking
            // call in the standard library restart on EINTR - and Rust's does
            // not. Without this a profiler's signal, or a resized terminal,
            // surfaces as "no answer from the gateway".
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(no_answer(server, &error)),
        }
    }
}

/// Index just past the blank line that ends a response head, or `None`.
///
/// A blank line has four spellings once a bare LF is allowed as a terminator -
/// `\r\n\r\n`, `\n\n`, `\r\n\n` and `\n\r\n` - and searching for the earlier of
/// `\n\n` and `\n\r\n` covers all four, because every one of them ends in one of
/// those two.
///
/// LF is accepted because RFC 9112 section 2.2 says a recipient may recognise a
/// single LF as a line terminator and ignore any preceding CR, and because this
/// gateway needs it: a 200 is framed CRLF and every refusal - 406, 407, 500 - is
/// framed with bare LF. A CRLF-only reader cannot report any refusal code from
/// it, which is the one thing this module exists to do.
fn head_end(head: &[u8]) -> Option<usize> {
    let mut end: Option<usize> = None;
    for terminator in [b"\n\n".as_slice(), b"\n\r\n".as_slice()] {
        let found = head
            .windows(terminator.len())
            .position(|window| window == terminator);
        if let Some(found) = found {
            let candidate = found + terminator.len();
            if end.is_none_or(|current| candidate < current) {
                end = Some(candidate);
            }
        }
    }
    end
}

/// Split a response head into status, reason phrase and headers.
///
/// A status line is accepted only when its **first two tokens** are an HTTP
/// version and a three-digit status code, per [`is_http_version`] and
/// [`is_status_code`]. Only the second was checked until 2026-09-08, which is how
/// `garbage 200 OK` parsed as a 200.
///
/// Lines are split on LF with one optional preceding CR stripped, which accepts
/// a head framed either way for the reason given in [`head_end`]. A CR anywhere
/// else in a line is left alone: it is part of the value, and this function does
/// not repair a malformed reply.
///
/// Decoded as **latin-1 and never as utf-8**. A header value is bytes by
/// specification and a gateway is free to put anything in a reason phrase;
/// `String::from_utf8` is the wrong call here and `String::from_utf8_lossy` is
/// worse, because it replaces the byte with U+FFFD and the caller cannot tell
/// what arrived. latin-1 cannot fail, is reversible, and the reason phrase is
/// something a human reads rather than something this crate matches on.
#[allow(clippy::type_complexity)]
fn parse_head(head: &[u8], server: &str) -> Result<(u16, String, Vec<(String, String)>)> {
    if head.is_empty() {
        return Err(Error::Check(format!(
            "{server} accepted the connection and then closed it without \
             answering. That is not one of the reactions this gateway is known \
             to have, so it is worth reporting with the parameters that produced \
             it."
        )));
    }
    let text: String = head.iter().map(|&byte| char::from(byte)).collect();
    let mut lines = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line));
    let first = lines.next().unwrap_or_default();

    let mut parts = first.splitn(3, ' ');
    let version = parts.next().unwrap_or_default();
    let code = parts.next().unwrap_or_default();
    if !is_http_version(version) || !is_status_code(code) {
        return Err(Error::Check(format!(
            "{server} answered {first:?}, which is not an HTTP status line. \
             Either something other than a proxy is listening on that port, or \
             a middlebox answered instead of the gateway."
        )));
    }
    // Infallible: three ASCII digits is at most 999.
    let status: u16 = code.parse().unwrap_or_default();
    let reason = parts.next().unwrap_or_default().to_string();

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    Ok((status, reason, headers))
}

/// Whether `text` is one or more of `0`-`9` and nothing else.
///
/// Spelled out rather than `char::is_ascii_digit` on an iterator, or a `parse`
/// that decides for itself - `u16::from_str` accepts a leading `+` and Python's
/// `int` accepts leading whitespace, an underscore separator and a Unicode minus,
/// so neither language's built-in parser is the rule. The rule is this one, in
/// four SDKs, and it is written as a comparison so it ports without argument.
fn is_ascii_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// Whether `text` is an HTTP status code: exactly three ASCII digits.
///
/// Three and not "one or more" because RFC 9110 says so, and because it is the
/// rule that lets [`Check::status`] be a `u16` without diverging from Python's
/// arbitrary-precision `int`. `Python`'s obvious spelling for this was
/// `str.isdigit()`, which was a defect found while writing this file and fixed
/// on 2026-09-07: it is True for `'\u{b2}'`, where `int()` raises, and this
/// function is reachable with exactly that character because the head above is
/// decoded latin-1 on purpose. Rust has the same trap under a different name -
/// `char::is_numeric` is True for the same character - which is why neither
/// SDK asks a Unicode question here.
fn is_status_code(text: &str) -> bool {
    text.len() == 3 && is_ascii_digits(text)
}

/// Whether `text` is an HTTP version token: `HTTP/` and two digits.
///
/// RFC 9112 section 2.3 spells it `HTTP-name "/" DIGIT "." DIGIT` and makes
/// `HTTP` case-sensitive, so this is the grammar and not a house rule. Eight
/// bytes exactly, because a CONNECT answered over a socket this module opened
/// itself is HTTP/1.x by construction - there is no version negotiation here to
/// be liberal about.
///
/// It exists because it was missing, found 2026-09-08 in an external review of
/// both SDKs and reproduced the same day: [`parse_head`] split the status line
/// and looked only at the *second* token, so the first was accepted whatever it
/// was. `garbage 200 OK` came back as a 200 with `ok` true, and carrying an exit
/// address if whatever was listening also sent the header.
///
/// **What this cost is the argument for writing a rule down rather than porting
/// one.** Three digits, ASCII digits and ASCII lower-casing are each a paragraph
/// in the Python SDK and each landed here intact. The version check was never
/// written down anywhere, so this file reproduced the hole line for line - the
/// discarded token was even spelled `let _version = parts.next();`, deliberate,
/// reviewed by nobody. A cross-language contract is only the part that was
/// written down; what is left implicit gets re-derived by hand in every port,
/// and re-derived the same way, because the same reading produced it.
fn is_http_version(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 8
        && &bytes[..5] == b"HTTP/"
        && bytes[5].is_ascii_digit()
        && bytes[6] == b'.'
        && bytes[7].is_ascii_digit()
}

/// Whether `text` can go into a request line as a single token.
///
/// Visible ASCII with no space: bytes 0x21 to 0x7E. Deliberately narrower than
/// the characters that hurt, because the request line is assembled by
/// interpolation and the set of bytes that change its shape is not something to
/// enumerate from memory.
///
/// The case that motivated it, measured 2026-09-08 in the Python SDK, which
/// builds the same request the same way: a target of
/// `"example.com:443\r\nX-Injected: yes"` produced a request line of
/// `CONNECT example.com:443` with **no version token at all**, and
/// `X-Injected: yes HTTP/1.1` as a header of our own request. Header injection is
/// the obvious half; the request line losing its version to a caller's string is
/// the half that is easy to miss.
///
/// Two things it deliberately does not do. It does not check that the target is
/// `host:port` - the gateway is entitled to its own opinion about what it will
/// tunnel to, and a client that refuses what the server would have accepted is a
/// client that has to be worked around. And it does not touch the username or the
/// password, which cannot inject anything: they go through [`base64`], whose
/// output alphabet is `A-Za-z0-9+/=` and holds neither CR nor LF. Both SDKs
/// validating what gets base64-encoded while leaving the one field that lands in
/// the clear unchecked was the actual shape of this defect.
fn is_request_target(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte > b' ' && byte < 0x7f)
}

/// Split `host:port`, or say why it is not one.
///
/// Splits on the **last** colon, so an IPv6 literal in brackets works and a bare
/// one does not - which matches Python's `rpartition` and is the same behaviour
/// as every proxy configuration string this crate produces.
fn split_address(server: &str) -> Result<(&str, u16)> {
    let refused = || {
        Error::Check(format!(
            "{server:?} is not a gateway address: it has to be host:port, with \
             a port from 1 to 65535. Nothing was sent."
        ))
    };
    let (host, port_text) = server.rsplit_once(':').ok_or_else(refused)?;
    if host.is_empty() {
        return Err(refused());
    }
    port_number(port_text)
        .map(|port| (host, port))
        .ok_or_else(refused)
}

/// `text` as a TCP port, or `None` if it is not one.
///
/// `pub(crate)` and not private, because [`ProxyBuilder::build`] needs the same
/// rule and a second implementation of it is how the two drift. That is not a
/// guess about what could happen: this crate shipped `let _version =
/// parts.next();` in [`parse_head`] because the `HTTP/` check lived only in the
/// Python SDK's prose and was never written as a rule with one home. The import
/// across modules is the point rather than a shortcut.
///
/// [`Proxy`] takes a `u16` from the builder, so only zero can be wrong on that
/// path; the environment path takes text and meets every case below.
///
/// Three things are refused and each was a defect somewhere before it was a
/// rule:
///
/// * **Not ASCII digits.** `u16::from_str` accepts a leading `+`, so `"+80"`
///   would parse. Python's `str.isdigit()` is True for `'\u{b2}'`, where `int()`
///   raises - the same trap under a different name, measured 2026-09-07.
/// * **More than five characters.** In Python `int()` succeeds on any length and
///   the `OverflowError` that follows derives from `ArithmeticError`, not
///   `OSError`, so it walks past the handler that turns socket problems into a
///   `CheckError`. Rust's `parse::<u16>` fails cleanly, but the rule is shared
///   across four SDKs and is stated once rather than per language.
/// * **Zero.** Legal in `bind`, where it means "any free port", and meaningless
///   in `connect`: Windows answers WinError 10049 and Linux ECONNREFUSED.
///   Refusing it replaces a platform-specific errno with a sentence naming the
///   mistake.
pub(crate) fn port_number(text: &str) -> Option<u16> {
    if !is_ascii_digits(text) || text.len() > 5 {
        return None;
    }
    match text.parse::<u16>() {
        Ok(port) if port > 0 => Some(port),
        _ => None,
    }
}

/// Standard base64 with padding, over bytes.
///
/// Hand-rolled for the same reason `percent_encode` is: this is 20 lines and a
/// dependency here would be a dependency in the manifest of a crate whose whole
/// claim is that it has one. The output is pinned against the Python SDK's
/// `base64.b64encode` by a test, because the header it goes into is the one
/// thing on the wire that has to be byte-identical across four SDKs.
fn base64(input: &[u8]) -> String {
    let mut encoded = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let triple = (u32::from(chunk[0]) << 16)
            | (u32::from(chunk.get(1).copied().unwrap_or(0)) << 8)
            | u32::from(chunk.get(2).copied().unwrap_or(0));
        encoded.push(char::from(BASE64[(triple >> 18 & 63) as usize]));
        encoded.push(char::from(BASE64[(triple >> 12 & 63) as usize]));
        encoded.push(if chunk.len() > 1 {
            char::from(BASE64[(triple >> 6 & 63) as usize])
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            char::from(BASE64[(triple & 63) as usize])
        } else {
            '='
        });
    }
    encoded
}

/// The one error message shape for "the gateway was never reached".
///
/// Takes the server and the io error and nothing else, on purpose. The caller
/// has the request bytes in scope, those bytes contain the credential, and a
/// format string that reached for them would put a working `Proxy-Authorization`
/// header into every backtrace, CI log and pasted bug report.
fn no_answer(server: &str, error: &std::io::Error) -> Error {
    Error::Check(format!(
        "no answer from {server}: {error}. Nothing here can tell you whether \
         the credentials are right, because the gateway was never reached."
    ))
}
