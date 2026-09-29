//! The account API client. No key, no live host.
//!
//! The behaviour shared with the Python SDK is pinned by the shared
//! specification's `cases/api.json`, run from `sdk-spec/runners/rust`. What is
//! here is the rest: the Rust surface, and the built-in transport driven over
//! a real socket on `127.0.0.1`, which a canned transport cannot exercise.

use std::io;
use std::sync::{Arc, Mutex};

use nodemaven::{Client, Error, Proxy, API_ROOT};
use serde_json::{json, Value};

type Calls = Arc<Mutex<Vec<(String, String, Vec<(String, String)>, Option<Value>)>>>;

/// A client whose transport serves `responses` in order (the last repeats) and
/// records every request.
fn client(responses: Vec<(u16, &str)>) -> (Client, Calls) {
    let queue: Vec<(u16, Vec<u8>)> = responses
        .into_iter()
        .map(|(status, body)| (status, body.as_bytes().to_vec()))
        .collect();
    let queue = Arc::new(Mutex::new(queue));
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let log = calls.clone();
    let transport =
        move |method: &str, url: &str, headers: &[(String, String)], body: Option<&[u8]>| {
            log.lock().unwrap().push((
                method.to_string(),
                url.to_string(),
                headers.to_vec(),
                body.map(|b| serde_json::from_slice(b).unwrap()),
            ));
            let mut queue = queue.lock().unwrap();
            let next = if queue.len() > 1 {
                queue.remove(0)
            } else {
                queue[0].clone()
            };
            Ok::<_, io::Error>(next)
        };
    let client = Client::builder()
        .api_key("secret-key")
        .base_url("https://dashboard.example")
        .transport(transport)
        .build()
        .unwrap();
    (client, calls)
}

fn api_error(result: Result<impl std::fmt::Debug, Error>) -> (Option<u16>, String) {
    match result {
        Err(Error::Api { status, message }) => (status, message),
        other => panic!("expected an Api error, got {other:?}"),
    }
}

mod where_things_go {
    use super::*;

    #[test]
    fn each_endpoint_is_where_the_spec_says_it_is() {
        let (api, calls) = client(vec![(200, r#"{"results": []}"#)]);
        api.countries(&[]).unwrap();
        api.regions(&[]).unwrap();
        api.cities(&[]).unwrap();
        api.isps(&[]).unwrap();
        api.isp_regions(&[]).unwrap();
        api.isp_cities(&[]).unwrap();
        api.zip_codes(&[]).unwrap();
        api.zip_code_regions(&[]).unwrap();
        api.zip_code_cities(&[]).unwrap();
        api.sub_users(&[]).unwrap();
        api.whitelist_ips(&[]).unwrap();
        let paths: Vec<String> = calls
            .lock()
            .unwrap()
            .iter()
            .map(|(_, url, _, _)| url.split('?').next().unwrap().to_string())
            .collect();
        let expected: Vec<String> = [
            "locations/countries/",
            "locations/regions/",
            "locations/cities/",
            "locations/isps/",
            "locations/isps/regions/",
            "locations/isps/cities/",
            "locations/zipcodes/",
            "locations/zipcodes/regions/",
            "locations/zipcodes/cities/",
            "sub-users/",
            "whitelist/ips",
        ]
        .iter()
        .map(|path| format!("https://dashboard.example{API_ROOT}/{path}"))
        .collect();
        assert_eq!(paths, expected);
    }

    #[test]
    fn the_key_travels_as_x_api_key_in_authorization() {
        let (api, calls) = client(vec![(200, "{}")]);
        api.me().unwrap();
        let headers = &calls.lock().unwrap()[0].2;
        assert!(headers.contains(&("Authorization".into(), "x-api-key secret-key".into())));
        assert!(headers.contains(&("User-Agent".into(), "nodemaven-rust".into())));
        assert!(!headers.iter().any(|(name, _)| name == "Content-Type"));
    }

    #[test]
    fn statistics_send_the_proxy_username_and_the_filters() {
        let (api, calls) = client(vec![(200, r#"{"labels": [], "data": []}"#)]);
        api.statistics_data("acct", &[("start", "20-08-2026"), ("end", "21-08-2026")])
            .unwrap();
        let url = calls.lock().unwrap()[0].1.clone();
        assert!(
            url.ends_with("statistics/data/?start=20-08-2026&end=21-08-2026&proxy_username=acct"),
            "{url}"
        );
    }

    #[test]
    fn an_id_in_a_path_is_escaped_whole() {
        let (api, calls) = client(vec![(200, r#"{"message": "ok"}"#)]);
        api.delete_whitelist_ip("1/../../users/me").unwrap();
        let url = calls.lock().unwrap()[0].1.clone();
        assert!(
            url.ends_with("/whitelist/ip/1%2F..%2F..%2Fusers%2Fme"),
            "{url}"
        );
    }

    #[test]
    fn a_callers_limit_wins_over_the_default() {
        let (api, calls) = client(vec![(200, r#"{"results": []}"#)]);
        api.countries(&[("limit", "500")]).unwrap();
        let url = calls.lock().unwrap()[0].1.clone();
        assert!(url.contains("limit=500&offset=0"), "{url}");
        assert!(!url.contains("limit=1000"), "{url}");
    }
}

mod the_key_never_appears {
    use super::*;

    #[test]
    fn in_debug_output() {
        let (api, _) = client(vec![(200, "{}")]);
        let shown = format!("{api:?} {:?}", Client::builder().api_key("secret-key"));
        assert!(!shown.contains("secret-key"), "{shown}");
        assert!(shown.contains("***"));
    }

    #[test]
    fn in_an_error_message() {
        let (api, _) = client(vec![(403, r#"{"detail": "no"}"#)]);
        let error = api.me().unwrap_err();
        assert!(!error.to_string().contains("secret-key"));
        assert!(!format!("{error:?}").contains("secret-key"));
    }
}

mod errors {
    use super::*;

    #[test]
    fn the_four_api_variants_are_api_and_carry_their_status() {
        for (status, name) in [
            (403, "auth"),
            (404, "not_found"),
            (429, "rate_limit"),
            (502, "api"),
        ] {
            let (api, _) = client(vec![(status, r#"{"detail": "x"}"#)]);
            let error = api.me().unwrap_err();
            assert!(error.is_api(), "{name}: {error:?}");
            assert_eq!(error.status(), Some(status), "{name}");
        }
        assert!(!Error::Param("x".into()).is_api());
    }

    #[test]
    fn a_field_error_is_flattened_into_one_line() {
        let (api, _) = client(vec![(
            400,
            r#"{"errors": {"proxy_username": ["This field is required."]}}"#,
        )]);
        let (status, message) = api_error(api.create_sub_user("u", "p", &[]));
        assert_eq!(status, Some(400));
        assert!(
            message.contains("proxy_username: This field is required."),
            "{message}"
        );
    }

    #[test]
    fn a_redirect_is_an_error_and_not_a_success() {
        let (api, _) = client(vec![(302, "")]);
        assert_eq!(api_error(api.me()).0, Some(302));
    }

    #[test]
    fn an_empty_2xx_is_an_answer_to_a_delete_and_to_nothing_else() {
        let (api, _) = client(vec![(204, "")]);
        assert_eq!(api.delete_sub_user(9).unwrap(), json!({}));
        let (api, _) = client(vec![(200, "")]);
        assert!(api_error(api.me()).1.contains("empty body"));
        let (api, _) = client(vec![(200, "")]);
        assert!(api_error(api.countries(&[])).1.contains("empty body"));
    }

    #[test]
    fn success_false_is_an_error_with_or_without_a_payload() {
        let (api, _) = client(vec![(200, r#"{"success": false, "description": "no"}"#)]);
        assert!(api_error(api.delete_sub_user(9))
            .1
            .contains("success=false"));
    }

    #[test]
    fn a_list_endpoint_answering_a_string_is_refused() {
        let (api, _) = client(vec![(200, r#""nothing""#)]);
        assert!(api_error(api.countries(&[]))
            .1
            .contains("neither a page nor a list"));
    }
}

mod sub_users {
    use super::*;

    const ENVELOPE: &str = r#"{"success": true, "description": "", "errors": [], "payload": {"id": 4, "proxy_username": "kid"}}"#;

    #[test]
    fn create_sends_the_two_credentials_and_unwraps_the_payload() {
        let (api, calls) = client(vec![(201, ENVELOPE)]);
        let created = api
            .create_sub_user("kid", "pw", &[("traffic_limit", json!(1024))])
            .unwrap();
        assert_eq!(created, json!({"id": 4, "proxy_username": "kid"}));
        let (method, _, headers, body) = calls.lock().unwrap()[0].clone();
        assert_eq!(method, "POST");
        assert_eq!(
            body.unwrap(),
            json!({"proxy_username": "kid", "proxy_password": "pw", "traffic_limit": 1024})
        );
        assert!(headers.contains(&("Content-Type".into(), "application/json".into())));
    }

    #[test]
    fn update_is_a_put_to_the_collection_with_the_id_in_the_body() {
        let (api, calls) = client(vec![(200, ENVELOPE)]);
        api.update_sub_user("9", &[("traffic_limit", json!(2048))])
            .unwrap();
        let (method, url, _, body) = calls.lock().unwrap()[0].clone();
        assert_eq!(method, "PUT");
        assert!(url.ends_with(&format!("{API_ROOT}/sub-users/")));
        assert_eq!(body.unwrap(), json!({"traffic_limit": 2048, "id": "9"}));
    }

    #[test]
    fn an_empty_change_set_is_refused_and_nothing_is_sent() {
        let (api, calls) = client(vec![(200, ENVELOPE)]);
        assert!(api_error(api.update_sub_user(9, &[]))
            .1
            .contains("nothing to change"));
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn reset_returns_the_array_it_was_given() {
        let (api, _) = client(vec![(200, r#"{"success": true, "payload": [{"id": 4}]}"#)]);
        assert_eq!(
            api.reset_sub_user_usage(&[json!(4)]).unwrap(),
            json!([{"id": 4}])
        );
    }
}

mod iterate {
    use super::*;

    #[test]
    fn a_next_link_on_the_same_origin_is_followed() {
        let (api, calls) = client(vec![
            (
                200,
                r#"{"results": [1], "next": "https://dashboard.example:443/api/v2/base/locations/countries/?offset=1"}"#,
            ),
            (200, r#"{"results": [2]}"#),
        ]);
        let first = api.countries(&[]).unwrap();
        let rows: Vec<Value> = api.iterate(first).collect::<Result<_, _>>().unwrap();
        assert_eq!(rows, vec![json!(1), json!(2)]);
        assert_eq!(calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_next_link_to_another_host_is_refused_and_nothing_is_sent_there() {
        // The guard that keeps the key from a host the server names. The
        // transport records every request, so a second one would show up.
        let (api, calls) = client(vec![(
            200,
            r#"{"results": [1], "next": "https://evil.example/api/v2/base/locations/countries/?offset=1"}"#,
        )]);
        let first = api.countries(&[]).unwrap();
        let error = api.iterate(first).find_map(Result::err).unwrap();
        assert!(error.to_string().contains("evil.example"), "{error}");
        assert!(
            error.to_string().contains("refusing to send the API key"),
            "{error}"
        );
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(!calls[0].1.contains("evil.example"));
    }

    #[test]
    fn a_next_link_that_repeats_is_a_loop() {
        let looping = r#"{"results": [1], "next": "https://dashboard.example/api/v2/base/x?p=2"}"#;
        let (api, _) = client(vec![(200, looping)]);
        let first = api.countries(&[]).unwrap();
        let error = api.iterate(first).find_map(Result::err).unwrap();
        assert!(error.to_string().contains("already returned"), "{error}");
    }

    #[test]
    fn a_server_that_ignores_the_cursor_is_caught() {
        let (api, _) = client(vec![(200, r#"{"results": [1, 2]}"#)]);
        let first = api.countries(&[("limit", "2")]).unwrap();
        let error = api.iterate(first).find_map(Result::err).unwrap();
        assert!(error.to_string().contains("ignoring it"), "{error}");
    }

    #[test]
    fn the_page_bound_is_an_error_and_not_a_truncation() {
        let (api, calls) = client(vec![
            (200, r#"{"results": [1]}"#),
            (200, r#"{"results": [2]}"#),
            (200, r#"{"results": [3]}"#),
        ]);
        let first = api.countries(&[("limit", "1")]).unwrap();
        let results: Vec<_> = api.iterate(first).max_pages(2).collect();
        assert!(results
            .last()
            .unwrap()
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("stopped after 2 pages"));
        assert_eq!(calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_page_number_walk_starts_at_one_and_counts_pages() {
        let envelope = |rows: &str| format!(r#"{{"success": true, "payload": {rows}}}"#);
        let (one, two, three) = (envelope("[1]"), envelope("[2]"), envelope("[]"));
        let (api, calls) = client(vec![(200, &one), (200, &two), (200, &three)]);
        let first = api.sub_users(&[]).unwrap();
        let rows: Vec<Value> = api.iterate(first).collect::<Result<_, _>>().unwrap();
        assert_eq!(rows.len(), 2);
        let urls: Vec<String> = calls.lock().unwrap().iter().map(|c| c.1.clone()).collect();
        assert!(
            urls[0].ends_with("?page=1")
                && urls[1].ends_with("?page=2")
                && urls[2].ends_with("?page=3"),
            "{urls:?}"
        );
    }
}

mod validate {
    use super::*;

    fn proxy(params: &[(&str, &str)]) -> Proxy {
        let mut builder = Proxy::builder()
            .login("acct")
            .password("pw")
            .host("gate.example")
            .port(8080);
        for (name, value) in params {
            builder = builder.param(*name, *value);
        }
        builder.build().unwrap()
    }

    #[test]
    fn a_city_without_a_region_is_reported_without_a_request() {
        let (api, calls) = client(vec![(200, r#"{"results": []}"#)]);
        let problems = api.validate(&proxy(&[("city", "abbeville")])).unwrap();
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("without a region"));
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_country_the_catalogue_lacks_is_reported_and_one_it_has_is_not() {
        let catalogue = r#"{"results": [{"code": "US"}, {"code": "de"}]}"#;
        let (api, calls) = client(vec![(200, catalogue), (200, r#"{"results": []}"#)]);
        assert!(api
            .validate(&proxy(&[("country", "us")]))
            .unwrap()
            .is_empty());
        let url = calls.lock().unwrap()[0].1.clone();
        assert!(url.contains("connection_type=residential"), "{url}");

        let (api, _) = client(vec![(200, catalogue), (200, r#"{"results": []}"#)]);
        let problems = api.validate(&proxy(&[("country", "zz")])).unwrap();
        assert!(problems[0].contains("407"), "{problems:?}");
    }
}

#[cfg(feature = "http")]
mod the_builtin_transport {
    //! Over a real socket on 127.0.0.1: what `ureq` sends and how its failures
    //! come back. No live host, no traffic.
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    /// Accept one connection, record the request head, answer `reply`
    /// (nothing at all when `None`), then hold the connection for `hold`.
    fn serve(reply: Option<&'static [u8]>, hold: Duration) -> (String, Arc<Mutex<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(String::new()));
        let log = seen.clone();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0u8; 8192];
                let mut head = Vec::new();
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => head.extend_from_slice(&buffer[..n]),
                    }
                }
                *log.lock().unwrap() = String::from_utf8_lossy(&head).into_owned();
                if let Some(reply) = reply {
                    let _ = stream.write_all(reply);
                }
                thread::sleep(hold);
            }
        });
        (base, seen)
    }

    fn real(base: &str, timeout: Duration) -> Client {
        Client::builder()
            .api_key("secret-key")
            .base_url(base)
            .timeout(timeout)
            .build()
            .unwrap()
    }

    #[test]
    fn a_json_answer_round_trips_with_the_key_in_the_header() {
        let (base, seen) = serve(
            Some(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 13\r\n\r\n{\"data\": 123}"),
            Duration::ZERO,
        );
        assert_eq!(
            real(&base, Duration::from_secs(5)).me().unwrap(),
            json!({"data": 123})
        );
        let head = seen.lock().unwrap().to_ascii_lowercase();
        assert!(
            head.starts_with("get /api/v2/base/users/me http/1.1"),
            "{head}"
        );
        assert!(
            head.contains("authorization: x-api-key secret-key"),
            "{head}"
        );
    }

    #[test]
    fn a_read_timeout_is_an_api_error_naming_the_timeout() {
        let (base, _) = serve(None, Duration::from_secs(3));
        let (status, message) = api_error(real(&base, Duration::from_millis(500)).me());
        assert_eq!(status, None);
        assert!(message.contains("before the timeout of 0.5 s"), "{message}");
    }

    #[test]
    fn a_truncated_body_is_an_api_error() {
        let (base, _) = serve(
            Some(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 100\r\n\r\nshort"),
            Duration::ZERO,
        );
        let (status, _) = api_error(real(&base, Duration::from_secs(5)).me());
        assert_eq!(status, None);
    }

    #[test]
    fn a_redirect_is_not_followed_so_the_key_stays_home() {
        // The request carries the key in a header. Following a redirect would
        // hand it to whatever host the Location names.
        let (elsewhere, reached) = serve(
            Some(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}"),
            Duration::ZERO,
        );
        let location: &'static str = Box::leak(
            format!(
                "HTTP/1.1 302 Found\r\nLocation: {elsewhere}/steal\r\nContent-Length: 0\r\n\r\n"
            )
            .into_boxed_str(),
        );
        let (base, _) = serve(Some(location.as_bytes()), Duration::ZERO);
        let (status, _) = api_error(real(&base, Duration::from_secs(5)).me());
        assert_eq!(status, Some(302));
        thread::sleep(Duration::from_millis(200));
        assert!(
            reached.lock().unwrap().is_empty(),
            "the redirect target was contacted"
        );
    }
}
