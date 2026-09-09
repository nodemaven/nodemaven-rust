# Changelog

Written against the source tree and against the run that produced each number,
not from memory.

A note on how entries here are worded, because it is the rule the crate itself
is built on: a claim about what the gateway accepts carries the probe that
established it and the date it was run. "The vendor's own documentation says so"
is not one of those, and an entry resting on it says so outright.

## Unreleased

Documentation only. No public item was added, removed or changed, and
`cargo test --offline` is 135 green before and after - down from 140 because six
README-reading test cases lost their subject and one replaced them.

**None of this reaches the crates.io page until 0.1.1.** crates.io renders the
README that was inside the published tarball and never re-reads the file, which
is the same mechanism that has left the Python package on PyPI without a
repository link for three versions. The badges below are correct on GitHub today
and on the package page only after a release.

### The README is a usage document again

It had grown to 485 lines, and roughly a third of them were evidence: a ten-row
table of what each status code means, a six-CONNECT paragraph about sending
`city` without `region`, a four-round separator probe, a twenty-round parameter
order probe, and a six-row retry table over 1464 attempts. Each was true and each
was in the wrong file. A reader deciding whether to add the dependency wants to
know what the crate does; the reader who wants the probe behind a claim is a
different person arriving on purpose. It is now 365 lines - it reached 361 on the
cut and gained four to the comment described two sections down.

**Nothing measured was deleted, and that was checked rather than assumed.** Every
finding cut from the README already ships on a surface a user of the crate can
reach, so the cut removed a second copy and not a fact:

- the status-code meanings are `connect_reactions` in the shipped provider
  definition, which is what `check()` prints and what `Provider::reaction()`
  answers from - the README table was a transcription of data the crate carries
- the parameter-order, ISP, `ttl` case and city findings are in that definition's
  `notes`, printed by `nodemaven::load("nodemaven")?.notes()`
- the 1464-attempt retry measurement is in the crate-level documentation, so it
  is on docs.rs

The API reference table went with them, for a reason specific to this registry:
crates.io renders the long description and docs.rs renders the reference, and a
hand-written copy of the second inside the first is a third copy of the contract
that can only go stale. `missing_docs` is `warn` and `cargo doc` is clean, so
every exported item is documented where the documentation is derived from the
code.

### Badges

Three, on GitHub and in the file that becomes the package page: the crates.io
version, the docs.rs build and the license. All three were fetched on 2026-09-09
before being added rather than assumed to resolve - `crates.io: v0.1.0`,
`docs: passing`, `license: MIT`.

Two that a Rust README usually carries are deliberately absent. **There is no CI
workflow in this repository**, checked the same day both locally and through the
API, so a build badge would be a permanently broken image. And the repository is
internal, so a link to it answers 404 to the reader this file is written for.
Both go in when the thing they point at exists.

One measurement trap worth keeping from that check, **stated wrongly here first
and corrected the same day**. `https://crates.io/crates/nodemaven` does answer
404 to a naive request, and this entry originally blamed the `User-Agent`,
because the retry that got a 200 sent a browser one. The retry changed two
things. It is the **`Accept` header**: with `Accept: text/html` the page is 200
and with no `Accept` at all it is 404, at an identical `User-Agent`.

What settles it is the positive control, and the first version had none.
`https://crates.io/crates/serde` answers **404 the same way** to a request that
does not ask for HTML. A crate with 500 million downloads is not missing, so the
404 was never evidence about whether our crate exists - which is precisely the
claim the check was run to make.

The general form is the one this crate keeps re-learning: an explanation that
fits was allowed to stand in for a control that would have separated two
variables. The rule that catches it costs one request - **ask the same question
about something whose answer you already know.**

### The logo at the top of the README linked to a 404

The banner's `href` was `https://go.nodemaven.com/ghrust`, which redirects once
to `https://nodemaven.com/404`. That is the first link in the file that becomes
the crates.io package page, so it is CEO rule 1 in the most expensive location
available, and it would have shipped with 0.1.1.

It is not a broken shortener. `https://go.nodemaven.com/ghpython`, the same
pattern in the Python README, answers **200** and lands on
`nodemaven.com/?utm_source=github&utm_content=nodemaven_python`. The domain is a
Bitly branded short domain where each slug has to be created by hand, and
`ghrust` never was - the link was written to match its Python neighbour and
never fetched.

Fixed by pointing the `href` at the destination the slug would have resolved to,
with `utm_content=nodemaven_rust`, verified 200. That loses the shortener's
click count and keeps the campaign attribution, and it is one line to put back
once somebody creates the slug. A comment above it says so, because the obvious
tidy for the next editor is to make it match the Python file again.

The logo `src` itself is fine. `raw.githubusercontent.com` answered **429** from
this host, which is rate limiting and not a dead link; the file is listed in
`nodemaven/.github` through the API, and that repository is public.

### `tests/readme.rs` went from eight cases to three

Six of them scanned a `## Reference` section that no longer exists. They were not
dropped for convenience - their subject moved to rustdoc, which is argued in the
file's own module documentation.

One replaces them, in the opposite direction and better suited to what the README
now is: `every_call_the_readme_shows_on_a_proxy_exists` derives the calls from
the README text and requires each to be `pub fn` on `Proxy`. The old form asked
whether the code was fully documented, which is rustdoc's job. The new form asks
whether the documentation is true, which is nobody else's.

## 0.1.0 - 2026-09-08

Published at 19:44:51 UTC, as crates.io recorded it.

**This heading read `0.1.0 - unreleased` in the copy that shipped inside the
0.1.0 tarball, and it will say that forever.** The reasoning was that a
publication date should be the time crates.io records rather than the day the
section was written, so the date was left out instead of guessed. That is right
about the date and wrong about the file: the tarball freezes whatever the
working tree held at `cargo publish`, so refusing to guess did not leave the
line blank until it could be filled in - it shipped the word "unreleased" on a
released version.

What it looked like from the inside: it read as the careful option, because
every other rule here says not to write a date you have not observed. The rule
that applies is a different one - a file that goes into a permanent artifact has
to be correct at the moment it is packaged, not afterwards. **Date the heading
in the release commit from now on.** The cost this time is one wrong word in one
shipped file; the same reflex applied to a version number or a URL would be
worse.

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
