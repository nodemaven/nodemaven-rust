# Changelog

Written against the source tree and against the run that produced each number,
not from memory.

A note on how entries here are worded, because it is the rule the crate itself
is built on: a claim about what the gateway accepts carries the probe that
established it and the date it was run. "The vendor's own documentation says so"
is not one of those, and an entry resting on it says so outright.

## 0.1.0 - unreleased

**The date on this heading is left blank on purpose and goes in when
`cargo publish` runs.** crates.io records a publication time; this line should
be that time rather than the day the section was written, and the two are only
the same by luck.

First release, so every entry below is an addition and the list is what the
crate holds rather than what changed. The part worth reading is the last
section: the decisions that the public API does not show.

### The crate

`Proxy` builds the username a proxy gateway expects, refuses input that gateway
would silently mishandle, and hands back a URL for whatever HTTP client you
already use. It holds no session and retries nothing. One call reaches the
network, by name and never on construction.

- **`Proxy`, `ProxyBuilder`, `BrowserProxy`.** `username()`, `server()`,
  `url()`, `url_with_scheme()`, and `browser()` for clients that want the host
  and the credentials as separate fields. `with_param()`, `without_param()` and
  `session()` return a new `Proxy` rather than mutating one, so a derived
  identity cannot change the one it came from.
- **`Provider`, `ProviderBuilder`, `available()`, `load()`, `load_file()`,
  `load_file_as()`, `load_str()`, `DEFAULT_PROVIDER`, `ASCII_WHITESPACE`.** A
  gateway is a data description, not a code path: one that ships no definition
  here goes through the same builder and the same validation.

  A provider with no `known_params` is not a stub. It refuses every parameter,
  because the alternative is sending a name to a gateway that answers 200 and
  drops it.
- **`Check`, `Connect`, `DEFAULT_TARGET`, `DEFAULT_TIMEOUT`.** One CONNECT to
  the gateway, the reply read and handed back whole: status, the reason phrase
  verbatim, every header, elapsed time, the exit address when the gateway sends
  one, and the gateway's own explanation of that status.

  **A refusal is a return value, not an error.** A 407 is the gateway answering
  the question, so it comes back as a `Check`. `Error` is returned only when
  nothing came back - DNS, a refused connection, a timeout, or a first line that
  is not a status line.
- **`Error`, `Result`.** Every refusal names the parameter and the value it
  refused.

### What the gateway definition carries

`src/data/providers/nodemaven.toml` is `status = "measured"`: every value in it
was read off the wire by raw CONNECT probe on the date recorded in the file,
rather than copied out of documentation. It is **byte-identical to the Python
SDK's copy**, checked with `cmp` on 2026-09-08, so the two libraries cannot
disagree about what the gateway accepts.

Two findings in it are worth naming here, because they are the reason the
validation exists at all:

- **An unrecognised parameter name is answered 200 and dropped.** So "the
  request succeeded" does not mean "the settings were applied", and a typo in a
  parameter name is invisible at the protocol level. This crate refuses unknown
  names before sending.
- **A bad value is answered five different ways and not one of them names the
  parameter.** 407 on `country`, `filter`, `ttl`, `type` and `speed`; 406 on
  `region`, `city` and `isp`; 410 on one ISP; 500 when `city` is sent without
  its `region`; and an empty value gets no reply at all for about 20 seconds.
  The 407 is the expensive one - it sends you to check credentials that are
  correct. `check()` prints the gateway's own sentence next to the code for
  exactly this reason.

`norotate` is **not** in `known_params`, and the round trip is recorded in the
definition file rather than quietly dropped. It was added on 2026-08-21 because
the vendor's own proxy generator emits it, and removed on 2026-08-26 after a
probe from the VPS with a negative control: the gateway answers a junk value 200
exactly as it answers a name nobody has implemented, and rotation is unchanged
in both the no-session and the fixed-session arms. A source that looks
authoritative is not a measurement.

### Decisions that the public API does not show

- **`repository` and `documentation` are absent from the manifest**, and this
  is the one entry here with a known cost attached. crates.io bakes the manifest
  into each published version and never re-reads it, so both keys arrive with
  the release that follows, never with a later edit to this file. The cost is
  measured rather than assumed: the Python package has been on PyPI for three
  versions with no `Repository` link on its page for precisely this reason, and
  making the source repository public did not change that page by one character.
- **The manifest carries an `include` allowlist rather than an `exclude` list.**
  `cargo publish` is permanent - `cargo yank` withdraws a version from
  dependency resolution and deletes nothing - so anything that goes out once
  stays out. An `exclude` list fixes the instance and not the class: a new
  scratch file is included by default and silently, while a new source file that
  an allowlist misses fails loudly at the verification build.
- **No badges in the README yet.** A crates.io version badge, a docs.rs badge
  and a CI badge all render "not found" until the crate is published, so they go
  in with the publishing commit and not before. This is the same rule as the
  first entry, applied to a surface that fails visibly instead of silently.
- **There is no retry policy, and that is the design.** Retrying a refused
  request is the one thing that reliably makes the next one worse: measured over
  1464 attempts, the chance the next attempt succeeds falls from 75% with no
  prior failure to 5.8% after five consecutive failures and 0.5% after seven,
  and the 294 attempts spent past six consecutive failures returned three pages
  - 98 attempts per delivered page against 1.7 in a healthy session. A default
  that hid that would be spending a shared pool's reputation on the caller's
  behalf.
- **`ttl` values are not case-folded, and the caller is told instead.** `10M` is
  refused where `10m` is accepted, and it is the only value on this gateway
  whose case is known to matter. Folding it would encode which spellings the
  gateway takes today into a library that cannot re-measure them.
- **`sid` is never rewritten.** It is an identifier the caller chose, and an SDK
  that normalises it renames an identity behind the caller's back. Whether this
  gateway treats `Order4417` and `order4417` as one session or two has not been
  probed, and the definition file says so rather than asserting either.
- **`#![forbid(unsafe_code)]`, and one dependency.** `toml` 1.1 with default
  features off, keeping `std`, `parse` and `serde`: nothing here emits TOML, so
  the writer half would be dead weight.
- **Two gaps, stated rather than left to be discovered.** There is no account
  API here, so unlike the Python SDK this crate cannot check a `city` against
  the catalogue before sending, and a `city` without its `region` reaches the
  gateway and comes back 500. And `url_with_scheme()` will build a `socks5://`
  URL, but **nothing has probed whether this gateway speaks SOCKS5 at all** -
  the whole dialect was read off HTTP CONNECT probes on port 8080.

### Verification

140 tests pass: 61 in `tests/check.rs`, 68 in `tests/proxy.rs`, 8 in
`tests/readme.rs`, 1 in `tests/env.rs`, and 2 doctests. Run on 2026-09-08 with
cargo 1.98.0 and rustc 1.98.0 on Windows.

`tests/readme.rs` is not a formality: it asserts that every exported name
appears in the README, that every public call is in the reference table, that
every internal anchor points at a heading that exists, and that the `ttl` values
the README names are the ones the provider definition names. A README that
drifts from the code fails the build.

**No test here touches the network.** The CONNECT reader is exercised against
recorded response heads, which is also how the bare-LF framing case is covered -
a 200 from this gateway is CRLF framed and a 406 and a 407 are not, so a reader
that ends a head at CRLF CRLF and nowhere else reports every refusal as a
transport failure.
