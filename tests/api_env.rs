//! The account API client and the environment, alone in its own process
//! because `std::env` is process-global and the harness runs tests on threads.
//! Every case is in one `#[test]`, sequentially, for the same reason.

use std::env;
#[cfg(feature = "http")]
use std::io::{Read, Write};
#[cfg(feature = "http")]
use std::net::TcpListener;

use nodemaven::{Client, Error};

#[test]
fn the_environment() {
    for name in [
        "NODEMAVEN_APIKEY",
        "NODEMAVEN_BASE_URL",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "http_proxy",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
    ] {
        env::remove_var(name);
    }

    // No key anywhere is a credentials error, before anything is sent.
    match Client::builder().build() {
        Err(Error::Credentials(message)) => assert!(message.contains("NODEMAVEN_APIKEY")),
        other => panic!("expected a Credentials error, got {other:?}"),
    }

    // The vendor's variable names are read.
    env::set_var("NODEMAVEN_APIKEY", "from-env");
    env::set_var("NODEMAVEN_BASE_URL", "https://example.test/");
    let client = Client::builder()
        .transport(
            |_: &str, _: &str, _: &[(String, String)], _: Option<&[u8]>| Ok((200, b"{}".to_vec())),
        )
        .build()
        .unwrap();
    assert!(format!("{client:?}").contains("https://example.test"));
    env::remove_var("NODEMAVEN_BASE_URL");

    // A proxy in the environment is not used by the built-in transport: the
    // variables are set on exactly the machines that use proxies, and an API
    // call routed through one nobody asked for is the trap the Python SDK
    // closes with `ProxyHandler({})`. Port 9 on loopback refuses, so a client
    // that obeyed these would fail instead of reaching the listener.
    //
    // `NO_PROXY` is cleared above for a reason found the hard way on
    // 2026-09-29: the first version of this case passed with `.proxy(None)`
    // removed. The workstation it ran on sets `HTTP_PROXY` and `HTTPS_PROXY` to
    // a local VPN client and `NO_PROXY=localhost,127.0.0.1,::1,.local`, so a
    // loopback target bypassed every proxy whatever the transport did, and the
    // case measured the machine's configuration instead of the code.
    #[cfg(feature = "http")]
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\n\r\n{\"data\": 123}")
                .unwrap();
        });
        for name in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "http_proxy",
            "https_proxy",
            "ALL_PROXY",
        ] {
            env::set_var(name, "http://127.0.0.1:9");
        }
        let me = Client::builder().base_url(base).build().unwrap().me();
        for name in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "http_proxy",
            "https_proxy",
            "ALL_PROXY",
        ] {
            env::remove_var(name);
        }
        assert_eq!(me.unwrap()["data"], 123);
        server.join().unwrap();
    }
    env::remove_var("NODEMAVEN_APIKEY");
}
