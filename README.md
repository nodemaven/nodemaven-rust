<div align="center">

<!-- Absolute, and pointing at `nodemaven/.github`, for the same reason the Python
     README does it: this file becomes the crates.io long description and crates.io
     resolves nothing relative, so a relative src is a broken image on the package
     page. `.github` is public, so its raw URL answers 200 to a logged-out
     visitor. -->
<a href="https://go.nodemaven.com/ghrust"><img src="https://raw.githubusercontent.com/nodemaven/.github/main/profile/assets/nodemaven-mark.svg" alt="NodeMaven" height="56"></a>

# NodeMaven Rust SDK

**Builds the proxy username a gateway expects, and refuses the input it would silently drop.**

<!-- No badges yet, deliberately. crates.io version, docs.rs and CI all render
     "not found" until the crate is published and the repository exists. They go
     in with the publishing commit, not before. -->

</div>

`Proxy` opens no socket. It builds the username a proxy gateway expects, refuses
the input that gateway would mishandle, and hands the result to whatever HTTP
client you already use.

One call does reach the network, by name and never on construction:
[`proxy.check()`](#asking-the-gateway) opens a single CONNECT and tells you what
the gateway said about it.

**Works with** reqwest · ureq · curl · chromiumoxide · thirtyfour - and anything
else that takes a proxy URL, because that is all it hands back.

```
cargo add nodemaven
```

<!-- In file order, so the line doubles as a table of contents. Anchors are
     GitHub's slugs; crates.io renders the same ones. -->

[Quickstart](#quickstart) · [Reference](#reference) · [Parameters](#parameters) · [Errors](#errors) · [Sticky sessions](#sticky-sessions) · [Why the validation is the point](#why-the-validation-is-the-point) · [Asking the gateway](#asking-the-gateway) · [What it does not do](#what-this-crate-does-not-do) · [Other gateways](#other-gateways) · [docs.rs](https://docs.rs/nodemaven)

## Quickstart

`login` and `password` are the **Proxy Username and Proxy Password** assigned
under Proxy Setup in the [dashboard](https://dashboard.nodemaven.com) - a separate
pair from the account you sign in with. The other option there is IP
whitelisting, which needs no credentials in the username at all; both are
described in
[authentication methods](https://docs.nodemaven.com/en/articles/9979031-authentication-methods).

```rust
use nodemaven::Proxy;

let proxy = Proxy::builder()
    .login("your-login")
    .password("your-password")
    .param("country", "us")
    .param("filter", "medium")
    .build()?;

let client = reqwest::Client::builder()
    .proxy(reqwest::Proxy::all(proxy.url())?)
    .build()?;
```

**No account? Any proxy you already have works.** A gateway is a data
description, not a code path, so one that ships no definition here goes through
the same builder and the same validation:

```rust
use nodemaven::{Provider, Proxy};

// No known_params is not a stub. It says nobody has established what this
// gateway recognises, so every parameter is refused rather than sent to be
// silently dropped - see "Why the validation is the point" below.
let mine = Provider::builder("mine", "My proxy").build()?;

let proxy = Proxy::builder()
    .provider(mine)
    .login("your-login")
    .password("your-password")
    .host("proxy.example.com")
    .port(8000)
    .build()?;
```

Describe the parameters it does take and it validates those too - see
[Other gateways](#other-gateways).

Credentials can come from the environment instead, so nothing is in your source:

```rust
// NODEMAVEN_LOGIN and NODEMAVEN_PASSWORD
let proxy = Proxy::builder().param("country", "us").build()?;
```

The same identity, for other clients:

```rust
proxy.url();          // http://user:pass@gate.nodemaven.com:8080  - reqwest, ureq, curl
proxy.browser();      // { server, username, password } - unencoded, for a browser driver
proxy.username();     // the username on its own
proxy.server();       // host:port, no credentials
```

`url()` percent-encodes both halves; `browser()` does not, because Playwright,
Patchright, Puppeteer and chromiumoxide take the three fields separately and
encode them themselves. Encoding twice turns `pa/ss` into `pa%252Fss` and
authentication fails while blaming the credentials.

## Reference

<!-- The rule this section is held to: nothing below this heading explains a
     decision. Reasoning belongs under "Why the validation is the point",
     "Errors" and "What this crate does not do", where a reader goes on purpose.
     If a sentence here would survive the decision being reversed, it does not
     belong here.

     docs.rs is the fuller reference once the crate is published. This is the
     part a reader needs before deciding to add the dependency. -->

Everything the crate exports, with what it returns. `Result<T>` is
`std::result::Result<T, Error>`; see [Errors](#errors).

### `Proxy` and `ProxyBuilder`

Built through `Proxy::builder()`. Every setter takes `self` and returns `Self`.

| builder call | falls back to | refused when |
|---|---|---|
| `.provider(Provider)` | `load("nodemaven")` | - |
| `.login(impl Into<String>)` | `{ID}_LOGIN` | absent from both |
| `.password(impl Into<String>)` | `{ID}_PASSWORD` | absent from both |
| `.host(impl Into<String>)` | `{ID}_HOST`, then the definition | absent from all three |
| `.port(u16)` | `{ID}_PORT`, then the definition | `0`, or `{ID}_PORT` is not 1-65535 |
| `.param(name, value: impl Display)` | - | unknown name, empty value, whitespace, or the separator inside a value |
| `.params(impl IntoIterator<Item = (K, V)>)` | - | the same, per pair |
| `.build() -> Result<Proxy>` | - | any of the above |

`{ID}` is the definition's id in upper case with `-` as `_`, so the shipped one
reads `NODEMAVEN_LOGIN` and a definition you write reads yours.

| on a built `Proxy` | returns |
|---|---|
| `.username()` | `String` - the built username, parameters included |
| `.server()` | `String` - `host:port`, no credentials |
| `.url()` | `String` - `http://user:pass@host:port`, both halves percent-encoded |
| `.url_with_scheme(&str)` | `String` - the same with another scheme |
| `.browser()` | `BrowserProxy { server, username, password }` - **not** encoded |
| `.params()` | `&[(String, String)]` - in the order they will be sent |
| `.provider()` | `&Provider` |
| `.with_param(name, value: impl Display)` | `Result<Proxy>` - a **new** identity |
| `.without_param(name)` | `Result<Proxy>` - a **new** identity |
| `.session(session_id)` | `Result<Proxy>` - sets the definition's session parameter |
| `.check()` | `Result<Check>` - **the only call here that opens a socket** |
| `.connect()` | `Connect` - the same CONNECT, configured but not sent |

`BrowserProxy` is a plain struct with three public `String` fields and no
methods.

### `Check` and `Connect`

`Check` is what one CONNECT produced. A refusal is a `Check` and not an `Err`.

| on a `Check` | returns |
|---|---|
| `.ok()` | `bool` - true only on 200 |
| `.status()` | `u16` |
| `.reason()` | `&str` - verbatim, not normalised, not even for case |
| `.server()` | `&str` - `host:port`, no credentials |
| `.exit_ip()` | `Option<&str>` |
| `.elapsed()` | `Duration` - includes DNS and the TCP handshake |
| `.headers()` | `&[(String, String)]` - arrival order, names ASCII-lower-cased |
| `.header(name)` | `Option<&str>` - case-insensitive, **last** match wins |
| `.meaning()` | `Option<&str>` - what this status means on this gateway |

`Connect` is the CONNECT before it is sent. `proxy.connect()` returns one that
already carries the server, the built username, the password, the exit-address
header and the whole reaction table.

| on a `Connect` | |
|---|---|
| `Connect::new(server, username, password)` | build one by hand, no `Provider` needed |
| `.target(impl Into<String>)` | default `DEFAULT_TARGET`, `api.ipify.org:443` |
| `.timeout(Duration)` | default `DEFAULT_TIMEOUT`, 15 s; zero is refused |
| `.exit_ip_header(impl Into<String>)` | which header carries the exit address |
| `.reaction(u16, meaning)` / `.reactions(BTreeMap<String, String>)` | what a status means here |
| `.send() -> Result<Check>` | sends it |

### `Provider` and the module functions

| function | returns |
|---|---|
| `load(provider_id: &str)` | `Result<Provider>` - a definition shipped in the crate |
| `load_file(path: impl AsRef<Path>)` | `Result<Provider>` - id from the filename |
| `load_file_as(path, provider_id: &str)` | `Result<Provider>` - id stated outright |
| `load_str(text: &str, provider_id: &str)` | `Result<Provider>` - TOML already in memory |
| `available()` | `Vec<&'static str>` - the ids the crate ships |

`DEFAULT_PROVIDER` is the id `load` is called with when a builder names none.
`ASCII_WHITESPACE` is the exact set trimmed during folding, exported so a
definition you write can be checked against the same rule the crate uses.

A `Provider` is read-only once built. Beyond the accessors that mirror the TOML
keys - `id()`, `label()`, `status()`, `known_params()`, `prefix()`,
`separator()`, `pair_separator()`, `session_param()`, `host()`, `port()`,
`exit_ip_header()`, `source()`, `source_read()`, `notes()` - four answer
questions about behaviour:

| | |
|---|---|
| `.is_measured()` | `bool` - was this dialect read off the wire, or out of documentation |
| `.normalizes(name)` | `bool` - is this parameter folded |
| `.normalized(name, value)` | `String` - the value as it will be sent |
| `.allowed(name)` | `Option<&[String]>` - the legal values, if the definition declares any |
| `.reaction(status)` | `Option<&str>` - what that status means on this gateway |
| `.connect_reactions()` | `&BTreeMap<String, String>` - the whole table, keyed by status as a string |
| `.spell(name)` | `&str` - the name as it goes on the wire, after aliases |

`Provider::builder(id, label)` returns a `ProviderBuilder` with a setter per key
above and a `.build() -> Result<Provider>`.

## Parameters

What the shipped NodeMaven definition accepts. Every name here was confirmed
against the gateway rather than transcribed:

| parameter | what it selects | values measured to work |
|---|---|---|
| `country` | country code, or `any` | `us`, `de`, ... |
| `region` | area inside the country | a name |
| `city` | city inside the country | a name |
| `isp` | the exit's ISP | a name |
| `type` | mobile or residential exits | `mobile`, `residential` |
| `sid` | the sticky session - see below | any string with no `-` |
| `ttl` | how long that session is held | `1m`, `10m`, `10h`, `24h` |
| `filter` | IP quality | `low`, `medium`, `high` |
| `speed` | claims a connection speed class - see below | `fast`, `slow` |
| `ipv4` | claims to force IPv4 - see below | `true` |

`type` picks a different pool rather than a filter over one pool. Five requests per arm with a fresh `sid` and `country=us`:
`type=mobile` drew T-Mobile and Verizon Wireless ASNs, while `type=residential`
and leaving it unset drew Comcast, Charter, Windstream and other wireline
carriers, with no mobile ASN among them.

**`ipv4` and `speed` are confirmed names whose effects are unmeasured**, and the
table says `claims to` for that reason. For `ipv4` the name took a different
method to confirm: a junk value on it is answered `200`, so it cannot be told
apart from an unimplemented name that way. What tells them apart is the sticky
session, whose key is the parsed parameter set - over two
independent session ids with both controls holding, `ipv4=true` moves the exit
and an unknown name does not. `ipv4=false` lands on the same exit as leaving it
out, which is what a default would do and also what a dropped value would do.

**Names are validated. Values, on this gateway, are not.** Passing a name that
is not in this table fails before anything is sent, because the gateway answers
an unknown name with 200 and drops the setting. Values are passed through,
because what is known is which ones have been *observed* to work - and that is
not the same as the set the gateway accepts. Refusing on a guessed list would
block a setting that would have worked, which is the worse mistake of the two.
The schema does carry a per-parameter list of legal values and refuses anything
outside it; the shipped definition leaves that list empty for every parameter,
deliberately, and a definition you write yourself gets the check as soon as you
fill it in.

A value is written with `Display`, so `.param("ipv4", true)` produces `ipv4-true`
and `.param("port_hint", 8080)` produces `port_hint-8080` without a cast.

### Case and spacing

`country`, `region`, `city`, `isp` and `type` are folded to the gateway's wire
form: surrounding whitespace trimmed, ASCII upper case lowered, each remaining
space turned into `_`. So

```rust
Proxy::builder().login("u").password("p")
    .param("region", "District of Columbia")
    .build()?
    .username();                       // u-region-district_of_columbia
```

That is not a tidying-up. `region-district_of_columbia` is the form this
gateway generates for itself - it appears in a username the dashboard issued -
and the vendor's own client applies the same transformation. Without the fold
the value goes out with a space in it, which a proxy username cannot carry: the
CONNECT line is one token, so the value is either malformed or cut short.

Which parameters fold is declared in the gateway definition, as data, so a
gateway you describe yourself folds what you say it folds and nothing else.

**`sid`, `filter`, `ttl` and `speed` are sent with their case unchanged.** `sid`
is yours, and folding an identifier a caller chose would rename their session,
so it is left alone whatever the gateway does with it.

For the other three, **pass lower case**, and for `ttl` that is not advice.
`ttl-10m` opens the tunnel and `ttl-10M` is answered `407`,
which reads as a credentials problem and is not one. `filter` and `speed` were
not refused in either case, and this crate still does not fold them - what the
gateway accepts today and what it will accept next month are different claims,
and the fold list is data in the gateway definition rather than a decision in
this code.

**`ttl` counts in minutes and hours.** `1m`, `10m`, `10h` and `24h` open the
tunnel; `10s`, `10d` and a bare `10` are answered `407`. Nothing enumerates the
range, so the crate refuses no `ttl` value - a list of the four measured ones
would block `2h` if the gateway takes it, and a false refusal carrying our name
is worse than the gap.

**Every other value is refused if it contains whitespace.** There is no spelling
of a space that works here - `username()` would emit it raw, `url()` would
percent-encode it to `%20`, and a browser driver would send a third thing - so
between the fold and the refusal, no value with whitespace in it can reach the
wire by any path.

Credentials come from `NODEMAVEN_LOGIN` and `NODEMAVEN_PASSWORD` when not passed
in, and the gateway address from `NODEMAVEN_HOST` and `NODEMAVEN_PORT`.

## Errors

One enum, [`Error`], with one variant per kind of thing that can be wrong. It is
`#[non_exhaustive]`, so matching it needs a `_` arm and a new variant is not a
breaking change.

| variant | returned when |
|---|---|
| `Error::Param` | a parameter name is unknown, a value is empty, a value contains the gateway's separator, or a value is outside a list the definition declares |
| `Error::Credentials` | no login, no password, or no gateway address, from arguments or environment; or a port that is not 1-65535 |
| `Error::Provider` | a gateway definition is missing, unreadable, or internally inconsistent |
| `Error::Check` | a CONNECT produced no answer at all - no route, no reply, no status line |

`Error::Param` also covers the structural case of `session()` on a definition
that declares no session parameter.

The first three are found before a socket exists. `Error::Check` is the one that
needs one, and it is deliberately narrow: **a gateway that answers is not an
error.** A 407 is the gateway answering the question that was asked, so it comes
back as a value carrying the status, the reason phrase and the headers - see
[Asking the gateway](#asking-the-gateway). `Error::Check` means there is nothing
to report at all.

## Sticky sessions

One `Proxy` is one identity. Pin it to a sticky session:

```rust
let held = proxy.session("order4417")?;
```

**A session id cannot contain the character the gateway separates parameters
with**, which for this one is `-`, and passing one returns an error rather than
connecting. That is measured and not a precaution: a probe opened
tunnels with `sid-order8e3bf9-4417` and with `sid-order8e3bf9`, four rounds each,
interleaved, and both landed on **one exit address** while a third arm spelled
`sid-order8e3bf94417` held a different one throughout. The gateway cuts the value
at the separator and reads the rest as something else, so every order id
beginning `order` would quietly share one session and one exit.

**The same cut applies to every parameter, not just `sid`,** which is why
`.param()` refuses a separator in any value. `isp-verizon`
opens the tunnel, a junk `isp` is answered `406`, and `isp-verizon-zzqqx-zzqqx`
is answered `200` - so the gateway took `verizon` as the ISP and read the tail as
a parameter name it does not know, which it drops silently.

**The session key is the whole parameter set, not the session id.**
`country=us, sid=A` and `country=us, sid=A, filter=medium` are two different
sessions on the gateway, so adding or removing any parameter moves you to a
different exit address. That is why parameters change through a method that
returns a new `Proxy` rather than through `&mut self` - the move is a different
identity, and the code should say so:

```rust
let germany = proxy.with_param("country", "de")?;   // a new identity, a new exit
let plain   = proxy.without_param("filter")?;       // also a new identity
```

**The set, not the order.** Measured over 20 rounds a side: the canonical
parameter order and a shuffled one drew the same exit 20 times each, while a
control differing by one parameter *value* drew a different exit 20 times. So
the order this crate emits parameters in cannot change which exit you get.

## Why the validation is the point

<!-- Everything below this heading is a measurement, and none of it carries a
     date. The dates and the probes are in the shipped definition's `notes`,
     which ships inside the crate, and since 2026-09-08 in CHANGELOG.md, which
     ships beside it - but neither is linked from here. crates.io resolves
     nothing relative, and the repository an absolute link would point at is
     internal, so both forms are dead for a logged-out reader. The pointer goes
     in with the release that follows the repository being public. -->

Every gateway behaviour below was measured against the live gateway rather than
transcribed from documentation. Each one carries its date and the probe behind
it in the shipped definition's `notes` field, which travels with the crate:

```rust
println!("{}", nodemaven::load("nodemaven")?.notes());
```

A gateway is bad at telling you that you got the username wrong. One class of
mistake - a value it will not take - comes back five different ways, and not one
of them names the parameter. Read by raw CONNECT, one arm per row:

| you sent | the gateway answers |
|---|---|
| bad `country` value | `407 Proxy Authentication Required` |
| bad `region` value | `406 Not Acceptable` |
| bad `city` value | `406 Not Acceptable` |
| `city` sent without `region` | `500 Internal Server Error` |
| bad `isp` value | `406 Not Acceptable`, and `410 Gone` for `comcast` |
| bad `filter` value | `407 Proxy Authentication Required` |
| bad `ttl` value | `407 Proxy Authentication Required` |
| bad `type` or `speed` value | `407 Proxy Authentication Required` |
| empty value | nothing, the connection hangs about 20 s |
| **unknown parameter name** | **`200`, and the parameter is ignored** |

Every `407` there sends you to check credentials that are correct, and the `406`
does not even say which of the two parameters it refused: a bad `region`, a bad
`isp` and `charter` - a real ISP - all answer it, so it separates neither the
parameter nor a misspelling from a pool you cannot have. `comcast` is the one
value measured to answer `410` instead, which reads as a name the gateway knows
and a pool this account cannot reach - one ISP, so read it that narrowly.

**The two `city` rows are one rule: send `city` with its `region`.** Measured over six CONNECTs holding the login, the password, the target, the
gateway host and port and the parameter order fixed. `country=us`,
`region=louisiana`, `city=abbeville` opens the tunnel, and so does a second city
in a second region. The same city with the region left out answers `500`, and an
invented name sent with a real region answers `406` - so the gateway does look
the name up, and the `500` is a request it could not resolve rather than a fault
on their side. The dashboard's own `locations/cities` catalogue is what says
which region a city belongs to.

The last row is worse than any of them: the request succeeds, your code carries
on, and the setting you asked for was never applied. Nothing that comes back
over the wire can tell you.

So this crate checks before anything is sent:

```text
Proxy::builder().login("u").password("p").param("contry", "us").build()

Error::Param: NodeMaven does not know the parameter "contry": it is answered
with 200 and dropped, so the connection would succeed and your setting would
NOT be applied. Known: ["city", "country", "filter", "ipv4", "isp", "region",
"sid", "speed", "ttl", "type"]
```

## Asking the gateway

Validation catches everything knowable without sending anything. For the rest -
a wrong password, a country the pool does not have, a value the gateway dislikes
- one call opens a single CONNECT and reports what came back:

```rust
let result = proxy.check()?;
println!("{result}");
```

<!-- 203.0.113.7 is RFC 5737 TEST-NET-3, reserved for documentation, so nobody
     reads it as a real exit address. Both output blocks in this section are
     pinned by `the_readme_shows_real_output` in tests/check.rs, which reads this
     file rather than carrying a copy - edit either block and that test fails. -->

```text
200 Connection established via gate.nodemaven.com:8080 in 0.42s, exit 203.0.113.7
```

The exit address arrives **on the CONNECT reply itself**, on a header the gateway
definition names, so knowing where you came out costs one handshake and no
traffic through the tunnel.

**Do not build anything on it being there.** More than one implementation
answers behind this hostname, which one you reach is decided by your username,
and they do not agree about the header: one measured `200` carried
`X-Exit-IP` where the shipped definition names `X-Proxy-Exit-IP`, and others
send no address at all. So `exit_ip()` is `None` more often than the definition
suggests, and that is normal rather than an error. If you need the address every
time, read it through the tunnel from a service that echoes it.

**A refusal is a value, not an `Err`.** The status code is the thing you came
for, and an `Err` would bury it - which is what a general HTTP client does, and
`reqwest` reports a failed tunnel as an `io::Error` with the status flattened
into its message and every header gone. So a refused tunnel comes back as
`Ok(Check)`, carrying the gateway's own reading of its own status code:

```text
407 Proxy Authentication Required via gate.nodemaven.com:8080 in 0.19s
usually NOT your credentials, despite what the status says. A value the gateway will not take on `country`, `filter`, `ttl`, `type` or `speed` answers 407, and so does a wrong password. Check the values before the password - and check the case of `ttl`, which is the one value that is case-sensitive: `10M` is refused where `10m` is accepted.
```

That second paragraph is data in the gateway definition, not a string in this
crate, because what a status code means is per-gateway. `Error::Check` is
returned only when nothing came back at all - DNS, a refused connection, a
timeout, or something that is not a proxy answering on that port.

Every field of a `Check` is in the [Reference](#check-and-connect) above.

**`ok()` does not mean your parameters were applied.** An unrecognised parameter
name is also answered `200` and dropped, which is the whole reason the section
above refuses unknown names before sending. No call can recover that after the
fact, and this one does not pretend to.

`check()` names the host it tunnels to - `api.ipify.org:443` by default - and
whatever you name will see a TCP connection from your exit address. There is no
CONNECT to nowhere, so it is configurable. Python spells that with keyword
arguments; here it is the same builder the rest of the crate uses:

```rust
let result = proxy.connect()
    .target("example.com:443")
    .timeout(Duration::from_secs(30))
    .send()?;
```

The timeout defaults to 15 seconds rather than something brisk, because one of
this gateway's measured reactions is no reply for about 20 seconds. A 5-second
timeout would report that as a network problem.

## What this crate does not do

**It does not retry.** That is deliberate, and it is the one design decision
here taken against a measurement rather than a preference.

Retrying a refused request is the thing that most reliably makes the next one
worse: each retry confirms automation to the target and burns the exit range for
everyone else sharing the pool. Measured over 1464 attempts, the chance that the
next attempt succeeds, by how many failures came immediately before it:

| failures before | P(next attempt succeeds) |
|---|---|
| 0 | 75% |
| 1 | 21% |
| 3 | 5.9% |
| 5 | 5.8% |
| 6 | 1.6% |
| 7-9 | 0.5% |

294 attempts were spent past six consecutive failures and returned 3 pages - 98
attempts per delivered page, against 1.7 in a healthy session. A crate that
shipped automatic retry as a default would be spending that on your behalf
without telling you.

Those 1464 attempts, and the cells they came from, are in
[nodemaven/proxy-benchmark](https://github.com/nodemaven/proxy-benchmark) - the
harness that measured them, open source, so the table above can be re-run rather
than believed.

It also does not own an HTTP client, a connection pool or a browser, and it
brings no async runtime with it. Those are yours, and they are better than
anything a vendor SDK would bundle.

## Other gateways

Parameters are data, not hardcoded keywords. A gateway is its prefix,
separators, session parameter and the set of parameter names it actually
recognises - and a definition written by you goes through the same builder and
the same validation as the one shipped here. Either build it in place, as in the
[quickstart](#quickstart), or keep it in a TOML file:

```toml
# my-gateway.toml
label = "My proxy"
known_params = ["country", "session"]
session_param = "session"
host = "proxy.example.com"
port = 8000
```

```rust
use nodemaven::{load_file, Proxy};

let mine = load_file("my-gateway.toml")?;
let proxy = Proxy::builder()
    .provider(mine)
    .login("u")
    .password("p")
    .param("country", "us")
    .build()?;
proxy.session("order4417")?;     // u-country-us-session-order4417
```

`known_params` is the whole point of the file: name a parameter that is not in
it and the build fails instead of connecting. Leave the list out and every
parameter is refused, which is the correct thing to say about a gateway whose
dialect nobody has established.

Credentials fall back to the environment under the definition's id in upper case,
so this one reads `MY_GATEWAY_LOGIN` and `MY_GATEWAY_PASSWORD` and never
`NODEMAVEN_*`. One process can hold several gateways without their credentials
reaching each other.

**The id comes from the filename, not from the variable you bind it to.**
`load_file("my-gateway.toml")` is `my-gateway` however it is named in your code,
and `-` becomes `_` in the variable names. Use `load_file_as` to say it
outright. The error returned when a credential is missing names the exact pair it
looked for, so this is one guess you never have to make.

Every definition carries a `status`. `measured` means traffic has gone through
that gateway and the dialect was read off the wire. `documented` means it was
transcribed from documentation and never exercised. Only `nodemaven` is shipped
here, and it is `measured`.

## Requirements

Rust 1.85 or newer, which is the MSRV `toml` declares - nothing in this crate
needs it. One dependency, `toml`, with default features off: the writer half is
dead weight here because nothing in this crate emits TOML. No unsafe code
(`#![forbid(unsafe_code)]`), no build script, no async runtime.

## License

MIT.
