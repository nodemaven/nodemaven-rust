//! What one CONNECT produces, measured against a socket on loopback.
//!
//! Every assertion here is about bytes that crossed a real socket. Nothing is
//! mocked, no live host is touched, no traffic is spent, and nothing routes
//! through the Happ tun gateway this workstation sits behind - which is the only
//! way this module can be tested here at all.
//!
//! What a real socket buys over a mocked one, and it is the whole reason for the
//! thread below: the exact wire bytes are pinned, including the
//! `Proxy-Authorization` header, so the hand-rolled base64 is checked against
//! output the Python SDK produced rather than against itself; and the credential
//! is asserted **absent** from every failure message and every accessor, which a
//! happy-path test cannot do.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use nodemaven::{Check, Connect, Error, Proxy};

const ESTABLISHED: &[u8] = b"HTTP/1.1 200 Connection established\r\n\
                             X-Proxy-Exit-IP: 203.0.113.7\r\n\
                             \r\n";

const REFUSED: &[u8] = b"HTTP/1.1 407 Proxy Authentication Required\r\n\
                         Proxy-Authenticate: Basic realm=\"gate\"\r\n\
                         \r\n";

/// One CONNECT served on loopback, and the request head it was sent.
struct Gateway {
    address: String,
    request: Receiver<Vec<u8>>,
}

impl Gateway {
    /// Bind, accept once, record the request head, answer with `answer`.
    ///
    /// An empty `answer` is the "accepted and then closed without saying
    /// anything" case, which is a reaction this gateway is not known to have and
    /// which the crate has to survive.
    ///
    /// Takes anything that becomes a `Vec<u8>` rather than a `&'static [u8]`, so
    /// a case can build its answer - a status line with one version substituted
    /// into it, say - instead of every variant having to be a `const`.
    fn answering(answer: impl Into<Vec<u8>>) -> Gateway {
        let answer = answer.into();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let address = listener.local_addr().expect("local address").to_string();
        let (sender, request) = mpsc::channel();
        thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut head = Vec::new();
            let mut chunk = [0u8; 4096];
            while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => head.extend_from_slice(&chunk[..read]),
                }
            }
            let _ = sender.send(head);
            // The error is ignored on purpose and it is not laziness: the
            // bounded-head test answers with more than the crate will read, so
            // the client closes the connection mid-write. That is the behaviour
            // under test, not a failure of the fake.
            let _ = stream.write_all(&answer);
        });
        Gateway { address, request }
    }

    /// The request head as it arrived, decoded latin-1 so nothing can panic.
    fn request(&self) -> String {
        self.request
            .recv_timeout(Duration::from_secs(5))
            .expect("the gateway thread recorded a request")
            .iter()
            .map(|&byte| char::from(byte))
            .collect()
    }
}

/// A `Connect` at this address with a short timeout, since nothing here hangs.
fn connect(gateway: &Gateway) -> Connect {
    Connect::new(&gateway.address, "acct", "pw").timeout(Duration::from_secs(5))
}

fn reactions() -> BTreeMap<String, String> {
    let mut table = BTreeMap::new();
    table.insert(
        "407".to_string(),
        "The gateway refuses a bad parameter value this way too, so this is not \
         necessarily about your credentials."
            .to_string(),
    );
    table
}

fn message(error: Error) -> String {
    error.to_string()
}

mod what_the_gateway_said {
    use super::*;

    #[test]
    fn a_200_with_the_exit_header_yields_the_exit_address() {
        let gateway = Gateway::answering(ESTABLISHED);
        let result = connect(&gateway)
            .exit_ip_header("X-Proxy-Exit-IP")
            .reactions(reactions())
            .send()
            .expect("a 200 came back");
        assert!(result.ok());
        assert_eq!(result.status(), 200);
        assert_eq!(result.exit_ip(), Some("203.0.113.7"));
        // One CONNECT and nothing else: the exit address arrived on the reply
        // itself, so it cost no target traffic. That is the whole reason this
        // module exists rather than a GET through the tunnel.
        assert!(result.elapsed() >= Duration::ZERO);
    }

    #[test]
    fn the_reason_phrase_is_kept_verbatim() {
        // Measured 2026-08-13: on the shipped gateway a 200 carrying the exit
        // header arrives as `Connection established`, while the ones arriving as
        // `OK` or `Connection Established` do not carry it. The phrase labels
        // which back end answered, so normalising it - even just its case -
        // destroys the only key a per-implementation figure can be split on.
        let gateway = Gateway::answering(b"HTTP/1.1 200 Connection Established\r\n\r\n");
        let result = connect(&gateway).send().expect("a 200 came back");
        assert_eq!(result.reason(), "Connection Established");
        assert_eq!(result.exit_ip(), None);
    }

    #[test]
    fn a_missing_exit_header_is_normal_and_not_an_error() {
        let gateway = Gateway::answering(b"HTTP/1.1 200 OK\r\n\r\n");
        let result = connect(&gateway)
            .exit_ip_header("X-Proxy-Exit-IP")
            .send()
            .expect("a 200 came back");
        assert!(result.ok());
        assert_eq!(result.exit_ip(), None);
    }

    #[test]
    fn headers_are_lowercased_and_kept_in_arrival_order() {
        let gateway = Gateway::answering(
            b"HTTP/1.1 200 Connection established\r\n\
              X-Proxy-Exit-IP: 203.0.113.7\r\n\
              Via: 1.1 something\r\n\
              \r\n",
        );
        let result = connect(&gateway).send().expect("a 200 came back");
        assert_eq!(result.header("x-proxy-exit-ip"), Some("203.0.113.7"));
        assert_eq!(result.header("X-Proxy-Exit-IP"), Some("203.0.113.7"));
        assert_eq!(result.header("via"), Some("1.1 something"));
        assert_eq!(
            result.headers(),
            [
                ("x-proxy-exit-ip".to_string(), "203.0.113.7".to_string()),
                ("via".to_string(), "1.1 something".to_string()),
            ]
        );
    }

    #[test]
    fn a_repeated_header_keeps_both_and_looks_up_the_last() {
        // Python stores headers in a `dict`, so the second value overwrites the
        // first and the first is gone. Here both are kept, because a duplicated
        // header is itself worth seeing when the question is which middlebox
        // answered - and the lookup returns the last one, so every `header()`
        // call still agrees with the Python SDK.
        let gateway = Gateway::answering(b"HTTP/1.1 200 OK\r\nVia: first\r\nVia: second\r\n\r\n");
        let result = connect(&gateway).send().expect("a 200 came back");
        assert_eq!(result.header("via"), Some("second"));
        assert_eq!(result.headers().len(), 2);
    }

    #[test]
    fn a_header_name_is_lowercased_ascii_only() {
        // A portability trap rather than a live bug - the shipped gateway's
        // header is `X-Proxy-Exit-IP` and every byte of it is ASCII. The head is
        // decoded latin-1 on purpose, so byte 0xC0 arrives as U+00C0; Python's
        // `str.lower()` folds it to U+00E0 and `to_ascii_lowercase` does not, so
        // the two SDKs would key one response two ways. RFC 9110 makes a field
        // name a token and a token is ASCII, so ASCII-only is the correct rule
        // as well as the portable one. Python was changed to match this on
        // 2026-09-07, rather than this being bent to Python.
        let gateway = Gateway::answering(b"HTTP/1.1 200 OK\r\n\xc0-Vendor: yes\r\n\r\n");
        let result = connect(&gateway).send().expect("a 200 came back");
        assert_eq!(result.header("\u{c0}-vendor"), Some("yes"));
        assert_eq!(result.header("\u{e0}-vendor"), None);
    }

    #[test]
    fn a_reason_phrase_carrying_a_byte_utf8_would_refuse_is_kept() {
        // `String::from_utf8` is the wrong call on a response head and this is
        // why: 0xFF is not valid utf-8, a proxy is entitled to send it, and
        // refusing here would turn a readable diagnosis into a decode error -
        // the same failure this module exists to avoid, by another route.
        // `from_utf8_lossy` is worse: it substitutes U+FFFD and the caller
        // cannot tell what arrived.
        let gateway = Gateway::answering(b"HTTP/1.1 200 caf\xff\r\n\r\n");
        let result = connect(&gateway).send().expect("a 200 came back");
        assert_eq!(result.reason(), "caf\u{ff}");
    }
}

mod a_refusal_is_a_value_and_not_an_error {
    use super::*;

    #[test]
    fn a_407_comes_back_as_a_check() {
        // Returning `Err` here would push the status - the thing the caller came
        // for - into something that has to be unwrapped. `requests` does exactly
        // that in Python, and the code survives only as text inside a nested
        // exception with the reason phrase and every header gone.
        let gateway = Gateway::answering(REFUSED);
        let result = connect(&gateway).send().expect("a 407 is not an error");
        assert!(!result.ok());
        assert_eq!(result.status(), 407);
        assert_eq!(result.reason(), "Proxy Authentication Required");
        assert_eq!(
            result.header("proxy-authenticate"),
            Some("Basic realm=\"gate\"")
        );
    }

    #[test]
    fn the_gateways_own_reading_of_a_407_is_attached() {
        let gateway = Gateway::answering(REFUSED);
        let result = connect(&gateway)
            .reactions(reactions())
            .send()
            .expect("a 407 is not an error");
        let meaning = result.meaning().expect("407 is in the table");
        assert!(meaning.contains("not necessarily about your credentials"));
        // And it is printed, because a caller who prints the check is the caller
        // who most needs telling not to go and check a correct password.
        assert!(result
            .to_string()
            .contains("not necessarily about your credentials"));
    }

    #[test]
    fn a_status_with_no_entry_has_no_meaning_and_that_is_not_an_error() {
        let gateway = Gateway::answering(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
        let result = connect(&gateway)
            .reactions(reactions())
            .send()
            .expect("a 502 is not an error");
        assert_eq!(result.meaning(), None);
        assert_eq!(result.status(), 502);
    }

    #[test]
    fn a_reaction_table_is_passed_whole_and_not_resolved_by_the_caller() {
        // The seam this pins: the meaning depends on the status and the status is
        // not known until the answer comes back. A caller who looked one up and
        // passed a single value would always pass nothing, the table would go
        // unread, and every other test here would still pass. The Python SDK
        // carries the same regression pin for the same reason.
        let gateway = Gateway::answering(REFUSED);
        let result = connect(&gateway)
            .reaction(407, "this text came from the table")
            .send()
            .expect("a 407 is not an error");
        assert_eq!(result.meaning(), Some("this text came from the table"));
    }

    #[test]
    fn ok_is_only_200() {
        let gateway = Gateway::answering(b"HTTP/1.1 201 Created\r\n\r\n");
        let result = connect(&gateway).send().expect("a 201 is not an error");
        assert!(!result.ok());
    }
}

mod when_nothing_came_back {
    use super::*;

    #[test]
    fn an_immediate_close_says_it_is_worth_reporting() {
        let gateway = Gateway::answering(b"");
        let error = connect(&gateway).send().expect_err("nothing came back");
        assert!(message(error).contains("without answering"));
    }

    #[test]
    fn something_that_is_not_a_status_line_is_refused() {
        // A captive portal, or something other than a proxy on that port.
        let gateway = Gateway::answering(b"<html>You must sign in</html>\r\n\r\n");
        let error = connect(&gateway).send().expect_err("not a status line");
        assert!(message(error).contains("not an HTTP status line"));
    }

    #[test]
    fn a_non_numeric_status_is_refused() {
        let gateway = Gateway::answering(b"HTTP/1.1 OK Fine\r\n\r\n");
        let error = connect(&gateway).send().expect_err("not a status line");
        assert!(message(error).contains("not an HTTP status line"));
    }

    #[test]
    fn a_latin1_superscript_status_is_refused() {
        // The defect this port found in the Python SDK on 2026-09-07 and the
        // reason `is_status_code` asks an ASCII question rather than a Unicode
        // one. `'\u{b2}'.isdigit()` is True in Python and `int()` on it raises;
        // Rust's `char::is_numeric` is True for it as well, so the same trap is
        // here under a different name. Reachable only because the head is
        // decoded latin-1 on purpose - a deliberate widening of the input
        // alphabet two functions away is what made the narrow check unsound.
        let answers: [&'static [u8]; 3] = [
            b"HTTP/1.1 \xb9 something\r\n\r\n",
            b"HTTP/1.1 \xb2 something\r\n\r\n",
            b"HTTP/1.1 \xb3 something\r\n\r\n",
        ];
        for answer in answers {
            let gateway = Gateway::answering(answer);
            let error = connect(&gateway).send().expect_err("not a status line");
            assert!(message(error).contains("not an HTTP status line"));
        }
    }

    #[test]
    fn a_status_that_is_not_three_digits_is_refused() {
        // Three digits and not "one or more" because RFC 9110 says so, and
        // because it is what lets `status()` be a `u16` without diverging from
        // Python's arbitrary-precision `int`. `99999` parses there and cannot be
        // represented here, so without this rule the two SDKs disagree on a real
        // input rather than on a hypothetical one.
        let answers: [&'static [u8]; 3] = [
            b"HTTP/1.1 20 something\r\n\r\n",
            b"HTTP/1.1 2000 something\r\n\r\n",
            b"HTTP/1.1 99999 something\r\n\r\n",
        ];
        for answer in answers {
            let gateway = Gateway::answering(answer);
            let error = connect(&gateway).send().expect_err("not a status line");
            assert!(message(error).contains("not an HTTP status line"));
        }
    }

    #[test]
    fn a_status_line_with_no_http_version_is_refused() {
        // The review's case, 2026-09-08, reproduced in the Python SDK the same
        // day: this answered `status=200 ok=true reason='OK'`. `parse_head`
        // split the line and looked only at the second token, so the first was
        // whatever the peer felt like sending - and here it was
        // `let _version = parts.next();`, discarded on purpose.
        let gateway = Gateway::answering(b"garbage 200 OK\r\n\r\n");
        let error = connect(&gateway).send().expect_err("no version token");
        assert!(message(error).contains("not an HTTP status line"));
    }

    #[test]
    fn a_faked_status_line_cannot_supply_an_exit_address() {
        // Worse than the status alone and not in the review: whatever is
        // listening also gets to name the exit, and the caller reports that
        // address as the one its traffic left from. `ok()` is what a caller
        // branches on and `exit_ip()` is what it then prints.
        let gateway = Gateway::answering(b"garbage 200 OK\r\nX-Proxy-Exit-IP: 1.2.3.4\r\n\r\n");
        let error = connect(&gateway)
            .exit_ip_header("X-Proxy-Exit-IP")
            .send()
            .expect_err("no version token");
        assert!(message(error).contains("not an HTTP status line"));
    }

    #[test]
    fn a_version_token_that_is_not_the_grammar_is_refused() {
        // RFC 9112 section 2.3: `HTTP-name "/" DIGIT "." DIGIT`, with the name
        // case-sensitive. Eight bytes exactly - a CONNECT answered over a socket
        // this crate opened itself is HTTP/1.x by construction, so there is no
        // version negotiation to be liberal about. `http/1.1` is in the list
        // because being liberal about case is the single most likely way one
        // port diverges from another.
        for version in [
            "HTTP/1",
            "HTTP/11",
            "http/1.1",
            "HTTP/1.1x",
            "HTTPS/1.1",
            "",
        ] {
            let gateway = Gateway::answering(format!("{version} 200 OK\r\n\r\n").into_bytes());
            let error = connect(&gateway).send().expect_err("not the grammar");
            assert!(
                message(error).contains("not an HTTP status line"),
                "{version:?} was accepted as a version token"
            );
        }
    }

    #[test]
    fn the_versions_a_proxy_may_answer_with_are_accepted() {
        // The control the rule above needs, and what stops the check from being
        // tightened into something that refuses a real gateway: without it,
        // "refuse anything that is not HTTP/1.1" passes every case above and
        // breaks against a proxy answering 1.0.
        for version in ["HTTP/1.1", "HTTP/1.0", "HTTP/0.9"] {
            let gateway = Gateway::answering(format!("{version} 200 OK\r\n\r\n").into_bytes());
            let result = connect(&gateway).send().expect("a 200 came back");
            assert_eq!(result.status(), 200, "{version} was refused");
        }
    }

    #[test]
    fn a_refused_connection_says_the_gateway_was_never_reached() {
        // Bound and immediately dropped, so the port is free and nothing is
        // listening. Loopback refuses rather than hanging, which is why this is
        // a test and not a fifteen-second wait.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let address = listener.local_addr().expect("local address").to_string();
        drop(listener);
        let error = Connect::new(&address, "acct", "pw")
            .timeout(Duration::from_secs(2))
            .send()
            .expect_err("nothing is listening");
        assert!(message(error).contains("never reached"));
    }

    #[test]
    fn an_address_without_a_port_is_refused_before_anything_is_sent() {
        let error = Connect::new("gate.example.com", "acct", "pw")
            .send()
            .expect_err("not host:port");
        assert!(message(error).contains("Nothing was sent"));
    }

    #[test]
    fn a_port_that_is_not_ascii_digits_is_refused_before_anything_is_sent() {
        // `u16::from_str` would accept a leading `+` and Python's `int` accepts
        // whitespace, an underscore separator and a Unicode minus, so neither
        // language's parser is the rule. The superscripts are the same defect as
        // the status-line case above, in the caller-facing half - and in Python
        // they raised an uncaught `ValueError` past a docstring promising only
        // `CheckError` leaves the module.
        for port in ["eight", "\u{b2}", "+80", " 80", "8_0"] {
            let error = Connect::new(format!("127.0.0.1:{port}"), "acct", "pw")
                .timeout(Duration::from_secs(2))
                .send()
                .expect_err("not a port");
            assert!(
                message(error).contains("host:port"),
                "port {port:?} was not refused"
            );
        }
    }

    #[test]
    fn a_port_outside_1_to_65535_is_refused_before_anything_is_sent() {
        // `65536` and `70000` do not typecheck as a `u16` anywhere else in this
        // crate; here they arrive as text off a caller's configuration file, so
        // the type system is not doing the work and this is. `0` is legal in
        // `bind`, where it means "any free port", and meaningless in `connect`:
        // Windows answers WinError 10049 and Linux ECONNREFUSED, so refusing it
        // in the parse replaces a platform-specific errno with a sentence that
        // names the mistake - and keeps the answer identical in four SDKs.
        //
        // The long one is not the same rule twice. Python's `isdigit()` gate
        // accepted it, `int()` succeeded, and `create_connection` raised
        // `OverflowError` - which derives from `ArithmeticError`, not `OSError`,
        // so it walked straight past the handler that exists to convert
        // everything from the socket layer.
        for port in ["0", "65536", "70000", "99999999999999999999"] {
            let error = Connect::new(format!("127.0.0.1:{port}"), "acct", "pw")
                .timeout(Duration::from_secs(2))
                .send()
                .expect_err("not a usable port");
            assert!(
                message(error).contains("Nothing was sent"),
                "port {port:?} was not refused"
            );
        }
    }

    #[test]
    fn a_zero_timeout_is_refused_before_anything_is_sent() {
        // A Rust-only refusal, and it earns its divergence. `connect_timeout`
        // rejects a zero duration with `InvalidInput`, which surfaces as
        // "no answer from the gateway: invalid input parameter" - a message that
        // reads as a bug in this crate rather than as a caller passing zero.
        // Python hands zero to the socket layer, which switches to non-blocking
        // mode and fails with whatever the platform says.
        let error = Connect::new("127.0.0.1:8080", "acct", "pw")
            .timeout(Duration::ZERO)
            .send()
            .expect_err("zero is not a timeout");
        assert!(message(error).contains("no time in which to answer"));
    }
}

mod the_credential_never_appears {
    use super::*;

    #[test]
    fn no_failure_message_on_any_path_carries_the_password() {
        // `send` holds the request bytes - which contain the base64 credential -
        // in scope for every error it can produce. A format string that reached
        // for them would put a working `Proxy-Authorization` into every
        // backtrace, CI log and pasted bug report. The base64 of
        // `swordfish-login:hunter2-password` is checked as well as the plain
        // text, because base64 is not encryption and a leak in that form is
        // still a leak.
        let secret = "hunter2-password";
        let encoded = "c3dvcmRmaXNoLWxvZ2luOmh1bnRlcjItcGFzc3dvcmQ=";
        let answers: [&'static [u8]; 3] = [
            b"",
            b"<html>You must sign in</html>\r\n\r\n",
            b"HTTP/1.1 OK Fine\r\n\r\n",
        ];
        for answer in answers {
            let gateway = Gateway::answering(answer);
            let error = Connect::new(&gateway.address, "swordfish-login", secret)
                .timeout(Duration::from_secs(5))
                .send()
                .expect_err("nothing usable came back");
            let text = message(error);
            assert!(!text.contains(secret), "the password leaked into {text:?}");
            assert!(
                !text.contains(encoded),
                "the credential leaked into {text:?}"
            );
        }
    }

    #[test]
    fn a_successful_check_carries_no_credential_anywhere() {
        let gateway = Gateway::answering(ESTABLISHED);
        let result = Connect::new(&gateway.address, "swordfish-login", "hunter2-password")
            .timeout(Duration::from_secs(5))
            .exit_ip_header("X-Proxy-Exit-IP")
            .send()
            .expect("a 200 came back");
        for text in [format!("{result}"), format!("{result:?}")] {
            assert!(!text.contains("hunter2-password"), "leaked into {text:?}");
            assert!(!text.contains("c3dvcmRmaXNo"), "leaked into {text:?}");
        }
        assert!(!result
            .headers()
            .iter()
            .any(|(_, value)| value.contains("hunter2-password")));
    }

    #[test]
    fn debug_on_a_connect_redacts_the_password_and_keeps_everything_else() {
        // `Connect` is the one type here that holds the secret between calls, so
        // it is the one a caller is most likely to print while debugging. The
        // derive would have printed it.
        let connect = Connect::new("127.0.0.1:8080", "acct-country-us", "hunter2-password");
        let text = format!("{connect:?}");
        assert!(!text.contains("hunter2-password"), "leaked into {text:?}");
        assert!(text.contains("***"));
        assert!(text.contains("acct-country-us"));
        assert!(text.contains("127.0.0.1:8080"));
    }
}

mod what_is_actually_sent_on_the_wire {
    use super::*;

    #[test]
    fn the_request_is_a_connect_with_the_credential_and_nothing_else() {
        // The base64 is pinned against a string Python's `base64.b64encode`
        // produced for the same input, not against this crate's own output. The
        // encoder here is hand-rolled to keep the manifest at one dependency, so
        // it needs a reference outside itself - and this header is the one thing
        // on the wire that has to be byte-identical in four SDKs.
        let gateway = Gateway::answering(ESTABLISHED);
        Connect::new(&gateway.address, "acct-country-us", "pw")
            .timeout(Duration::from_secs(5))
            .send()
            .expect("a 200 came back");
        assert_eq!(
            gateway.request(),
            "CONNECT api.ipify.org:443 HTTP/1.1\r\n\
             Host: api.ipify.org:443\r\n\
             Proxy-Authorization: Basic YWNjdC1jb3VudHJ5LXVzOnB3\r\n\
             Proxy-Connection: close\r\n\
             \r\n"
        );
    }

    #[test]
    fn the_target_is_a_parameter_and_reaches_both_lines() {
        // There is no null CONNECT, so whatever is named here sees a real TCP
        // connection from the exit address. That makes it a parameter rather
        // than a constant, and it has to appear on the request line *and* in
        // `Host:` - a gateway that routes on one and logs the other would
        // otherwise be told two different things.
        let gateway = Gateway::answering(ESTABLISHED);
        connect(&gateway)
            .target("example.com:443")
            .send()
            .expect("a 200 came back");
        let request = gateway.request();
        assert!(request.starts_with("CONNECT example.com:443 HTTP/1.1\r\n"));
        assert!(request.contains("\r\nHost: example.com:443\r\n"));
    }

    #[test]
    fn a_target_carrying_crlf_is_refused_before_anything_is_sent() {
        // Measured 2026-09-08 in the Python SDK, which builds this request the
        // same way: the target is interpolated into `CONNECT {target} HTTP/1.1`,
        // so the gateway received a request line of `CONNECT example.com:443` -
        // no version token at all - and `X-Injected: yes HTTP/1.1` as a header of
        // our own request. Header injection is the obvious half; the request line
        // losing its version to a caller's string is the half that is easy to
        // miss. The address is a port nothing listens on, so a test that fails
        // does so by not connecting rather than by quietly passing.
        let error = Connect::new("127.0.0.1:1", "acct", "pw")
            .target("example.com:443\r\nX-Injected: yes")
            .timeout(Duration::from_secs(2))
            .send()
            .expect_err("the target is not one token");
        assert!(message(error).contains("Nothing was sent"));
    }

    #[test]
    fn a_target_that_is_not_one_token_of_visible_ascii_is_refused() {
        // Deliberately wider than the characters that hurt. The request line is
        // built by interpolation and the set of bytes that change its shape is
        // not a thing to enumerate from memory - a space alone splits the line
        // into a different request.
        for target in [
            "",
            "a b:443",
            "a\nb:443",
            "a\tb:443",
            "a\0b:443",
            "ex\u{e4}mple:443",
        ] {
            let error = Connect::new("127.0.0.1:1", "acct", "pw")
                .target(target)
                .timeout(Duration::from_secs(2))
                .send()
                .expect_err("the target is not one token");
            assert!(
                message(error).contains("Nothing was sent"),
                "{target:?} was accepted as a target"
            );
        }
    }

    #[test]
    fn the_credential_is_not_the_injection_surface_and_is_left_alone() {
        // The control on the rule above: a login containing CRLF cannot inject
        // anything, because it goes through base64 whose output alphabet is
        // A-Za-z0-9+/= and holds neither CR nor LF. Validating it would be
        // cargo-culting the fix onto the field that was already safe - and the
        // defect was the reverse of that. `Proxy` refuses CRLF in `country`,
        // which is base64-encoded, while the target, which lands in the request
        // line in the clear, was unchecked.
        let gateway = Gateway::answering(ESTABLISHED);
        let result = Connect::new(&gateway.address, "acct\r\nX-Injected: yes", "pw")
            .timeout(Duration::from_secs(5))
            .send()
            .expect("a 200 came back");
        assert!(result.ok());
        let request = gateway.request();
        assert!(!request.contains("X-Injected"));
        assert_eq!(request.matches("\r\n\r\n").count(), 1);
    }

    #[test]
    fn base64_agrees_with_python_on_all_three_padding_cases() {
        // Every remainder mod 3, plus a non-ASCII password, which is the case a
        // hand-rolled encoder gets wrong by encoding characters instead of
        // bytes. Right-hand sides produced by `base64.b64encode` in the Python
        // SDK's interpreter on 2026-09-07.
        for (login, password, expected) in [
            ("a", "b", "YTpi"),
            ("ab", "c", "YWI6Yw=="),
            ("abc", "d", "YWJjOmQ="),
            ("acct", "p\u{e4}ss", "YWNjdDpww6Rzcw=="),
        ] {
            let gateway = Gateway::answering(ESTABLISHED);
            Connect::new(&gateway.address, login, password)
                .timeout(Duration::from_secs(5))
                .send()
                .expect("a 200 came back");
            assert!(
                gateway
                    .request()
                    .contains(&format!("Proxy-Authorization: Basic {expected}\r\n")),
                "{login}:{password} did not encode to {expected}"
            );
        }
    }
}

mod the_head_is_bounded {
    use super::*;

    #[test]
    fn a_gateway_that_answers_with_a_stream_does_not_exhaust_memory() {
        // 16 KiB and stop. A head is a few hundred bytes, and an unbounded read
        // here is a memory exhaustion bug waiting for a gateway that answers
        // with a stream. The answer below never contains a blank line, so the
        // only thing that can end the read is the bound.
        //
        // This asserted "not an HTTP status line" until 2026-09-08, which passed
        // for the wrong reason: the bound was a `break`, so the truncated buffer
        // went on to `parse_head` and was refused there for being 64 KiB of `x`
        // rather than for being unterminated. Substitute a valid status line at
        // the top of the flood - which is what a gateway answering with a stream
        // would actually send - and the old code reported a complete 200.
        static FLOOD: &[u8] = &[b'x'; 64 * 1024];
        let gateway = Gateway::answering(FLOOD);
        let error = connect(&gateway).send().expect_err("no end to the head");
        assert!(message(error).contains("no end to the response head"));
    }

    #[test]
    fn a_valid_status_line_at_the_top_of_a_flood_is_not_reported_as_an_answer() {
        // The case the test above could not see, and the reason it was rewritten
        // rather than deleted. `parse_head` reads the first line and stops, so a
        // 16 KiB truncation that begins with `HTTP/1.1 200 OK` parsed as an
        // opened tunnel with whatever headers happened to fit inside the bound.
        let mut flood = b"HTTP/1.1 200 Connection established\r\n".to_vec();
        flood.extend(std::iter::repeat_n(b'x', 64 * 1024));
        let gateway = Gateway::answering(flood);
        let error = connect(&gateway).send().expect_err("no end to the head");
        assert!(message(error).contains("no end to the response head"));
    }
}

mod an_unfinished_head_is_not_an_answer {
    use super::*;

    #[test]
    fn a_head_cut_off_mid_block_is_refused_rather_than_reported_as_ok() {
        // Found 2026-09-08 in an external review and reproduced the same day in
        // the Python SDK, which had the identical shape: this exact payload gave
        // `status=200 ok=true` with an empty header list. Both parts are wrong
        // and the second is the quieter one - the exit header is *in* the bytes
        // and `parse_head` drops it, because a header only enters the table when
        // the blank line proves it arrived whole. So a caller was told the
        // tunnel opened and the field it came for had gone missing.
        let gateway =
            Gateway::answering(b"HTTP/1.1 200 Connection established\r\nX-Proxy-Exit-IP: 1.2.3.4");
        let error = connect(&gateway)
            .exit_ip_header("X-Proxy-Exit-IP")
            .send()
            .expect_err("the head never ended");
        assert!(message(error).contains("part-way through"));
    }

    #[test]
    fn a_status_line_with_no_blank_line_after_it_is_refused() {
        // The minimal case: everything a caller needs is present and the head
        // still never ended, so there is no way to know whether a header was on
        // its way. A gateway that means to answer 200 sends the blank line.
        let gateway = Gateway::answering(b"HTTP/1.1 200 Connection established\r\n");
        let error = connect(&gateway).send().expect_err("the head never ended");
        assert!(message(error).contains("part-way through"));
    }

    #[test]
    fn truncation_is_told_apart_from_an_immediate_close() {
        // Two failures with two different causes: nothing at all is a documented
        // reaction of the shipped gateway - an empty parameter value hangs and
        // then closes - while a head that starts and stops is not, and is worth
        // reporting. One message for both would lose that.
        let cut = Gateway::answering(b"HTTP/1.1 200 OK\r\nX-Pad: a");
        let truncated = connect(&cut).send().expect_err("the head never ended");
        let silent = Gateway::answering(b"");
        let nothing = connect(&silent).send().expect_err("nothing came back");
        assert!(message(truncated).contains("part-way through"));
        assert!(message(nothing).contains("without answering"));
    }
}

/// Exactly what the shipped gateway answered on 2026-09-08, byte for byte, read
/// off `gate.nodemaven.com:8080` with a raw dump rather than off formatted
/// output. The success path frames its head with CRLF and every refusal frames
/// it with bare LF, so these are three payloads and one framing question.
///
/// The exit address is the only edit: TEST-NET-3 per RFC 5737 in place of the
/// address the gateway returned. Everything else including the header *names* is
/// reproduced, because this back end sends `X-Exit-IP` and not the
/// `X-Proxy-Exit-IP` a caller may be looking for.
mod the_real_gateways_bytes {
    use super::*;

    const LIVE_407: &[u8] = b"HTTP/1.1 407 Proxy Authentication Required\n\
                              Proxy-Authenticate: Basic realm=\"Invalid credentials\"\n\
                              Connection: close\n\
                              \n";

    const LIVE_406: &[u8] = b"HTTP/1.1 406 Not Acceptable\nConnection: close\n\n";

    const LIVE_200: &[u8] = b"HTTP/1.1 200 OK\r\n\
                              X-Exit-IP: 203.0.113.104\r\n\
                              X-Exit-Country: US\r\n\
                              X-Exit-Timezone: America/Los_Angeles\r\n\
                              X-Exit-ASN: 6167\r\n\
                              \r\n";

    /// RFC 9112 section 2.2 lets a recipient treat a bare LF as a line
    /// terminator and ignore any preceding CR. That is permission rather than
    /// obligation everywhere except here: this gateway frames every refusal with
    /// LF and carries `Connection: close`, so a CRLF-only reader sees the peer
    /// hang up with no blank line, calls it a truncated head, and can report no
    /// refusal code at all - which is the one thing this module exists to do.
    #[test]
    fn the_407_that_a_bad_filter_value_produces() {
        let gateway = Gateway::answering(LIVE_407);
        let result = connect(&gateway)
            .reactions(reactions())
            .send()
            .expect("a 407 is an answer");
        assert_eq!(result.status(), 407);
        assert_eq!(result.reason(), "Proxy Authentication Required");
        assert!(!result.ok());
        assert_eq!(result.header("connection"), Some("close"));
        assert_eq!(
            result.header("proxy-authenticate"),
            Some("Basic realm=\"Invalid credentials\"")
        );
        // The whole point of carrying the table: the status says credentials and
        // the cause is a value the gateway would not take.
        assert!(result.meaning().is_some());
    }

    #[test]
    fn the_406_that_a_bad_region_produces() {
        let gateway = Gateway::answering(LIVE_406);
        let result = connect(&gateway).send().expect("a 406 is an answer");
        assert_eq!(result.status(), 406);
        assert_eq!(result.reason(), "Not Acceptable");
        assert_eq!(result.headers().len(), 1);
        assert_eq!(result.header("connection"), Some("close"));
    }

    #[test]
    fn the_200_is_framed_the_other_way_and_still_parses() {
        let gateway = Gateway::answering(LIVE_200);
        let result = connect(&gateway).send().expect("a 200 came back");
        assert!(result.ok());
        assert_eq!(result.reason(), "OK");
        assert_eq!(result.header("x-exit-ip"), Some("203.0.113.104"));
        assert_eq!(result.header("x-exit-asn"), Some("6167"));
    }

    #[test]
    fn the_header_this_back_end_omits_reads_as_absent() {
        // A caller asking for a name this reply does not carry gets None, not a
        // wrong address and not an error. The case above proves the address was
        // on the wire under another name; resolving that is a provider
        // definition question and not this module's.
        let gateway = Gateway::answering(LIVE_200);
        let result = connect(&gateway)
            .exit_ip_header("X-Proxy-Exit-IP")
            .send()
            .expect("a 200 came back");
        assert_eq!(result.exit_ip(), None);
    }
}

mod a_blank_line_has_four_spellings {
    use super::*;

    /// Once a bare LF is a terminator, the blank line is any of these four,
    /// including the two mixed ones - which are what a gateway assembling a
    /// reply from a template and a variable body produces.
    #[test]
    fn every_spelling_ends_the_head() {
        for terminator in [
            b"\r\n\r\n".as_slice(),
            b"\n\n".as_slice(),
            b"\r\n\n".as_slice(),
            b"\n\r\n".as_slice(),
        ] {
            let mut answer = b"HTTP/1.1 200 OK\r\nX-Exit-ASN: 6167".to_vec();
            answer.extend_from_slice(terminator);
            let gateway = Gateway::answering(answer);
            let result = connect(&gateway)
                .send()
                .unwrap_or_else(|error| panic!("{terminator:?} ended the head: {error}"));
            assert_eq!(result.status(), 200);
            assert_eq!(result.header("x-exit-asn"), Some("6167"));
            assert_eq!(result.headers().len(), 1);
        }
    }

    #[test]
    fn body_bytes_in_the_same_segment_are_not_parsed_as_headers() {
        // The head ends at the blank line and the read stops there. Anything
        // after it belongs to the tunnel, and a header table assembled from
        // tunnel bytes would be a security bug rather than a parsing one.
        let mut answer = b"HTTP/1.1 200 OK\r\nX-Exit-IP: 203.0.113.104\r\n\r\n".to_vec();
        answer.extend_from_slice(b"\x16\x03\x01\x00\x01X-Exit-IP: 10.0.0.1\r\n");
        let gateway = Gateway::answering(answer);
        let result = connect(&gateway).send().expect("a 200 came back");
        assert_eq!(result.headers().len(), 1);
        assert_eq!(result.header("x-exit-ip"), Some("203.0.113.104"));
    }

    #[test]
    fn a_cr_that_is_not_before_the_lf_stays_in_the_value() {
        // Only a CR immediately before the LF is a line terminator. One in the
        // middle of a value is part of the value, and repairing it would be this
        // crate inventing a reply the gateway did not send.
        let gateway = Gateway::answering(b"HTTP/1.1 200 OK\nX-Pad: a\rb\n\n");
        let result = connect(&gateway).send().expect("a 200 came back");
        assert_eq!(result.header("x-pad"), Some("a\rb"));
    }
}

/// The fix widens what counts as a complete head and nothing else.
///
/// Both halves have to hold at once: an LF-framed refusal is an answer, and an
/// LF-framed head that stops without its blank line is still not one. A fix that
/// got only the first half would report every truncation as a 200 again.
mod lf_framing_does_not_undo_the_truncation_guard {
    use super::*;

    #[test]
    fn an_lf_framed_head_with_no_blank_line_is_still_refused() {
        let gateway = Gateway::answering(
            b"HTTP/1.1 407 Proxy Authentication Required\nConnection: close\n".as_slice(),
        );
        let error = connect(&gateway).send().expect_err("the head never ended");
        assert!(message(error).contains("part-way through"));
    }

    #[test]
    fn a_bare_lf_status_line_alone_is_still_refused() {
        let gateway = Gateway::answering(b"HTTP/1.1 200 OK\n");
        let error = connect(&gateway).send().expect_err("the head never ended");
        assert!(message(error).contains("part-way through"));
    }

    #[test]
    fn a_single_lf_is_not_a_blank_line() {
        // The one case that separates "ends a line" from "ends the head": a head
        // whose last line ended and whose blank line never came.
        let gateway = Gateway::answering(b"HTTP/1.1 406 Not Acceptable\n");
        let error = connect(&gateway).send().expect_err("the head never ended");
        assert!(message(error).contains("part-way through"));
    }
}

mod proxy_wires_the_provider_in {
    use super::*;

    fn proxy_at(gateway: &Gateway) -> Proxy {
        let (host, port) = gateway.address.rsplit_once(':').expect("host:port");
        Proxy::builder()
            .login("acct")
            .password("pw")
            .host(host)
            .port(port.parse::<u16>().expect("a port"))
            .param("country", "us")
            .build()
            .expect("a valid proxy")
    }

    #[test]
    fn check_sends_the_built_username_and_not_the_bare_login() {
        // The whole point of checking through a `Proxy`: the parameters are
        // under test too, not just the credentials. Pinned by decoding the
        // header back, which is also a second check that the username the
        // builder produces is the one that reaches the wire.
        let gateway = Gateway::answering(ESTABLISHED);
        let proxy = proxy_at(&gateway);
        assert_eq!(proxy.username(), "acct-country-us");
        proxy.check().expect("a 200 came back");
        assert!(gateway
            .request()
            .contains("Proxy-Authorization: Basic YWNjdC1jb3VudHJ5LXVzOnB3\r\n"));
    }

    #[test]
    fn check_reads_the_exit_header_the_provider_declares() {
        let gateway = Gateway::answering(ESTABLISHED);
        let result = proxy_at(&gateway).check().expect("a 200 came back");
        assert_eq!(result.exit_ip(), Some("203.0.113.7"));
    }

    #[test]
    fn check_attaches_the_providers_own_reading_of_a_407() {
        // The shipped definition's table, not a table this test wrote. It is the
        // answer to "I got a 407, now what" for the one status where the gateway
        // is measurably misleading: a bad `filter` value is answered 407, which
        // sends the caller to check credentials that are correct.
        let gateway = Gateway::answering(REFUSED);
        let proxy = proxy_at(&gateway);
        let expected = proxy.provider().reaction(407).expect("407 is described");
        let result = proxy.check().expect("a 407 is not an error");
        assert_eq!(result.meaning(), Some(expected));
    }

    #[test]
    fn connect_lets_the_target_and_the_timeout_be_changed() {
        // Python spells this `proxy.check(target=..., timeout=...)`. Rust has no
        // keyword arguments, so the builder is the faithful port rather than a
        // second API - and it must still carry everything `check()` carries.
        let gateway = Gateway::answering(ESTABLISHED);
        let result = proxy_at(&gateway)
            .connect()
            .target("example.com:443")
            .timeout(Duration::from_secs(3))
            .send()
            .expect("a 200 came back");
        assert_eq!(result.exit_ip(), Some("203.0.113.7"));
        assert!(gateway
            .request()
            .starts_with("CONNECT example.com:443 HTTP/1.1\r\n"));
    }

    #[test]
    fn check_reports_the_server_without_credentials_in_it() {
        let gateway = Gateway::answering(ESTABLISHED);
        let proxy = proxy_at(&gateway);
        let result = proxy.check().expect("a 200 came back");
        assert_eq!(result.server(), proxy.server());
        assert!(!result.server().contains("pw"));
        assert!(!result.server().contains('@'));
    }
}

mod how_a_check_prints {
    use super::*;

    fn check(answer: &'static [u8], with_reactions: bool) -> Check {
        let gateway = Gateway::answering(answer);
        let mut connect = Connect::new(&gateway.address, "acct", "pw")
            .timeout(Duration::from_secs(5))
            .exit_ip_header("X-Proxy-Exit-IP");
        if with_reactions {
            connect = connect.reactions(reactions());
        }
        connect.send().expect("an answer came back")
    }

    #[test]
    fn a_200_prints_the_status_the_phrase_the_server_and_the_exit() {
        let result = check(ESTABLISHED, true);
        let text = result.to_string();
        assert!(text.starts_with("200 Connection established via 127.0.0.1:"));
        assert!(text.ends_with(", exit 203.0.113.7"));
        // No meaning line, and there are two reasons rather than one: this
        // table has no entry for 200, and `Display` suppresses the line when
        // `ok()` anyway. The second is what stops the shipped definition - whose
        // 200 entry warns that an unrecognised parameter is answered 200 and
        // dropped - from printing a warning on every successful check.
        assert!(!text.contains('\n'));
    }

    #[test]
    fn a_407_prints_the_meaning_on_its_own_line() {
        let text = check(REFUSED, true).to_string();
        let (head, meaning) = text.split_once('\n').expect("two lines");
        assert!(head.starts_with("407 Proxy Authentication Required via 127.0.0.1:"));
        assert!(meaning.contains("not necessarily about your credentials"));
    }

    #[test]
    fn a_407_with_no_table_prints_one_line() {
        let text = check(REFUSED, false).to_string();
        assert!(!text.contains('\n'));
        assert!(text.starts_with("407 Proxy Authentication Required via "));
    }
}

/// The README quotes what a check prints. These read that file rather than
/// carrying a copy of the strings.
///
/// That is a deliberate difference from the sibling module in tests/proxy.rs,
/// which spells its expected message out. It matters most for the 407
/// paragraph: that text is **data in the provider definition**, so a test
/// holding its own copy would keep passing after the definition was edited and
/// the README had gone stale - which is the same failure that module was
/// written for, where a quoted list of nine parameter names had become ten.
mod the_readme_shows_real_output {
    use super::*;

    const README: &str = include_str!("../README.md");

    /// Replace the server address and the elapsed time, keep everything else.
    ///
    /// They are the two things a README example cannot share with a loopback
    /// run - the address is a random high port, the time is whatever the
    /// machine did - and every other character of the line is format.
    fn shape(line: &str) -> String {
        let (status, rest) = line.split_once(" via ").expect("a via clause");
        let (_, elapsed) = rest.split_once(" in ").expect("an in clause");
        let (_, tail) = elapsed.split_once('s').expect("seconds");
        format!("{status} via SERVER in TIMEs{tail}")
    }

    fn readme_line(starting: &str) -> &'static str {
        README
            .lines()
            .find(|line| line.starts_with(starting))
            .unwrap_or_else(|| panic!("the README no longer shows a {starting:?} line"))
    }

    #[test]
    fn the_200_line_is_what_the_code_prints() {
        let gateway = Gateway::answering(ESTABLISHED);
        let live = connect(&gateway)
            .exit_ip_header("X-Proxy-Exit-IP")
            .send()
            .expect("an answer came back")
            .to_string();
        assert_eq!(
            shape(readme_line("200 Connection established via ")),
            shape(&live)
        );
    }

    #[test]
    fn the_407_line_is_what_the_code_prints() {
        // No reaction table, so this is the one-line form the README quotes;
        // the explanation below it is pinned separately.
        let gateway = Gateway::answering(REFUSED);
        let live = connect(&gateway)
            .send()
            .expect("an answer came back")
            .to_string();
        assert_eq!(
            shape(readme_line("407 Proxy Authentication Required via ")),
            shape(&live)
        );
    }

    #[test]
    fn the_407_explanation_is_the_shipped_definitions_own_text() {
        let provider = nodemaven::load("nodemaven").expect("the shipped definition");
        let meaning = provider.reaction(407).expect("a 407 entry");
        assert!(
            README.contains(meaning),
            "the README's 407 paragraph is no longer the definition's own text"
        );
    }
}
