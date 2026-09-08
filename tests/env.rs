//! The environment fallback, in its own test binary.
//!
//! `std::env` is process-global and the test harness runs tests on threads of
//! one process, so every case here lives in a single `#[test]` that sets and
//! unsets in sequence. Splitting them into separate `#[test]` functions in this
//! file would let one case's variables leak into another's, and the failure
//! would be intermittent rather than a red test. Cargo gives each file in
//! `tests/` its own process, which is why this is not in `proxy.rs`: the cases
//! there must never see a `NODEMAVEN_*` variable, and they do not.

use nodemaven::{Error, Proxy};

fn set(suffix: &str, value: &str) {
    std::env::set_var(format!("NODEMAVEN_{suffix}"), value);
}

fn clear() {
    for suffix in ["LOGIN", "PASSWORD", "HOST", "PORT"] {
        std::env::remove_var(format!("NODEMAVEN_{suffix}"));
    }
}

fn credentials_error(result: Result<Proxy, Error>) -> String {
    match result {
        Err(Error::Credentials(message)) => message,
        Err(other) => panic!("expected a credentials error, got {other:?}"),
        Ok(proxy) => panic!("expected a credentials error, got {proxy:?}"),
    }
}

#[test]
fn the_environment_fallback() {
    clear();

    // Nothing set anywhere: the refusal names both what is missing and the two
    // ways to supply it, because a caller who has neither read the docs nor set
    // the variables gets this message and nothing else.
    let message = credentials_error(Proxy::builder().build());
    assert!(
        message.contains("no login and no password for NodeMaven"),
        "{message}"
    );
    assert!(message.contains("NODEMAVEN_LOGIN"), "{message}");
    assert!(message.contains("NODEMAVEN_PASSWORD"), "{message}");

    // Half set is still a refusal, and it names only the half that is missing.
    set("LOGIN", "env-login");
    let message = credentials_error(Proxy::builder().build());
    assert!(message.contains("no password for NodeMaven"), "{message}");
    assert!(!message.contains("no login"), "{message}");

    // Both set: the gateway address comes from the shipped definition, so a
    // caller with two variables set needs to pass nothing at all.
    set("PASSWORD", "env-password");
    let proxy = Proxy::builder().build().expect("both credentials are set");
    assert_eq!(proxy.username(), "env-login");
    assert_eq!(proxy.server(), "gate.nodemaven.com:8080");

    // An empty variable is absent rather than empty. A shell that exports
    // `NODEMAVEN_LOGIN=` sets the variable, and treating that as a login would
    // build a proxy whose username is the parameter list with no account on the
    // front of it - which the gateway answers, on somebody else's account or on
    // none.
    set("LOGIN", "");
    let message = credentials_error(Proxy::builder().build());
    assert!(message.contains("no login for NodeMaven"), "{message}");
    set("LOGIN", "env-login");

    // The builder wins over the environment. This is the direction that matters:
    // a developer with credentials in a shell profile and a second account in
    // code gets the one in code.
    let proxy = Proxy::builder()
        .login("call-login")
        .password("call-password")
        .build()
        .expect("passed credentials");
    assert_eq!(proxy.username(), "call-login");

    // Host and port are overridable the same way, and the port is parsed rather
    // than trusted.
    set("HOST", "gate.example.com");
    set("PORT", "1080");
    let proxy = Proxy::builder().build().expect("credentials are still set");
    assert_eq!(proxy.server(), "gate.example.com:1080");

    // A port that is not a number is refused with the variable's own name and
    // value quoted, because the alternative - falling back to the definition's
    // 8080 - is a connection to a port the caller did not ask for.
    set("PORT", "eight thousand");
    let message = credentials_error(Proxy::builder().build());
    assert!(message.contains("NODEMAVEN_PORT"), "{message}");
    assert!(message.contains("\"eight thousand\""), "{message}");

    // Out of range counts as not a port number: 70000 parses as an integer and
    // is not one.
    set("PORT", "70000");
    let message = credentials_error(Proxy::builder().build());
    assert!(message.contains("not a TCP port"), "{message}");

    // Five more, and **only the first of them was a live defect** - the other
    // four were already refused and are here as the regression fence around the
    // rewrite, not as fixes. Measured 2026-09-08 with `rustc` on the five
    // strings before changing anything: `"+8080"` parsed to `Ok(8080)`,
    // `" 8080"` and `"8_080"` gave `InvalidDigit`, `"099999"` gave
    // `PosOverflow`. Stating that split rather than lumping them together,
    // because "we fixed five cases" would be four fifths untrue.
    //
    // `+8080` is the one that mattered: `u16::from_str` accepts a leading `+`,
    // so the variable was taken as 8080 and nothing said anything. It is not the
    // same class of mistake as an outright refusal - it is a value the caller
    // typed, silently rewritten into a different one, which is what this crate
    // refuses to do with a gateway parameter and was doing with a port.
    //
    // `\u{b2}` is Python's `str.isdigit()` trap arriving here. It was never a
    // defect in Rust, where `parse` refuses it; it is tested on both sides
    // because the rule is shared and a shared rule is only shared where both
    // sides are pinned.
    for text in ["+8080", "\u{b2}", " 8080", "8_080", "099999"] {
        set("PORT", text);
        let message = credentials_error(Proxy::builder().build());
        assert!(message.contains("not a TCP port"), "{text:?}: {message}");
        assert!(message.contains("1 to 65535"), "{text:?}: {message}");
    }

    // Zero through the environment. It parses, so it used to arrive as
    // `Some(0)`, get filtered out by the address arm below and be reported as
    // "no gateway address ... pass host() and port()" - a sentence about the
    // one thing the caller got right. Both halves are asserted: the port is
    // named, and the message that used to be wrong is not the one produced.
    set("PORT", "0");
    let message = credentials_error(Proxy::builder().build());
    assert!(message.contains("not a TCP port"), "{message}");
    assert!(!message.contains("no gateway address"), "{message}");

    clear();
}
