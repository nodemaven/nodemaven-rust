//! Offline in full: no network, no credentials, no gateway.
//!
//! These cases are the seed of the golden vectors the other language SDKs will
//! run, and they are ported case for case from the Python SDK's
//! `tests/test_proxy.py`. Anything asserted here is a statement about what a
//! correct username is, not about how this implementation happens to be written.
//!
//! There is no environment fixture, unlike the Python suite: every case below
//! passes all four credentials explicitly, so the environment is never consulted
//! and a machine with `NODEMAVEN_*` set cannot change a result. The one case that
//! does read the environment is in `tests/env.rs`, on its own, because
//! integration tests inside one file share a process.

use nodemaven::{available, load, load_str, Error, Proxy, ProxyBuilder, Result};

/// The credentials every case uses. `gate.example.com` and not the shipped host,
/// so nothing here could be pointed at a real gateway by accident.
fn creds() -> ProxyBuilder {
    Proxy::builder()
        .login("acct")
        .password("pw")
        .host("gate.example.com")
        .port(8080)
}

fn param_error(result: Result<Proxy>) -> String {
    match result {
        Err(Error::Param(message)) => message,
        other => panic!("expected a Param error, got {other:?}"),
    }
}

fn credentials_error(result: Result<Proxy>) -> String {
    match result {
        Err(Error::Credentials(message)) => message,
        other => panic!("expected a Credentials error, got {other:?}"),
    }
}

fn provider_error(result: Result<nodemaven::Provider>) -> String {
    match result {
        Err(Error::Provider(message)) => message,
        other => panic!("expected a Provider error, got {other:?}"),
    }
}

mod the_username {
    use super::*;

    #[test]
    fn parameters_are_spelled_in_the_gateway_dialect() {
        let proxy = creds()
            .param("country", "us")
            .param("filter", "medium")
            .param("sid", "abc123")
            .build()
            .unwrap();
        assert_eq!(proxy.username(), "acct-country-us-filter-medium-sid-abc123");
    }

    #[test]
    fn no_parameters_is_the_bare_login() {
        assert_eq!(creds().build().unwrap().username(), "acct");
    }

    #[test]
    fn order_follows_the_call() {
        let first = creds()
            .param("country", "us")
            .param("sid", "x")
            .build()
            .unwrap();
        let second = creds()
            .param("sid", "x")
            .param("country", "us")
            .build()
            .unwrap();
        assert_eq!(first.username(), "acct-country-us-sid-x");
        assert_eq!(second.username(), "acct-sid-x-country-us");
    }

    #[test]
    fn a_non_string_value_is_spelled_the_way_the_gateway_reads_it() {
        // Rust's Display for bool prints the lower-case form, and Python
        // special-cases away from `str(True)` to reach it, so both SDKs send
        // these bytes. `ipv4-true` is the exact value the 2026-09-08
        // session-key run sent. It is not a form the gateway has been shown to
        // prefer: `True` and `TRUE` are answered 200 as well, and only `ttl`
        // has a measured case rule.
        let proxy = creds().param("ipv4", true).build().unwrap();
        assert_eq!(proxy.username(), "acct-ipv4-true");
    }
}

mod validation_the_gateway_cannot_do {
    use super::*;

    #[test]
    fn an_unknown_name_is_refused_before_anything_is_sent() {
        // The gateway answers this with 200 and drops the setting, so the run
        // completes claiming a setting that was never applied.
        let message = param_error(creds().param("contry", "us").build());
        assert!(message.contains("does not know the parameter"), "{message}");
    }

    #[test]
    fn an_empty_value_is_refused() {
        // The gateway does not reply at all; the connection hangs about 20 s.
        let message = param_error(creds().param("country", "").build());
        assert!(message.contains("empty value"), "{message}");
    }

    #[test]
    fn a_separator_inside_a_value_is_refused() {
        // "us-east" would be parsed as country=us plus a parameter named east.
        let message = param_error(creds().param("country", "us-east").build());
        assert!(message.contains("separate parameters"), "{message}");
    }
}

mod the_port {
    use super::*;

    #[test]
    fn zero_is_refused_by_naming_the_port_and_not_the_address() {
        // Found 2026-09-08 in an external review of the Python SDK, present here
        // in the same shape, and fixed in both on the same day. `port(0)` was
        // filtered out and then reported through the address arm as "no gateway
        // address for NodeMaven: pass host() and port() to the builder", which
        // is a sentence about the two things the caller had already done.
        //
        // The second assertion is the one that would have caught it. The first
        // passes on the old code too, because the old message happens to contain
        // the word "port" - so a test written only for "does it fail" would have
        // been green through the whole defect.
        let message = credentials_error(creds().port(0).build());
        assert!(message.contains("not a TCP port"), "{message}");
        assert!(!message.contains("no gateway address"), "{message}");
        assert!(message.contains("1 to 65535"), "{message}");
    }

    #[test]
    fn a_missing_address_still_says_so() {
        // The other half of the case above, and it had no test at all before
        // 2026-09-08 - only the two negative assertions that were just written,
        // which is a shape worth naming: an arm asserted about only by
        // `!contains` is an arm nobody has seen fire. If splitting zero out had
        // made this unreachable, every test in the file would still have passed.
        //
        // A provider that declares neither, so there is nothing to fall back to.
        // The environment cannot interfere: the variables are named from the
        // provider id, so this path reads `MINE_HOST` and `MINE_PORT`.
        let mine = nodemaven::Provider::builder("mine", "My proxy")
            .build()
            .unwrap();
        let message = credentials_error(
            Proxy::builder()
                .provider(mine)
                .login("u")
                .password("p")
                .build(),
        );
        assert!(message.contains("no gateway address"), "{message}");
        assert!(message.contains("MINE_HOST"), "{message}");
    }

    #[test]
    fn the_ports_a_caller_actually_uses_still_work() {
        // The control. A guard that refuses a real port is worse than the defect
        // it replaces, and 1 and 65535 are the two a range check is most likely
        // to get wrong. The assertion reads `server` because there is no `port`
        // accessor: the address is the only public place the value appears.
        for port in [1u16, 8080, 65535] {
            let proxy = creds().port(port).build().expect("a real port");
            assert_eq!(proxy.server(), format!("gate.example.com:{port}"));
        }
    }

    #[test]
    fn the_builder_type_is_what_rules_out_the_rest() {
        // Worth stating rather than leaving as an absent test. Python has to
        // refuse `port='abc'`, `port='8o80'`, `port='\u{b2}'`, `port=-1` and
        // `port=70000` at run time, and each of those was a case in
        // `test_proxy.py`. Here `port()` takes a `u16`, so none of them compiles
        // and zero is the only value left to check - which is why the module
        // above it is one case long and the Python one is five.
        //
        // The environment path has no such type, and every one of those cases
        // comes back there. They are in `tests/env.rs`.
        let _: fn(ProxyBuilder, u16) -> ProxyBuilder = ProxyBuilder::port;
    }
}

mod the_url_is_safe_to_hand_to_a_client {
    use super::*;

    #[test]
    fn credentials_are_percent_encoded() {
        let proxy = Proxy::builder()
            .login("acct")
            .password("pa/ss@1:2")
            .host("gate.example.com")
            .port(8080)
            .param("country", "us")
            .build()
            .unwrap();
        assert_eq!(
            proxy.url(),
            "http://acct-country-us:pa%2Fss%401%3A2@gate.example.com:8080"
        );
    }

    #[test]
    fn the_slash_is_encoded_and_that_is_the_whole_point() {
        // Unencoded, the authority ends at the slash and the host becomes "pa".
        let proxy = creds().password("pa/ss").build().unwrap();
        assert!(proxy.url().contains("%2F"), "{}", proxy.url());
        assert!(proxy.url().ends_with("@gate.example.com:8080"));
    }

    #[test]
    fn the_unreserved_set_matches_the_python_sdk() {
        // Python's quote(text, safe="") leaves `-._~` alone and encodes the
        // rest. A dependency's NON_ALPHANUMERIC set would encode all four, and
        // the two SDKs would then disagree on a url built from one username.
        let proxy = creds().password("-._~ +").build().unwrap();
        assert!(proxy.url().contains(":-._~%20%2B@"), "{}", proxy.url());
    }

    #[test]
    fn the_browser_fields_are_not_encoded() {
        // Playwright and chromiumoxide encode the fields themselves; encoding
        // here too would send pa%252Fss and fail authentication while blaming
        // the credentials.
        let browser = creds().password("pa/ss").build().unwrap().browser();
        assert_eq!(browser.password, "pa/ss");
        assert_eq!(browser.server, "http://gate.example.com:8080");
    }
}

mod the_debug_output_carries_no_secret {
    use super::*;

    #[test]
    fn the_password_is_redacted() {
        let proxy = creds()
            .password("hunter2")
            .param("country", "us")
            .build()
            .unwrap();
        let shown = format!("{proxy:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("***"), "{shown}");
    }

    #[test]
    fn it_survives_being_put_in_a_container() {
        // A container's Debug calls Debug on its elements, which is how a
        // careful impl gets bypassed in a log line.
        let proxy = creds().password("hunter2").build().unwrap();
        assert!(!format!("{:?}", vec![&proxy]).contains("hunter2"));
        assert!(!format!("{:?}", Some(&proxy)).contains("hunter2"));
        assert!(!format!("{:?}", proxy.browser()).contains("hunter2"));
    }

    #[test]
    fn the_builder_does_not_carry_it_either() {
        // The builder holds the password for as long as the chain is being
        // written, and a `dbg!` halfway through a chain is a normal thing to do.
        let builder = creds().password("hunter2");
        assert!(!format!("{builder:?}").contains("hunter2"));
    }

    #[test]
    fn the_parameters_are_still_visible() {
        let proxy = creds().param("country", "us").build().unwrap();
        assert!(format!("{proxy:?}").contains(r#"country: "us""#));
    }
}

mod moving_is_a_new_identity {
    use super::*;

    #[test]
    fn with_param_returns_a_new_object() {
        let first = creds().param("country", "us").build().unwrap();
        let second = first.with_param("country", "de").unwrap();
        assert_eq!(first.username(), "acct-country-us");
        assert_eq!(second.username(), "acct-country-de");
    }

    #[test]
    fn a_changed_parameter_keeps_its_place_in_the_username() {
        // The position is part of the string, and the string is what the
        // gateway may or may not be keying the session on.
        let proxy = creds()
            .param("country", "us")
            .param("filter", "medium")
            .build()
            .unwrap()
            .with_param("country", "de")
            .unwrap();
        assert_eq!(proxy.username(), "acct-country-de-filter-medium");
    }

    #[test]
    fn without_param_removes_one() {
        let proxy = creds()
            .param("country", "us")
            .param("filter", "medium")
            .build()
            .unwrap()
            .without_param("filter")
            .unwrap();
        assert_eq!(proxy.username(), "acct-country-us");
    }

    #[test]
    fn removing_a_parameter_that_was_not_set_is_a_no_op() {
        let proxy = creds().param("country", "us").build().unwrap();
        assert_eq!(
            proxy.without_param("filter").unwrap().username(),
            "acct-country-us"
        );
    }

    #[test]
    fn session_uses_the_providers_own_name_for_it() {
        let proxy = creds().param("country", "us").build().unwrap();
        assert_eq!(
            proxy.session("order4417").unwrap().username(),
            "acct-country-us-sid-order4417"
        );
    }

    #[test]
    fn a_session_id_carrying_the_separator_is_refused() {
        // `order-4417` is the obvious thing to use as a session key and this
        // dialect cannot carry it: separator and pair separator are both "-",
        // so `sid-order-4417` reads as sid=order followed by a parameter named
        // 4417. Measured 2026-08-20 rather than assumed: tunnels opened with
        // `sid-order<x>-4417` and with `sid-order<x>` landed on one exit across
        // four interleaved rounds each, while `sid-order<x>4417` held another
        // one - so the gateway cuts the tail off and every order id beginning
        // `order` would silently share a session.
        let proxy = creds().param("country", "us").build().unwrap();
        let message = param_error(proxy.session("order-4417"));
        assert!(message.contains("cannot carry"), "{message}");
    }

    #[test]
    fn a_provider_with_no_session_parameter_refuses_rather_than_guesses() {
        let provider = load_str("label = \"P\"\nknown_params = [\"country\"]\n", "p").unwrap();
        let proxy = creds().provider(provider).build().unwrap();
        let message = param_error(proxy.session("order4417"));
        assert!(
            message.contains("declares no session parameter"),
            "{message}"
        );
    }
}

mod legal_values {
    //! The values half of the check, and the reason it is off by default.
    //!
    //! Names are validated against `known_params`; values are validated against
    //! `values`, which is empty for the shipped definition. The mechanism is
    //! here before the data is, because four language SDKs parse this schema and
    //! a key added later is four parsers rather than one data file.

    use super::*;

    const WITH_VALUES: &str = "\
label = \"P\"
known_params = [\"country\", \"filter\"]
values = { filter = [\"medium\", \"high\"] }
";

    #[test]
    fn an_unlisted_parameter_is_not_value_checked() {
        // No entry means nobody established the legal values, so the caller is
        // left exactly where they were rather than being refused on a guess.
        let provider = load_str(WITH_VALUES, "p").unwrap();
        let proxy = creds()
            .provider(provider)
            .param("country", "whatever")
            .build()
            .unwrap();
        assert_eq!(proxy.username(), "acct-country-whatever");
    }

    #[test]
    fn a_value_outside_the_list_is_refused() {
        let provider = load_str(WITH_VALUES, "p").unwrap();
        let message = param_error(creds().provider(provider).param("filter", "medim").build());
        assert!(message.contains("is not a value"), "{message}");
    }

    #[test]
    fn a_value_inside_the_list_passes() {
        let provider = load_str(WITH_VALUES, "p").unwrap();
        let proxy = creds()
            .provider(provider)
            .param("filter", "high")
            .build()
            .unwrap();
        assert_eq!(proxy.username(), "acct-filter-high");
    }

    #[test]
    fn values_for_an_unknown_parameter_are_refused_at_load() {
        // Otherwise the entry looks like a working check and never runs.
        let message = provider_error(load_str(
            "label = \"P\"\nknown_params = [\"country\"]\nvalues = { filter = [\"medium\"] }\n",
            "p",
        ));
        assert!(message.contains("not in known_params"), "{message}");
    }

    #[test]
    fn an_empty_list_is_refused_at_load() {
        // It would mean "every value of this parameter is illegal". Leaving the
        // entry out is how you say "not established".
        let message = provider_error(load_str(
            "label = \"P\"\nknown_params = [\"filter\"]\nvalues = { filter = [] }\n",
            "p",
        ));
        assert!(message.contains("non-empty list"), "{message}");
    }

    #[test]
    fn a_hand_built_definition_goes_through_the_same_checks() {
        // The builder is the path the README's "Other gateways" example uses
        // for a proxy from somewhere else, so it cannot be the lenient one.
        let message = provider_error(
            nodemaven::Provider::builder("mine", "My proxy")
                .known_params(["country"])
                .session_param("sid")
                .build(),
        );
        assert!(
            message.contains("every sticky session would be refused"),
            "{message}"
        );
    }
}

mod what_a_status_means_on_this_gateway {
    //! `connect_reactions`, the third schema key added before the vectors freeze.
    //!
    //! A status code on this gateway is not a diagnosis. Measured 2026-09-08: a
    //! value the gateway will not take on `country`, `filter`, `ttl`, `type` or
    //! `speed` is answered **407**, which sends the caller to check credentials
    //! that are correct. The translation is per-gateway dialect, so it lives in
    //! the TOML beside the separators.
    //!
    //! The table was in this crate's copy of the definition for a few hours with
    //! no parser behind it, because the schema is one file and Python added the
    //! key first. The Rust parser reads named keys out of a `toml::Table` with no
    //! serde derive, so an unknown table is ignored - the safe direction, and a
    //! silent one. These tests are what stops it being silent again.

    use super::*;

    #[test]
    fn the_shipped_definition_explains_the_status_it_is_misleading_about() {
        let provider = load(nodemaven::DEFAULT_PROVIDER).unwrap();
        let meaning = provider.reaction(407).expect("407 is described");
        assert!(!meaning.is_empty());
        // The whole point of the entry: a 407 here is not necessarily about the
        // password, and a caller told to check their credentials spends the next
        // hour on the one thing that is correct.
        assert!(
            provider.reaction(200).is_some(),
            "200 needs an entry too, because `ok` does not mean the parameters \
             were applied"
        );
    }

    #[test]
    fn each_reaction_names_the_parameters_it_can_come_from() {
        // Naming the parameter is the only thing the code itself does not do,
        // so an entry that names none is what this pins. It is deliberately
        // not "four parameters, four codes": measured 2026-09-08, `region` and
        // `isp` both answer 406, so 406 has to name both of them.
        let provider = load(nodemaven::DEFAULT_PROVIDER).unwrap();
        let ambiguous = provider.reaction(406).unwrap();
        assert!(ambiguous.contains("region"), "{ambiguous}");
        assert!(ambiguous.contains("isp"), "{ambiguous}");
        assert!(provider.reaction(500).unwrap().contains("city"));
        assert!(provider.reaction(410).unwrap().contains("isp"));
        assert!(provider.reaction(407).unwrap().contains("country"));
    }

    #[test]
    fn the_406_does_not_claim_to_know_which_parameter_it_means() {
        // The table is asked to report an ambiguity rather than a diagnosis in
        // exactly one place, and this is now that place. It used to be the 410,
        // on a reading the run of 2026-09-08 overturned: a junk `isp` answers
        // 406 and `comcast` answers 410, so the codes *do* separate those two.
        // What 406 does not separate is `region` from `isp`, or `charter` - a
        // real ISP - from a value nobody implemented, since all three produce
        // it. Most likely row to be tidied into a confident sentence later.
        let provider = load(nodemaven::DEFAULT_PROVIDER).unwrap();
        let meaning = provider.reaction(406).expect("406 is described");
        assert!(meaning.contains("charter"), "{meaning}");
        assert!(meaning.contains("unavailable"), "{meaning}");
    }

    #[test]
    fn the_410_says_how_narrow_its_sample_is() {
        // One ISP value has ever produced a 410. An entry reading as a rule
        // about real-versus-junk names would be this crate generalising from a
        // sample of one, which is the shape of most corrections in its history.
        let provider = load(nodemaven::DEFAULT_PROVIDER).unwrap();
        let meaning = provider.reaction(410).expect("410 is described");
        assert!(meaning.contains("comcast"), "{meaning}");
        assert!(meaning.contains("one value"), "{meaning}");
    }

    #[test]
    fn a_status_nobody_described_is_none_rather_than_an_error() {
        let provider = load(nodemaven::DEFAULT_PROVIDER).unwrap();
        assert_eq!(provider.reaction(418), None);
        // The table is only as long as the measurements. A default sentence
        // here would be this crate inventing a diagnosis, which is the thing it
        // exists to stop the gateway doing.
        assert_eq!(provider.reaction(502), None);
    }

    #[test]
    fn the_key_is_formatted_from_the_number_and_not_looked_up_as_one() {
        // Keys are strings in the file for a reason that is not about Rust: TOML
        // has no integer keys and neither does JSON, and the golden vectors are
        // JSON. So `407` and `"407"` have to reach the same entry, in all four
        // SDKs, and the conversion belongs in the lookup.
        let provider = load_str(
            "label = \"P\"\nknown_params = [\"country\"]\n\
             [connect_reactions]\n407 = \"refused\"\n",
            "p",
        )
        .unwrap();
        assert_eq!(provider.reaction(407), Some("refused"));
        assert_eq!(
            provider.connect_reactions().get("407").map(String::as_str),
            Some("refused")
        );
    }

    #[test]
    fn an_empty_explanation_is_refused_at_load() {
        // The same rule as an empty `values` list and for the same reason: a
        // present-but-blank entry reads as "this status is explained" and
        // explains nothing. Worse here, because the text is shown to a caller as
        // the reason their connection was refused.
        let message = provider_error(load_str(
            "label = \"P\"\nknown_params = [\"country\"]\n\
             [connect_reactions]\n407 = \"\"\n",
            "p",
        ));
        assert!(message.contains("non-empty string"), "{message}");
    }

    #[test]
    fn a_status_a_u16_cannot_hold_loads_and_is_simply_never_found() {
        // Deliberately not refused. A file describing a status outside 1..=65535
        // is describing something that is not an HTTP status, and refusing it at
        // load would turn a dead entry - which the lookup never finds - into a
        // definition that will not load at all. The narrow type is enforced
        // where a status comes off the wire, in `check.rs`, which is the only
        // place a real one appears.
        let provider = load_str(
            "label = \"P\"\nknown_params = [\"country\"]\n\
             [connect_reactions]\n99999 = \"unreachable\"\nnonsense = \"also\"\n",
            "p",
        )
        .unwrap();
        assert_eq!(provider.reaction(407), None);
        assert_eq!(provider.connect_reactions().len(), 2);
    }

    #[test]
    fn a_table_that_is_not_a_table_is_refused_at_load() {
        let message = provider_error(load_str(
            "label = \"P\"\nknown_params = [\"country\"]\nconnect_reactions = \"nope\"\n",
            "p",
        ));
        assert!(message.contains("not a table"), "{message}");
    }

    #[test]
    fn a_definition_may_describe_no_status_at_all() {
        // Unlike `values`, an entry here cannot refuse anything, so an absent
        // table costs a caller nothing but an unexplained status code.
        let provider = load_str("label = \"P\"\nknown_params = [\"country\"]\n", "p").unwrap();
        assert!(provider.connect_reactions().is_empty());
        assert_eq!(provider.reaction(407), None);
    }

    #[test]
    fn a_hand_built_definition_can_describe_a_status() {
        let provider = nodemaven::Provider::builder("mine", "My proxy")
            .known_params(["country"])
            .connect_reaction(407, "check the password, this gateway means it")
            .build()
            .unwrap();
        assert_eq!(
            provider.reaction(407),
            Some("check the password, this gateway means it")
        );
    }
}

mod the_shipped_definition {
    use super::*;

    #[test]
    fn nodemaven_is_shipped_and_is_measured() {
        assert!(available().contains(&"nodemaven"));
        assert!(load("nodemaven").unwrap().is_measured());
    }

    #[test]
    fn norotate_is_refused() {
        // The Python SDK asserted the opposite between 2026-08-21 and
        // 2026-08-26, on the strength of `norotate` appearing in the vendor's
        // proxy generator. Probed from the VPS on 2026-08-26: with
        // `norotate=true` and no `sid` the gateway hands out 6 distinct exits in
        // 6 draws, and with a fixed `sid` and `ttl=1m` it hands out 3 distinct
        // in 3 - the same as a negative control carrying a name nobody has
        // implemented. It is answered 200 and dropped, which is the exact
        // failure this crate exists to catch, so it belongs on the refused side.
        assert!(!load("nodemaven")
            .unwrap()
            .known_params()
            .contains("norotate"));
        let message = param_error(creds().param("norotate", "true").build());
        assert!(message.contains("norotate"), "{message}");
    }

    #[test]
    fn no_value_is_checked_for_the_shipped_definition_yet() {
        // Guards the difference between "not established" and "nothing is
        // legal". If this ever fails, somebody filled in `values` - which is
        // wanted, but the vectors and the README claim have to move with it.
        assert!(load("nodemaven").unwrap().allowed("filter").is_none());
    }

    #[test]
    fn the_session_parameter_is_asked_for_rather_than_spelled() {
        // Eleven call sites in the benchmark wrote "sid" directly. It is the
        // name this gateway happens to use, which is exactly why the literal
        // survived.
        assert_eq!(load("nodemaven").unwrap().session_param(), Some("sid"));
    }

    #[test]
    fn the_gateway_address_comes_from_the_definition() {
        assert_eq!(
            load("nodemaven").unwrap().host(),
            Some("gate.nodemaven.com")
        );
        assert_eq!(load("nodemaven").unwrap().port(), Some(8080));
    }

    #[test]
    fn a_definition_that_is_not_shipped_says_what_is() {
        let message = provider_error(load("oxylabs"));
        assert!(message.contains("load_file()"), "{message}");
    }
}

/// The README quotes output. These pin the three places it does, because a
/// README that shows a message the code no longer produces is worse than one
/// that shows none: it is the first thing a reader tries and the only part of
/// the documentation they will trust afterwards. The wording of the first was
/// wrong in the README when it was written - it dropped the brackets and quotes
/// that `{:?}` on a list produces - and it was caught by printing it rather than
/// by reading the format string.
mod the_readme_shows_real_output {
    use super::*;
    use nodemaven::Provider;

    #[test]
    fn the_unknown_parameter_message_is_quoted_verbatim() {
        let message = param_error(
            Proxy::builder()
                .login("u")
                .password("p")
                .param("contry", "us")
                .build(),
        );
        assert_eq!(
            message,
            "NodeMaven does not know the parameter \"contry\": it is answered with 200 and \
             dropped, so the connection would succeed and your setting would NOT be applied. \
             Known: [\"city\", \"country\", \"filter\", \"ipv4\", \"isp\", \"region\", \"sid\", \
             \"speed\", \"ttl\", \"type\"]"
        );
    }

    #[test]
    fn a_provider_declaring_nothing_builds_and_refuses_every_parameter() {
        // The in-place `Provider::builder` snippet under "Other gateways". A
        // gateway nobody has established is `documented`, never `measured`.
        let mine = Provider::builder("mine", "My proxy").build().unwrap();
        assert!(!mine.is_measured());
        let message = param_error(
            Proxy::builder()
                .provider(mine)
                .login("u")
                .password("p")
                .host("proxy.example.com")
                .port(8000)
                .param("country", "us")
                .build(),
        );
        assert!(message.contains("My proxy does not know"), "{message}");
    }

    #[test]
    fn the_five_line_toml_produces_the_username_the_readme_claims() {
        let mine = load_str(
            "label = \"My proxy\"\n\
             known_params = [\"country\", \"session\"]\n\
             session_param = \"session\"\n\
             host = \"proxy.example.com\"\n\
             port = 8000\n",
            "my-gateway",
        )
        .unwrap();
        let proxy = Proxy::builder()
            .provider(mine)
            .login("u")
            .password("p")
            .param("country", "us")
            .build()
            .unwrap();
        assert_eq!(
            proxy.session("order4417").unwrap().username(),
            "u-country-us-session-order4417"
        );
    }
}

/// The fold to the wire form, and the refusal that completes it.
///
/// The evidence for the fold is in the provider TOML: a username this gateway
/// generated for a real account carries `region-district_of_columbia`, and the
/// vendor's own client applies the same transformation. So these cases assert a
/// form the gateway has been seen to emit, not one that looked tidy.
///
/// The refusal is the other half. A parameter nobody folds cannot carry
/// whitespace either, because there is no spelling of a space in a proxy
/// username that works - so between the two, no value with whitespace in it can
/// reach the wire by any path.
mod values_are_folded_to_the_form_the_gateway_emits {
    use super::*;

    /// The case this whole module exists for: a username the gateway itself
    /// generated, reproduced parameter for parameter from a caller writing the
    /// region the way a person writes it.
    ///
    /// The login is `acct` here and the real one is not written down anywhere in
    /// this repository. Everything to the right of it is the generated string
    /// unchanged, and that part is what carries the evidence - before the fold
    /// this same call produced `region-District of Columbia`, which is not a
    /// thing that can be sent.
    #[test]
    fn it_reproduces_a_username_the_gateway_generated() {
        let proxy = creds()
            .param("country", "us")
            .param("region", "District of Columbia")
            .param("sid", "bfd1c859433a4")
            .param("filter", "medium")
            .build()
            .unwrap();
        assert_eq!(
            proxy.username(),
            "acct-country-us-region-district_of_columbia-sid-bfd1c859433a4-filter-medium"
        );
    }

    #[test]
    fn a_space_becomes_an_underscore() {
        let proxy = creds()
            .param("region", "District of Columbia")
            .build()
            .unwrap();
        assert_eq!(proxy.username(), "acct-region-district_of_columbia");
    }

    #[test]
    fn case_is_folded() {
        let proxy = creds().param("country", "US").build().unwrap();
        assert_eq!(proxy.username(), "acct-country-us");
    }

    /// The folded value is what the object reports, not just what it sends. A
    /// proxy answering `District of Columbia` while sending
    /// `district_of_columbia` would let two callers with the same visible
    /// configuration sit on different sticky sessions and see no reason why.
    #[test]
    fn the_stored_value_is_the_folded_one() {
        let proxy = creds()
            .param("city", "New York")
            .param("country", "US")
            .build()
            .unwrap();
        assert_eq!(
            proxy.params(),
            &[
                ("city".to_string(), "new_york".to_string()),
                ("country".to_string(), "us".to_string()),
            ]
        );
    }

    /// And it is idempotent, which is what makes `with_param` safe to chain.
    #[test]
    fn folding_a_folded_value_changes_nothing() {
        let once = creds().param("city", "New York").build().unwrap();
        let twice = once.with_param("city", once.params()[0].1.clone()).unwrap();
        assert_eq!(once.username(), twice.username());
    }

    #[test]
    fn surrounding_whitespace_is_trimmed_rather_than_refused() {
        let proxy = creds().param("country", "  US  ").build().unwrap();
        assert_eq!(proxy.username(), "acct-country-us");
    }

    /// A value that is nothing but whitespace trims to empty, and the empty
    /// refusal is the more useful of the two messages: the gateway does not
    /// answer an empty value at all, it hangs for twenty seconds.
    #[test]
    fn a_value_of_only_whitespace_is_refused_as_empty() {
        let message = param_error(creds().param("country", "   ").build());
        assert!(message.contains("empty value"), "{message}");
    }

    #[test]
    fn a_parameter_nobody_folds_refuses_whitespace_instead() {
        let message = param_error(creds().param("sid", "order 4417").build());
        assert!(message.contains("contains whitespace"), "{message}");
        assert!(
            message.contains(r#"["city", "country", "isp", "region", "type"]"#),
            "the message should name what is folded, and it says: {message}"
        );
    }

    /// A tab and a newline are refused for the same reason as a space, and the
    /// set is spelled out in the crate rather than delegated - Rust's
    /// `is_ascii_whitespace` excludes the vertical tab and Python's
    /// `str.isspace` includes it, so a shared contract cannot use either name.
    #[test]
    fn every_ascii_whitespace_character_is_refused() {
        for bad in [' ', '\t', '\n', '\r', '\u{0b}', '\u{0c}'] {
            let message = param_error(creds().param("sid", format!("a{bad}b")).build());
            assert!(
                message.contains("contains whitespace"),
                "{bad:?} was not refused: {message}"
            );
        }
    }

    /// Unicode whitespace is *not* refused, and that is deliberate rather than
    /// an oversight. A no-break space is a character the gateway has never been
    /// asked about, and the ASCII set is the one four languages can agree on
    /// without depending on their Unicode tables. It is caught, if at all, as a
    /// value the gateway rejects - which is the honest outcome for input nobody
    /// has measured.
    #[test]
    fn a_no_break_space_is_not_ascii_whitespace_and_passes() {
        let proxy = creds().param("sid", "a\u{a0}b").build().unwrap();
        assert_eq!(proxy.username(), "acct-sid-a\u{a0}b");
    }

    /// `sid` is excluded from the fold on purpose: a session id is opaque and
    /// caller-chosen, and lowercasing it would silently move the caller to a
    /// different sticky session than the one they named.
    #[test]
    fn a_session_id_keeps_its_case() {
        let proxy = creds().param("sid", "Order4417").build().unwrap();
        assert_eq!(proxy.username(), "acct-sid-Order4417");
        let held = creds().build().unwrap().session("Order4417").unwrap();
        assert_eq!(held.username(), "acct-sid-Order4417");
    }

    /// So does `filter`. The vendor's client does not fold it either, and the
    /// generated username that evidences the fold says nothing about it because
    /// `medium` was already lower case. Folding it would be a guess.
    #[test]
    fn filter_and_ttl_keep_their_case() {
        let proxy = creds()
            .param("filter", "MEDIUM")
            .param("ttl", "10M")
            .build()
            .unwrap();
        assert_eq!(proxy.username(), "acct-filter-MEDIUM-ttl-10M");
    }

    /// The fold is ASCII-only at every step. `I` with a dot above lower-cases to
    /// two code points under full Unicode rules in some languages and to one in
    /// others, so a vector built on it would fail in one SDK for a reason that
    /// has nothing to do with proxies.
    #[test]
    fn the_fold_is_ascii_only() {
        let proxy = creds().param("city", "\u{130}stanbul").build().unwrap();
        assert_eq!(proxy.username(), "acct-city-\u{130}stanbul");
    }

    /// A definition that folds nothing gets neither half of this: no fold, and
    /// the whitespace refusal still applies, because that one is about what a
    /// username can carry rather than about any gateway's dialect.
    #[test]
    fn a_definition_that_folds_nothing_still_refuses_whitespace() {
        let toml = "label = \"Plain\"\nknown_params = [\"country\"]\n";
        let built = creds()
            .provider(load_str(toml, "plain").unwrap())
            .param("country", "US")
            .build()
            .unwrap();
        assert_eq!(built.username(), "acct-country-US");

        let message = param_error(
            creds()
                .provider(load_str(toml, "plain").unwrap())
                .param("country", "New York")
                .build(),
        );
        assert!(message.contains("contains whitespace"), "{message}");
    }
}

/// A definition that declares a fold it cannot honour is refused when it loads,
/// not when somebody calls it. Same principle as the three checks already there:
/// a declaration that reads like a working setting and can never fire is the
/// class of mistake this crate exists to make loud.
mod a_definition_cannot_declare_an_impossible_fold {
    use super::*;

    #[test]
    fn normalizing_a_parameter_that_is_not_known_is_refused() {
        let message = provider_error(load_str(
            "label = \"Wrong\"\nknown_params = [\"country\"]\nnormalize = [\"country\", \"city\"]\n",
            "wrong",
        ));
        assert!(message.contains("normalizes"), "{message}");
        assert!(message.contains("city"), "{message}");
    }

    /// The fold inserts an underscore, so a gateway that separates on one would
    /// have the value cut in half by the very step meant to make it sendable.
    /// The caller would be blamed for input that is correct.
    #[test]
    fn folding_into_the_separator_is_refused() {
        let message = provider_error(load_str(
            "label = \"Underscored\"\nknown_params = [\"city\"]\nseparator = \"_\"\nnormalize = [\"city\"]\n",
            "underscored",
        ));
        assert!(message.contains("Pick one"), "{message}");
    }
}
