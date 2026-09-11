<div align="center">

<!-- Absolute, and pointing at `nodemaven/.github`, for the same reason the Python
     README does it: this file becomes the crates.io long description and crates.io
     resolves nothing relative, so a relative src is a broken image on the package
     page. `.github` is public, so its raw URL answers 200 to a logged-out
     visitor. -->
<!-- `/ghrust` did not exist on 2026-09-09 and answered 404, so this href was the
     destination itself; the slug was created and both were re-fetched on
     2026-09-10: `/ghrust` 302s to nodemaven.com and `/githrust`, used in Quick
     start, 302s to the dashboard. Two slugs, two destinations, not a typo. The
     rule that produced the 404 stands - a URL written by analogy with a working
     one is not a check, so fetch any new slug before it ships. -->
<a href="https://go.nodemaven.com/ghrust"><img src="https://raw.githubusercontent.com/nodemaven/.github/main/profile/assets/nodemaven-mark.svg" alt="NodeMaven" height="56"></a>

# NodeMaven Rust SDK

**Builds the proxy username a gateway expects, and refuses the input it would silently drop.**

<!-- Four badges now. This comment said "three, and the two missing ones are
     deliberate" - no CI workflow to point a build badge at, and a repository
     that was internal and would have 404'd for the reader this file is written
     for. Both of those stopped being true on 2026-09-11: the user made the
     repository public and a workflow landed the same day, so the badge goes in
     with the thing it points at rather than ahead of it.

     The crates.io and docs.rs badges render from the published tarball and lag
     a release, so all four are correct on GitHub the moment this is pushed and
     on the package page only after the version that carries them ships. -->
[![CI](https://github.com/nodemaven/nodemaven-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/nodemaven/nodemaven-rust/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/nodemaven.svg)](https://crates.io/crates/nodemaven)
[![docs.rs](https://docs.rs/nodemaven/badge.svg)](https://docs.rs/nodemaven)
[![license](https://img.shields.io/crates/l/nodemaven.svg)](#license)

</div>

`Proxy` opens no socket. It builds the username a proxy gateway expects, refuses
the input that gateway would mishandle, and hands the result to whatever HTTP
client you already use. One call reaches the network, by name and never on
construction: `proxy.check()` opens a single CONNECT and reports what came back.

**Works with** reqwest · ureq · curl · chromiumoxide · thirtyfour - and anything
else that takes a proxy URL, because that is all it hands back.

```
cargo add nodemaven
```

The API reference is on [docs.rs](https://docs.rs/nodemaven); what follows is the
part you need before adding the dependency.

## Quick start

`login` and `password` are the **Proxy Username and Proxy Password** assigned
under Proxy Setup in the [dashboard](https://go.nodemaven.com/githrust) - a separate
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

Credentials can come from `NODEMAVEN_LOGIN` and `NODEMAVEN_PASSWORD` instead, so
nothing is in your source, and the gateway address from `NODEMAVEN_HOST` and
`NODEMAVEN_PORT`:

```rust
let proxy = Proxy::builder().param("country", "us").build()?;
```

The same identity, for other clients:

```rust
proxy.url();          // http://user:pass@gate.nodemaven.com:8080  - reqwest, ureq, curl
proxy.browser();      // { server, username, password } - unencoded, for a browser driver
proxy.username();     // the username on its own
proxy.server();       // host:port, no credentials
```

`url()` encodes the credentials for use in a URL; `browser()` returns them raw,
because a browser driver takes the three fields separately and encodes them
itself.

## Parameters

What the shipped NodeMaven definition accepts. Every name here was confirmed
against the gateway rather than transcribed:

| parameter | what it selects | values measured to work |
|---|---|---|
| `country` | country code, or `any` | `us`, `de`, ... |
| `region` | area inside the country | a name |
| `city` | city inside the country - send it with its `region` | a name |
| `isp` | the exit's ISP | a name |
| `type` | mobile or residential exits, which are different pools | `mobile`, `residential` |
| `sid` | the sticky session - see below | any string with no `-` |
| `ttl` | how long that session is held | `1m`, `10m`, `10h`, `24h` |
| `filter` | IP quality | `low`, `medium`, `high` |
| `speed` | claims a connection speed class | `fast`, `slow` |
| `ipv4` | claims to force IPv4 | `true` |

`ipv4` and `speed` are accepted by the gateway, but their effects have not been
independently verified, and the table says `claims to` for that reason.

**Parameter names are validated locally; values are passed through** unless a
gateway definition constrains them. A name that is not in this table fails
before anything is sent, because the gateway answers an unknown name with 200
and drops the setting.

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

Which parameters fold is declared in the gateway definition, as data, so a
gateway you describe yourself folds what you say it folds and nothing else.

**`sid`, `filter`, `ttl` and `speed` are sent with their case unchanged**, so
pass those in lower case. `sid` is yours, and folding an identifier a caller
chose would rename their session.

**Every value not folded is refused if it contains whitespace.** There is no
spelling of a space that reaches this gateway intact.

## Sticky sessions

One `Proxy` is one identity. Pin it to a sticky session:

```rust
let held = proxy.session("order4417")?;
```

**A session id cannot contain the gateway's separator**, which for this one is
`-`, and the builder returns an error rather than connecting. The gateway cuts a
value at its separator, so this applies to every parameter and not only to
`sid`.

**The session key is the whole parameter set, not the session id.** `country=us,
sid=A` and `country=us, sid=A, filter=medium` are two different sessions, so
adding or removing any parameter moves you to a different exit address. That is
why parameters change through a method returning a new `Proxy` rather than
through `&mut self`:

```rust
let germany = proxy.with_param("country", "de")?;   // a new identity, a new exit
let plain   = proxy.without_param("filter")?;       // also a new identity
```

The order the parameters are written in does not change which exit you get.

## Validation

A proxy gateway is bad at telling you that you got the username wrong. A value it
will not take comes back as `406`, `407`, `410`, `500` or a connection that hangs,
none of which name the parameter - and an unknown parameter *name* is answered
`200` and dropped, so the request succeeds, your code carries on, and the setting
you asked for was never applied. Nothing that comes back over the wire can tell
you that happened.

So this crate checks before anything is sent:

```text
Proxy::builder().login("u").password("p").param("contry", "us").build()

Error::Param: NodeMaven does not know the parameter "contry": it is answered
with 200 and dropped, so the connection would succeed and your setting would
NOT be applied. Known: ["city", "country", "filter", "ipv4", "isp", "region",
"sid", "speed", "ttl", "type"]
```

Which names a gateway recognises, what its status codes mean and which values it
folds are data in the gateway definition rather than decisions in this code, so
they are quotable, correctable and per-gateway. Print the shipped one's own notes
with `nodemaven::load("nodemaven")?.notes()`.

## Errors

One enum, `Error`, with one variant per kind of thing that can be wrong. It is
`#[non_exhaustive]`, so matching it needs a `_` arm and a new variant is not a
breaking change.

| variant | returned when |
|---|---|
| `Error::Param` | a parameter name is unknown, a value is empty, a value contains the gateway's separator, or a value is outside a list the definition declares |
| `Error::Credentials` | no login, no password, or no gateway address, from arguments or environment; or a port that is not 1-65535 |
| `Error::Provider` | a gateway definition is missing, unreadable, or internally inconsistent |
| `Error::Check` | a CONNECT produced no answer at all - no route, no reply, no status line |

The first three are found before a socket exists. `Error::Check` is deliberately
narrow: **a gateway that answers is not an error.** A 407 is the gateway
answering the question that was asked, so it comes back as a value.

## Checking a connection

Validation catches everything knowable without sending anything. For the rest - a
wrong password, a country the pool does not have, a value the gateway dislikes -
one call opens a single CONNECT and reports what came back:

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

The exit address arrives on the CONNECT reply itself, so knowing where you came
out costs one handshake and no traffic through the tunnel. **Handle `exit_ip()`
being `None`** - more than one implementation answers behind this hostname and
they do not agree about the header. If you need it every time, read it through
the tunnel from a service that echoes it.

**A refusal is a value, not an `Err`.** A refused tunnel comes back as
`Ok(Check)` carrying the gateway's own reading of its own status code, because
the status code is the thing you came for and a general HTTP client buries it:

```text
407 Proxy Authentication Required via gate.nodemaven.com:8080 in 0.19s
usually NOT your credentials, despite what the status says. A value the gateway will not take on `country`, `filter`, `ttl`, `type` or `speed` answers 407, and so does a wrong password. Check the values before the password - and check the case of `ttl`, which is the one value that is case-sensitive: `10M` is refused where `10m` is accepted.
```

**`ok()` does not mean your parameters were applied.** An unrecognised name is
answered `200` too, which is the whole reason unknown names are refused before
sending. No call can recover that after the fact and this one does not pretend to.

`check()` tunnels to `api.ipify.org:443` by default, and whatever you name will
see a TCP connection from your exit address. To choose it, or to change the
timeout, configure the CONNECT instead of taking the default:

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

**It does not retry.** Retry policy belongs to the caller, and ours measured
that repeated failures correlate with sharply worse outcomes - a session several
failures deep spends roughly a hundred attempts per delivered page against under
two in a healthy one. The harness behind that number is open source at
[nodemaven/proxy-benchmark](https://github.com/nodemaven/proxy-benchmark), so it
can be re-run rather than believed.

It also does not own an HTTP client, a connection pool or a browser, and it
brings no async runtime with it. Those are yours, and they are better than
anything a vendor SDK would bundle.

## Other gateways

Parameters are data, not hardcoded keywords. A gateway is its prefix, separators,
session parameter and the set of parameter names it recognises - and a definition
written by you goes through the same builder and the same validation as the one
shipped here. Keep it in a TOML file:

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

`Provider::builder` does the same thing in code, for a proxy you have no
definition for. An empty `known_params` is not a stub: it says nobody has
established what that gateway recognises, so every parameter is refused rather
than sent to be dropped in silence.

Credentials fall back to the environment under the definition's id in upper case,
so the file above reads `MY_GATEWAY_LOGIN` and `MY_GATEWAY_PASSWORD` and never
`NODEMAVEN_*`. One process can hold several gateways without their credentials
reaching each other. **The id comes from the filename, not from the variable you
bind it to** - use `load_file_as` to state it outright.

## Requirements

Rust 1.85 or newer, which is the MSRV `toml` declares - nothing in this crate
needs it. One dependency, `toml`, with default features off: the writer half is
dead weight here because nothing in this crate emits TOML. No unsafe code
(`#![forbid(unsafe_code)]`), no build script, no async runtime.

## License

MIT.
