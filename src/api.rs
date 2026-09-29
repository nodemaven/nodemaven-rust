//! The account API: traffic, sub-users, whitelisted addresses, locations.
//!
//! A port of the Python SDK's `nodemaven.api`, made on 2026-09-29 to close the
//! gap the shared specification's coverage table recorded. Every path, paging
//! rule and deviation from the vendor's OpenAPI document here was measured
//! against the live API from the Python side, and the measurements live in
//! `api/overlay.json` of that specification and in the Python module's
//! docstrings; this module carries the rules, not the history.
//!
//! Separate from [`crate::Proxy`] and separately credentialled: the proxy login
//! and the dashboard API key are different secrets from different places.
//!
//! ```no_run
//! # fn main() -> Result<(), nodemaven::Error> {
//! use nodemaven::Client;
//!
//! let client = Client::builder().build()?; // NODEMAVEN_APIKEY from the environment
//! let me = client.me()?;
//! println!("{} bytes of traffic left", me["data"]);
//!
//! let cities = client.cities(&[("country__code", "us")])?;
//! let every_city = client.iterate(cities).collect::<Result<Vec<_>, _>>()?;
//! # Ok(())
//! # }
//! ```
//!
//! Five things a caller needs, all measured:
//!
//! - **Responses are returned unmodelled**, as [`serde_json::Value`]. The
//!   vendor's document is wrong about several of its own fields, so a struct
//!   would be a claim about names nobody here can promise.
//! - **`me()`, `sub_users()`, `create_sub_user()` and `reset_sub_user_usage()`
//!   return live proxy passwords.** Do not log their results whole.
//! - **A `2xx` whose body is not JSON is an [`Error::Api`].** The dashboard host
//!   answers a path it does not serve with `200` and its web page, so a status
//!   code alone never proves a call did anything.
//! - **One page is not the collection.** `limit` is capped at 1000 by the
//!   server; [`Client::iterate`] walks the rest.
//! - **Statistics dates are `dd-mm-yyyy`**, not the ISO form the vendor's
//!   document types them as, and omitting `start`, `end` and `period` together
//!   is answered `500`.

use std::collections::BTreeSet;
use std::fmt;
use std::io;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::error::{Error, Result};
use crate::proxy::{percent_encode, Proxy};

/// The dashboard host the API lives on. Overridden by `NODEMAVEN_BASE_URL`.
///
/// Not `api.nodemaven.com`, which the vendor's document names and which answers
/// `404` from nginx.
pub const DEFAULT_BASE_URL: &str = "https://dashboard.nodemaven.com";

/// Every path hangs off this.
pub const API_ROOT: &str = "/api/v2/base";

/// Rows asked for per request on the `limit`/`offset` endpoints: the largest
/// value every one of them accepts. `locations/isps/` refuses a request with no
/// `limit` at all, and the others answer one with 50 rows and no total.
pub const DEFAULT_PAGE_SIZE: u64 = 1000;

/// How long the built-in transport waits for a whole response.
pub const DEFAULT_API_TIMEOUT: Duration = Duration::from_secs(30);

const USER_AGENT: &str = "nodemaven-rust";
const REDACTED: &str = "***";

/// How a request reaches the API: `(method, url, headers, body)` in, the status
/// and the raw body out.
///
/// It returns the status rather than failing on it - mapping a status to an
/// [`Error`] is this module's job - and it fails only when no response came
/// back at all. An [`io::ErrorKind::TimedOut`] or [`io::ErrorKind::WouldBlock`]
/// is reported as a timeout; any other `io::Error` as a connection that failed
/// mid-request. Both become [`Error::Api`] with no status.
///
/// Any `Fn` with this shape is a transport, which is how tests and callers with
/// their own HTTP client plug in. The built-in one, behind the `http` feature,
/// uses `ureq`, reads no proxy from the environment and follows no redirect.
pub trait Transport: Send + Sync {
    /// Send one request and return `(status, body)`.
    fn send(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
    ) -> io::Result<(u16, Vec<u8>)>;
}

impl<F> Transport for F
where
    F: Fn(&str, &str, &[(String, String)], Option<&[u8]>) -> io::Result<(u16, Vec<u8>)>
        + Send
        + Sync,
{
    fn send(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
    ) -> io::Result<(u16, Vec<u8>)> {
        self(method, url, headers, body)
    }
}

/// How one endpoint numbers its pages. This API uses three conventions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Paging {
    /// The query parameter carrying the page size.
    pub size_key: &'static str,
    /// The query parameter carrying the cursor.
    pub cursor_key: &'static str,
    /// Whether the cursor counts rows (`offset`) rather than pages (`page`).
    pub cursor_counts_rows: bool,
    /// The size sent when the caller gives none, where one is measured safe.
    pub default_size: Option<u64>,
    /// The cursor of the first page: 0 for `offset`, 1 for both page-number
    /// endpoints (measured 2026-09-09).
    pub first_cursor: Option<u64>,
    /// Whether the server ends the collection with a `404` rather than an empty
    /// page. True on `whitelist/ips` alone; everywhere else a `404` is an error,
    /// because it is also what a wrong path looks like.
    pub ends_with_not_found: bool,
}

/// The `locations/*` endpoints: `limit`/`offset` from 0.
pub const BY_OFFSET: Paging = Paging {
    size_key: "limit",
    cursor_key: "offset",
    cursor_counts_rows: true,
    default_size: Some(DEFAULT_PAGE_SIZE),
    first_cursor: Some(0),
    ends_with_not_found: false,
};

/// `locations/isps/cities/`: pages of 100. This endpoint's time grows with the
/// page - 100 rows in 8.3 s, 1000 in 56.0 s, measured 2026-09-29 - and 1000 did
/// not fit in the default timeout.
pub const ISP_CITIES_PAGING: Paging = Paging {
    default_size: Some(100),
    ..BY_OFFSET
};

/// `sub-users/`: `page`/`per_page` from 1, no size sent - the document gives
/// `per_page` no default and no maximum.
pub const BY_PAGE_NUMBER: Paging = Paging {
    size_key: "per_page",
    cursor_key: "page",
    cursor_counts_rows: false,
    default_size: None,
    first_cursor: Some(1),
    ends_with_not_found: false,
};

/// `whitelist/ips`: `page`/`page_size` from 1, at the document's maximum of 100
/// because its default is 5, and a `404` past the end.
pub const WHITELIST_PAGING: Paging = Paging {
    size_key: "page_size",
    cursor_key: "page",
    cursor_counts_rows: false,
    default_size: Some(100),
    first_cursor: Some(1),
    ends_with_not_found: true,
};

/// One page of a list endpoint. **One page, not the collection** - see
/// [`Client::iterate`].
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    results: Vec<Value>,
    count: Option<u64>,
    next: Option<String>,
    previous: Option<String>,
    rows_key: String,
    request_path: Option<String>,
    request_params: Vec<(String, String)>,
    paging: Option<Paging>,
}

impl Page {
    /// The rows on this page.
    pub fn results(&self) -> &[Value] {
        &self.results
    }

    /// The total the server reported, which on this API it never does.
    pub fn count(&self) -> Option<u64> {
        self.count
    }

    /// A next-page url, if the server sent one. No declared schema has one.
    pub fn next(&self) -> Option<&str> {
        self.next.as_deref()
    }

    /// A previous-page url, if the server sent one.
    pub fn previous(&self) -> Option<&str> {
        self.previous.as_deref()
    }

    /// The field the rows came out of: `results`, `isps`, `regions`, `cities`,
    /// `payload` or `data`, per endpoint.
    pub fn rows_key(&self) -> &str {
        &self.rows_key
    }

    /// How many rows are on this page.
    pub fn len(&self) -> usize {
        self.results.len()
    }

    /// Whether this page has no rows.
    pub fn is_empty(&self) -> bool {
        self.results.is_empty()
    }
}

impl IntoIterator for Page {
    type Item = Value;
    type IntoIter = std::vec::IntoIter<Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.results.into_iter()
    }
}

/// Talks to the account API. One client, one API key.
pub struct Client {
    key: String,
    base: String,
    timeout: Duration,
    builtin_transport: bool,
    transport: Box<dyn Transport>,
}

impl fmt::Debug for Client {
    /// Never carries the key.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &self.base)
            .field("api_key", &REDACTED)
            .finish()
    }
}

/// Builds a [`Client`].
#[derive(Default)]
pub struct ClientBuilder {
    key: Option<String>,
    base: Option<String>,
    timeout: Option<Duration>,
    transport: Option<Box<dyn Transport>>,
}

impl fmt::Debug for ClientBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientBuilder")
            .field("base_url", &self.base)
            .field("api_key", &self.key.as_ref().map(|_| REDACTED))
            .field("timeout", &self.timeout)
            .field("transport", &self.transport.as_ref().map(|_| "custom"))
            .finish()
    }
}

impl ClientBuilder {
    /// The dashboard API key. Falls back to `NODEMAVEN_APIKEY`, the vendor's own
    /// name for it. Not the proxy password.
    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The API host. Falls back to `NODEMAVEN_BASE_URL`, then
    /// [`DEFAULT_BASE_URL`].
    pub fn base_url(mut self, base: impl Into<String>) -> Self {
        self.base = Some(base.into());
        self
    }

    /// How long the built-in transport waits for a whole response. Defaults to
    /// [`DEFAULT_API_TIMEOUT`]. A transport you pass runs on its own timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Send requests through `transport` instead of the built-in one.
    pub fn transport(mut self, transport: impl Transport + 'static) -> Self {
        self.transport = Some(Box::new(transport));
        self
    }

    /// Resolve the key and the host and hand over a client.
    pub fn build(self) -> Result<Client> {
        let key = self
            .key
            .filter(|key| !key.is_empty())
            .or_else(|| std::env::var("NODEMAVEN_APIKEY").ok())
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty())
            .ok_or_else(|| {
                Error::Credentials(
                    "no API key: pass api_key() to Client::builder() or set NODEMAVEN_APIKEY \
                     in the environment. This is the dashboard API key and not the proxy \
                     password - they are different secrets."
                        .into(),
                )
            })?;
        let base = self
            .base
            .filter(|base| !base.is_empty())
            .or_else(|| std::env::var("NODEMAVEN_BASE_URL").ok())
            .filter(|base| !base.is_empty())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string();
        let timeout = self.timeout.unwrap_or(DEFAULT_API_TIMEOUT);
        let (transport, builtin_transport) = match self.transport {
            Some(transport) => (transport, false),
            None => (builtin(timeout)?, true),
        };
        Ok(Client {
            key,
            base,
            timeout,
            builtin_transport,
            transport,
        })
    }
}

#[cfg(feature = "http")]
fn builtin(timeout: Duration) -> Result<Box<dyn Transport>> {
    Ok(Box::new(http::UreqTransport::new(timeout)))
}

#[cfg(not(feature = "http"))]
fn builtin(_timeout: Duration) -> Result<Box<dyn Transport>> {
    Err(Error::Api {
        status: None,
        message: "this build has no built-in HTTP transport: enable the `http` feature or \
                  pass one with ClientBuilder::transport()."
            .into(),
    })
}

impl Client {
    /// A builder. `Client::builder().build()` reads the key from the environment.
    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }

    // -- the account ------------------------------------------------------

    /// The account: `data` (traffic left, bytes), `email`, `is_traffic_frozen`
    /// (a **string**, not a bool), `proxy_password`, `proxy_username`,
    /// `subscription_status`. Carries the proxy password in clear text.
    pub fn me(&self) -> Result<Value> {
        self.get_object("GET", &format!("{API_ROOT}/users/me"), &[], None)
    }

    // -- where you can exit from ------------------------------------------

    /// Countries. `connection_type` defaults to `residential` on the server.
    pub fn countries(&self, filters: &[(&str, &str)]) -> Result<Page> {
        self.list("locations/countries/", filters, "results", Some(BY_OFFSET))
    }

    /// Regions. Filter with `country__code` - two underscores, the server's
    /// spelling.
    pub fn regions(&self, filters: &[(&str, &str)]) -> Result<Page> {
        self.list("locations/regions/", filters, "results", Some(BY_OFFSET))
    }

    /// Cities. The one catalogue a single call does not finish: 1000 rows per
    /// page against about 1950 US cities. City codes repeat across regions.
    pub fn cities(&self, filters: &[(&str, &str)]) -> Result<Page> {
        self.list("locations/cities/", filters, "results", Some(BY_OFFSET))
    }

    /// ISPs. Rows are under `isps`.
    pub fn isps(&self, filters: &[(&str, &str)]) -> Result<Page> {
        self.list("locations/isps/", filters, "isps", Some(BY_OFFSET))
    }

    /// Regions that have ISPs, grouped. Needs `country__code`.
    pub fn isp_regions(&self, filters: &[(&str, &str)]) -> Result<Page> {
        self.list(
            "locations/isps/regions/",
            filters,
            "regions",
            Some(BY_OFFSET),
        )
    }

    /// Cities that have ISPs, grouped. Needs `country__code`. Pages of 100 -
    /// see [`ISP_CITIES_PAGING`].
    pub fn isp_cities(&self, filters: &[(&str, &str)]) -> Result<Page> {
        self.list(
            "locations/isps/cities/",
            filters,
            "cities",
            Some(ISP_CITIES_PAGING),
        )
    }

    /// ZIP codes. Needs `country__code`. The path is `zipcodes`, solid.
    pub fn zip_codes(&self, filters: &[(&str, &str)]) -> Result<Page> {
        self.list("locations/zipcodes/", filters, "results", Some(BY_OFFSET))
    }

    /// Regions that have ZIP codes, grouped. Needs `country__code`.
    pub fn zip_code_regions(&self, filters: &[(&str, &str)]) -> Result<Page> {
        self.list(
            "locations/zipcodes/regions/",
            filters,
            "regions",
            Some(BY_OFFSET),
        )
    }

    /// Cities that have ZIP codes, grouped. Needs `country__code`.
    pub fn zip_code_cities(&self, filters: &[(&str, &str)]) -> Result<Page> {
        self.list(
            "locations/zipcodes/cities/",
            filters,
            "cities",
            Some(BY_OFFSET),
        )
    }

    // -- what you used ----------------------------------------------------

    /// Traffic over time: `{"labels": [...], "data": [...]}`. Dates are
    /// `dd-mm-yyyy`; send a `period` (`today`, `hours24`) or a `start`/`end`
    /// range - omitting all three is answered `500`.
    pub fn statistics_data(&self, proxy_username: &str, filters: &[(&str, &str)]) -> Result<Value> {
        let query = with(filters, "proxy_username", proxy_username);
        self.get_object("GET", &format!("{API_ROOT}/statistics/data/"), &query, None)
    }

    /// Request counts over time, in the same shape and with the same filters as
    /// [`Client::statistics_data`].
    pub fn statistics_requests(
        &self,
        proxy_username: &str,
        filters: &[(&str, &str)],
    ) -> Result<Value> {
        let query = with(filters, "proxy_username", proxy_username);
        self.get_object(
            "GET",
            &format!("{API_ROOT}/statistics/requests/"),
            &query,
            None,
        )
    }

    /// Usage by target domain. Not paginated; `limit` is a top-N cut.
    ///
    /// Each row is a **three-element array**, not the object the vendor's
    /// document declares (measured 2026-09-29). Which position holds the domain,
    /// the request count and the bytes is not measured, so the rows are
    /// returned as they arrive.
    pub fn domain_statistics(
        &self,
        proxy_username: &str,
        filters: &[(&str, &str)],
    ) -> Result<Page> {
        let query = with(filters, "proxy_username", proxy_username);
        let path = format!("{API_ROOT}/statistics/domains/");
        let body = self.request("GET", &path, &query, None)?;
        let mut page = page_from(body, "data")?;
        page.request_path = Some(path);
        page.request_params = query;
        Ok(page)
    }

    // -- sub-users --------------------------------------------------------

    /// The sub-users on this account. Rows are under `payload`, and **each
    /// carries its `proxy_password` in clear text**.
    pub fn sub_users(&self, filters: &[(&str, &str)]) -> Result<Page> {
        self.list("sub-users/", filters, "payload", Some(BY_PAGE_NUMBER))
    }

    /// Create a sub-user. `extra` adds fields such as `traffic_limit`. The
    /// server answers `201`, and the created object carries the password.
    pub fn create_sub_user(
        &self,
        proxy_username: &str,
        proxy_password: &str,
        extra: &[(&str, Value)],
    ) -> Result<Value> {
        let mut body = Map::new();
        body.insert("proxy_username".into(), proxy_username.into());
        body.insert("proxy_password".into(), proxy_password.into());
        for (name, value) in extra {
            body.insert((*name).to_string(), value.clone());
        }
        self.envelope("POST", &format!("{API_ROOT}/sub-users/"), &[], Some(body))
    }

    /// Change a sub-user: a `PUT` to the collection with the id in the body.
    /// Refuses an empty change set, which the server could answer `200` to
    /// while changing nothing.
    pub fn update_sub_user(
        &self,
        id: impl Into<Value>,
        changes: &[(&str, Value)],
    ) -> Result<Value> {
        let id = id.into();
        if changes.is_empty() {
            return Err(Error::Api {
                status: None,
                message: format!(
                    "update_sub_user({}) was given nothing to change. A body carrying only \
                     an id can be answered 200, so this would look like it worked and do \
                     nothing.",
                    text(&id)
                ),
            });
        }
        let mut body = Map::new();
        for (name, value) in changes {
            body.insert((*name).to_string(), value.clone());
        }
        body.insert("id".into(), id);
        self.envelope("PUT", &format!("{API_ROOT}/sub-users/"), &[], Some(body))
    }

    /// Delete a sub-user; the id travels as a query parameter. Irreversible.
    pub fn delete_sub_user(&self, id: impl Into<Value>) -> Result<Value> {
        let query = vec![("id".to_string(), text(&id.into()))];
        self.envelope("DELETE", &format!("{API_ROOT}/sub-users/"), &query, None)
    }

    /// Zero the recorded traffic of the given sub-users. Returns their full
    /// rows, **proxy passwords included**, as an array.
    pub fn reset_sub_user_usage(&self, ids: &[Value]) -> Result<Value> {
        let mut body = Map::new();
        body.insert("ids".into(), Value::Array(ids.to_vec()));
        self.envelope(
            "POST",
            &format!("{API_ROOT}/sub-users/reset/usage"),
            &[],
            Some(body),
        )
    }

    // -- authorising by address -------------------------------------------

    /// The addresses allowed to use this account without a password. The path
    /// has no trailing slash, and the collection ends with a `404`.
    pub fn whitelist_ips(&self, filters: &[(&str, &str)]) -> Result<Page> {
        self.list("whitelist/ips", filters, "results", Some(WHITELIST_PAGING))
    }

    /// One whitelisted address by the `ip_id` [`Client::upsert_whitelist_ip`]
    /// returns: the eighteen fields of the document's `WhitelistIP` schema.
    pub fn whitelist_ip(&self, ip_id: impl Into<Value>) -> Result<Value> {
        let path = format!(
            "{API_ROOT}/whitelist/ip/{}",
            percent_encode(&text(&ip_id.into()))
        );
        self.get_object("GET", &path, &[], None)
    }

    /// Add or update a whitelisted address. Returns `{"ip_id", "message"}`.
    ///
    /// `protocol` is sent as `HTTP` unless `extra` names one: the server does
    /// not apply the default its own document declares, and a body without it
    /// is refused `400` (measured 2026-09-09). Re-adding an address removed
    /// within the last hour may be refused as a duplicate.
    pub fn upsert_whitelist_ip(
        &self,
        ip: &str,
        ports_count: u32,
        extra: &[(&str, Value)],
    ) -> Result<Value> {
        let mut body = Map::new();
        body.insert("ip".into(), ip.into());
        body.insert("ports_count".into(), ports_count.into());
        body.insert("protocol".into(), "HTTP".into());
        for (name, value) in extra {
            body.insert((*name).to_string(), value.clone());
        }
        self.get_object(
            "POST",
            &format!("{API_ROOT}/whitelist/ip/upsert"),
            &[],
            Some(body),
        )
    }

    /// Remove an address from the whitelist, by `ip_id` or a listing row's `id`.
    pub fn delete_whitelist_ip(&self, ip_id: impl Into<Value>) -> Result<Value> {
        let path = format!(
            "{API_ROOT}/whitelist/ip/{}",
            percent_encode(&text(&ip_id.into()))
        );
        self.request("DELETE", &path, &[], None)
    }

    // -- paging -----------------------------------------------------------

    /// Every row from `page` onward, across pages, stopping after 100 pages.
    /// See [`Pages`].
    pub fn iterate(&self, page: Page) -> Pages<'_> {
        Pages {
            client: self,
            current: Some(page),
            index: 0,
            pages: 0,
            max_pages: 100,
            seen_urls: BTreeSet::new(),
            done: false,
        }
    }

    // -- checking a Proxy against the live catalogue ------------------------

    /// Problems with a [`Proxy`]'s location, checked against the catalogue.
    ///
    /// A `city` without a `region` is reported without any request - the
    /// gateway answers it `500`, which reads as a fault on its side. The
    /// `country` is then looked up in the country catalogue for the proxy's
    /// `type`; a country the catalogue lacks is answered `407` by the gateway,
    /// which reads as a credentials problem. An empty result means no problem
    /// was found, not that every value was checked: regions, cities and ISPs
    /// are not matched.
    pub fn validate(&self, proxy: &Proxy) -> Result<Vec<String>> {
        let param = |name: &str| {
            proxy
                .params()
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        let mut problems = Vec::new();
        if let (Some(city), None) = (param("city"), param("region")) {
            problems.push(format!(
                "city={city:?} was sent without a region. The gateway answers that with \
                 500 Internal Server Error, which reads as a fault on their side and is \
                 not one - the same city with its own region answers 200."
            ));
        }
        let Some(wanted) = param("country") else {
            return Ok(problems);
        };
        let connection_type = param("type").unwrap_or("residential");
        let page = self.countries(&[("connection_type", connection_type)])?;
        let mut codes = BTreeSet::new();
        for row in self.iterate(page) {
            if let Some(code) = row?.get("code").and_then(Value::as_str) {
                codes.insert(code.to_ascii_lowercase());
            }
        }
        if codes.is_empty() {
            problems.push(
                "the country catalogue came back with no readable codes, so nothing was \
                 checked. This is a bug here rather than a problem with your parameters - \
                 do not treat it as a pass."
                    .into(),
            );
        } else if wanted != "any" && !codes.contains(&wanted.to_ascii_lowercase()) {
            problems.push(format!(
                "country={wanted:?} is not in the catalogue for \
                 connection_type={connection_type:?}. The gateway answers this with 407 \
                 Proxy Authentication Required, which reads as a credentials problem and \
                 is not one."
            ));
        }
        Ok(problems)
    }

    // -- transport --------------------------------------------------------

    fn list(
        &self,
        path: &str,
        filters: &[(&str, &str)],
        rows_key: &str,
        paging: Option<Paging>,
    ) -> Result<Page> {
        let path = format!("{API_ROOT}/{path}");
        let mut query = owned(filters);
        if let Some(paging) = paging {
            if let Some(size) = paging.default_size {
                set_default(&mut query, paging.size_key, &size.to_string());
            }
            if let Some(first) = paging.first_cursor {
                set_default(&mut query, paging.cursor_key, &first.to_string());
            }
        }
        let body = self.request("GET", &path, &query, None)?;
        let mut page = page_from(body, rows_key)?;
        page.request_path = Some(path);
        page.request_params = query;
        page.paging = paging;
        Ok(page)
    }

    fn get_object(
        &self,
        method: &str,
        path: &str,
        query: &[(String, String)],
        body: Option<Map<String, Value>>,
    ) -> Result<Value> {
        let answer = self.request(method, path, query, body)?;
        if !answer.is_object() {
            return Err(Error::Api {
                status: None,
                message: format!(
                    "{method} {path} answered with {} where an object was expected. The \
                     API's shape has changed or something other than the API answered.",
                    kind(&answer)
                ),
            });
        }
        Ok(answer)
    }

    /// One sub-user call, with `{success, description, errors, payload}`
    /// unwrapped.
    fn envelope(
        &self,
        method: &str,
        path: &str,
        query: &[(String, String)],
        body: Option<Map<String, Value>>,
    ) -> Result<Value> {
        let answer = self.request(method, path, query, body)?;
        let Value::Object(mut object) = answer else {
            return Ok(answer);
        };
        // Checked before `payload` is looked for: a `success: false` with no
        // payload is the same disagreement, and it used to come back as a
        // result. Found in review 2026-09-29, in both SDKs.
        if object.get("success") == Some(&Value::Bool(false)) {
            let reason = detail(Some(&Value::Object(object.clone())));
            return Err(Error::Api {
                status: None,
                message: format!(
                    "{method} {path} answered 2xx with success=false: {}. A success flag \
                     inside a 2xx is the server disagreeing with its own status line.",
                    if reason.is_empty() {
                        "no reason given".into()
                    } else {
                        reason
                    }
                ),
            });
        }
        match object.remove("payload") {
            Some(payload) => Ok(payload),
            None => Ok(Value::Object(object)),
        }
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        query: &[(String, String)],
        body: Option<Map<String, Value>>,
    ) -> Result<Value> {
        let mut url = if path.starts_with("http") {
            // Only `iterate` passes an absolute url, and it is one the server
            // wrote. The check is here because this is where the key is
            // attached.
            if !same_origin(&self.base, path) {
                return Err(Error::Api {
                    status: None,
                    message: format!(
                        "refusing to send the API key to {path:?}, which is not {}. This \
                         url came back from the API as a paging link, and the request that \
                         would follow it carries your key in a header.",
                        self.base
                    ),
                });
            }
            path.to_string()
        } else {
            format!("{}{path}", self.base)
        };
        if !query.is_empty() {
            let encoded: Vec<String> = query
                .iter()
                .map(|(name, value)| format!("{}={}", percent_encode(name), percent_encode(value)))
                .collect();
            url.push(if url.contains('?') { '&' } else { '?' });
            url.push_str(&encoded.join("&"));
        }

        let mut headers = vec![
            // `x-api-key <key>` in `Authorization`, measured 2026-09-08: `Bearer`
            // and `Token` answer 403.
            (
                "Authorization".to_string(),
                format!("x-api-key {}", self.key),
            ),
            ("Accept".to_string(), "application/json".to_string()),
            ("User-Agent".to_string(), USER_AGENT.to_string()),
        ];
        let payload = body.map(|body| {
            headers.push(("Content-Type".to_string(), "application/json".to_string()));
            Value::Object(body).to_string().into_bytes()
        });

        let (status, raw) = self
            .transport
            .send(method, &url, &headers, payload.as_deref())
            .map_err(|error| {
                let reason = if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) {
                    if self.builtin_transport {
                        format!(
                            "no complete answer before the timeout of {} s",
                            self.timeout.as_secs_f64()
                        )
                    } else {
                        "no complete answer before the timeout".to_string()
                    }
                } else {
                    format!("the connection failed mid-request: {:?}", error.kind())
                };
                Error::Api {
                    status: None,
                    message: format!(
                        "{method} {url}: {reason}. No status came back, so this says \
                         nothing about the API key or the request."
                    ),
                }
            })?;
        interpret(status, &raw, method, &url)
    }
}

/// Every row from a page onward, across pages. Yields `Err` once and stops.
///
/// A `next` url is followed when the server sends one; otherwise the same call
/// is repeated at the next cursor - `offset` advanced by the rows that came
/// back, or `page + 1`. The walk stops on an **empty** page, not a short one,
/// because the server caps the page size below what was asked. Three refusals:
/// more than [`Pages::max_pages`] pages, a `next` url seen before, and a page
/// identical to the one before it, which means the server is ignoring the
/// cursor.
pub struct Pages<'a> {
    client: &'a Client,
    current: Option<Page>,
    index: usize,
    pages: usize,
    max_pages: usize,
    seen_urls: BTreeSet<String>,
    done: bool,
}

impl fmt::Debug for Pages<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pages")
            .field("pages", &self.pages)
            .field("max_pages", &self.max_pages)
            .finish()
    }
}

impl Pages<'_> {
    /// Stop after this many pages instead of 100. Reaching the bound is an
    /// error, not a truncated list.
    pub fn max_pages(mut self, max_pages: usize) -> Self {
        self.max_pages = max_pages;
        self
    }

    fn fail(&mut self, error: Error) -> Option<Result<Value>> {
        self.done = true;
        Some(Err(error))
    }
}

impl Iterator for Pages<'_> {
    type Item = Result<Value>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.done {
                return None;
            }
            let current = self.current.as_ref()?;
            if let Some(row) = current.results.get(self.index) {
                self.index += 1;
                return Some(Ok(row.clone()));
            }

            let url = current.next.clone();
            let step = if url.is_some() {
                None
            } else {
                next_step(current)
            };
            if url.is_none() && step.is_none() {
                self.done = true;
                return None;
            }
            self.pages += 1;
            if self.pages >= self.max_pages {
                let bound = self.max_pages;
                return self.fail(Error::Api {
                    status: None,
                    message: format!(
                        "stopped after {bound} pages, which is the bound in max_pages(). \
                         Raise it deliberately if the collection really is this large."
                    ),
                });
            }

            if let Some(url) = url {
                if !self.seen_urls.insert(url.clone()) {
                    let pages = self.pages;
                    return self.fail(Error::Api {
                        status: None,
                        message: format!(
                            "the API returned a next page url it had already returned \
                             ({url:?}), so following it is a loop. Stopped after {pages} pages."
                        ),
                    });
                }
                let rows_key = current.rows_key.clone();
                match self
                    .client
                    .request("GET", &url, &[], None)
                    .and_then(|body| page_from(body, &rows_key))
                {
                    Ok(page) => {
                        self.current = Some(page);
                        self.index = 0;
                    }
                    Err(error) => return self.fail(error),
                }
                continue;
            }

            let (path, asked) = step.expect("checked above");
            let paging = current.paging.expect("a step implies paging");
            let body = match self.client.request("GET", &path, &asked, None) {
                Ok(body) => body,
                Err(Error::NotFound(_)) if paging.ends_with_not_found => {
                    self.done = true;
                    return None;
                }
                Err(error) => return self.fail(error),
            };
            let following = match page_from(body, &current.rows_key) {
                Ok(page) => page,
                Err(error) => return self.fail(error),
            };
            if !following.results.is_empty() && following.results == current.results {
                let cursor = paging.cursor_key;
                let was = lookup(&current.request_params, cursor).unwrap_or_default();
                let now = lookup(&asked, cursor).unwrap_or_default();
                let rows = following.results.len();
                return self.fail(Error::Api {
                    status: None,
                    message: format!(
                        "asking {path} for {cursor}={now} returned the same {rows} rows as \
                         {cursor}={was}, so the server is accepting `{cursor}` and ignoring \
                         it. Every further page would repeat these rows."
                    ),
                });
            }
            self.current = Some(Page {
                request_path: Some(path),
                request_params: asked,
                paging: Some(paging),
                ..following
            });
            self.index = 0;
        }
    }
}

/// The request that would follow `page`, or `None` to stop.
fn next_step(page: &Page) -> Option<(String, Vec<(String, String)>)> {
    let path = page.request_path.as_ref()?;
    let paging = page.paging?;
    if page.request_params.is_empty() || page.results.is_empty() {
        return None;
    }
    let cursor: u64 = lookup(&page.request_params, paging.cursor_key)?
        .parse()
        .ok()?;
    let mut following = page.request_params.clone();
    let advanced = if paging.cursor_counts_rows {
        let size: u64 = lookup(&page.request_params, paging.size_key)?
            .parse()
            .ok()?;
        if size == 0 {
            return None;
        }
        // By the rows that came back, not the size asked for: the server caps
        // the size, and advancing by the request skips what the cap withheld.
        cursor + page.results.len() as u64
    } else {
        cursor + 1
    };
    set(&mut following, paging.cursor_key, &advanced.to_string());
    Some((path.clone(), following))
}

/// Turn a status and a body into a value or the right error.
fn interpret(status: u16, raw: &[u8], method: &str, url: &str) -> Result<Value> {
    let text = String::from_utf8_lossy(raw);
    let text = text.trim();
    let parsed: Option<Value> = if text.is_empty() {
        None
    } else {
        serde_json::from_str(text).ok()
    };
    let where_ = format!("{method} {url}");

    if (200..300).contains(&status) {
        if text.is_empty() {
            // 204 and an empty 200 are real answers to a DELETE, and to nothing
            // else here: every other call is answered with a JSON body, so an
            // empty one on a GET would otherwise become `{}` or an empty page
            // and read as "nothing there". Narrowed in review 2026-09-29.
            if method == "DELETE" {
                return Ok(Value::Object(Map::new()));
            }
            return Err(Error::Api {
                status: Some(status),
                message: format!(
                    "{where_} answered {status} with an empty body where JSON was \
                     expected, so there is nothing to read the answer from."
                ),
            });
        }
        return parsed.ok_or_else(|| {
            let what = if text.starts_with('<') {
                "HTML"
            } else {
                "text"
            };
            Error::Api {
                status: Some(status),
                message: format!(
                    "{where_} answered {status} with {} bytes of {what} where JSON was \
                     expected. This host answers a path it does not serve with 200 and its \
                     web front end, so the status says nothing about whether the call did \
                     anything.",
                    raw.len()
                ),
            }
        });
    }

    let reason = match &parsed {
        Some(value) => detail(Some(value)),
        None => text.to_string(),
    };
    let reason = if reason.is_empty() {
        format!("HTTP {status}")
    } else {
        reason
    };
    match status {
        401 | 403 => Err(Error::Auth {
            status,
            message: format!(
                "{where_} was refused: {reason}. This is the dashboard API key, not the \
                 proxy password - check NODEMAVEN_APIKEY."
            ),
        }),
        404 => Err(Error::NotFound(format!(
            "{where_} found nothing: {reason}."
        ))),
        429 => Err(Error::RateLimit(format!(
            "{where_} was rate limited: {reason}. Nothing here retries; back off on your \
             own schedule."
        ))),
        500.. => Err(Error::Api {
            status: Some(status),
            message: format!(
                "{where_} failed on the server: {reason}. A 5xx that repeats is worth \
                 reporting rather than hammering."
            ),
        }),
        _ => Err(Error::Api {
            status: Some(status),
            message: format!("{where_} was refused: {reason}."),
        }),
    }
}

/// A `Page` from whichever shape the endpoint used.
fn page_from(body: Value, rows_key: &str) -> Result<Page> {
    let empty = |results: Vec<Value>| Page {
        results,
        count: None,
        next: None,
        previous: None,
        rows_key: rows_key.to_string(),
        request_path: None,
        request_params: Vec::new(),
        paging: None,
    };
    match body {
        Value::Array(rows) => Ok(empty(rows)),
        Value::Object(mut object) => {
            if let Some(Value::Array(rows)) = object.remove(rows_key) {
                let mut page = empty(rows);
                page.count = object.get("count").and_then(Value::as_u64);
                page.next = object
                    .get("next")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                page.previous = object
                    .get("previous")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                return Ok(page);
            }
            if object.is_empty() {
                Ok(empty(Vec::new()))
            } else {
                // A single object where a list was expected: a filter that
                // matched one row.
                Ok(empty(vec![Value::Object(object)]))
            }
        }
        other => Err(Error::Api {
            status: None,
            message: format!(
                "a list endpoint answered with {}, which is neither a page nor a list. \
                 Either the API changed shape or something other than the API answered.",
                kind(&other)
            ),
        }),
    }
}

/// The most useful sentence in an error body, whatever shape it arrived in.
/// The API speaks four error dialects: `{detail}`, `{errors: {field: msg}}`,
/// the sub-user envelope, and `{error}` on the whitelist.
fn detail(parsed: Option<&Value>) -> String {
    let Some(value) = parsed else {
        return String::new();
    };
    match value {
        Value::String(text) => text.clone(),
        Value::Array(_) => flatten(value),
        Value::Object(object) => {
            for key in ["detail", "message", "error", "description"] {
                if let Some(Value::String(text)) = object.get(key) {
                    if !text.is_empty() {
                        return text.clone();
                    }
                }
            }
            match object.get("errors") {
                Some(Value::String(text)) if !text.is_empty() => text.clone(),
                Some(nested @ (Value::Object(_) | Value::Array(_))) if !is_empty(nested) => {
                    flatten(nested)
                }
                _ => flatten(value),
            }
        }
        _ => String::new(),
    }
}

fn flatten(value: &Value) -> String {
    match value {
        Value::Array(items) => items.iter().map(flatten).collect::<Vec<_>>().join("; "),
        Value::Object(object) => object
            .iter()
            .map(|(key, value)| format!("{key}: {}", flatten(value)))
            .collect::<Vec<_>>()
            .join(", "),
        other => text(other),
    }
}

fn is_empty(value: &Value) -> bool {
    match value {
        Value::Array(items) => items.is_empty(),
        Value::Object(object) => object.is_empty(),
        _ => false,
    }
}

/// Whether `url` goes to the same scheme, host and port as `base`. A url with
/// credentials in it, or a port that is not a port, is not.
fn same_origin(base: &str, url: &str) -> bool {
    match (origin(base), origin(url)) {
        (Some(here), Some(there)) => here == there,
        _ => false,
    }
}

fn origin(url: &str) -> Option<(String, String, u16)> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.contains('@') {
        return None;
    }
    let (host, port) = match authority.rfind(':') {
        Some(at) if !authority[at..].contains(']') => {
            let port = crate::check::port_number(&authority[at + 1..])?;
            (&authority[..at], port)
        }
        _ => (
            authority,
            match scheme.as_str() {
                "https" => 443,
                "http" => 80,
                _ => return None,
            },
        ),
    };
    if host.is_empty() {
        return None;
    }
    Some((scheme, host.to_ascii_lowercase(), port))
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a bool",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// An id or a value as it goes into a path or a query: a string bare, anything
/// else as JSON writes it.
fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn owned(filters: &[(&str, &str)]) -> Vec<(String, String)> {
    filters
        .iter()
        .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
        .collect()
}

fn with(filters: &[(&str, &str)], name: &str, value: &str) -> Vec<(String, String)> {
    let mut query = owned(filters);
    set(&mut query, name, value);
    query
}

fn lookup<'a>(query: &'a [(String, String)], name: &str) -> Option<&'a str> {
    query
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn set(query: &mut Vec<(String, String)>, name: &str, value: &str) {
    match query.iter_mut().find(|(key, _)| key == name) {
        Some(entry) => entry.1 = value.to_string(),
        None => query.push((name.to_string(), value.to_string())),
    }
}

fn set_default(query: &mut Vec<(String, String)>, name: &str, value: &str) {
    if lookup(query, name).is_none() {
        query.push((name.to_string(), value.to_string()));
    }
}

#[cfg(feature = "http")]
mod http {
    use std::io::{self, Read};
    use std::time::Duration;

    /// The built-in transport: `ureq`, with no proxy read from the environment
    /// and no redirect followed.
    ///
    /// `ureq` reads `HTTPS_PROXY` and friends by default, and those are set on
    /// exactly the machines that use proxies, so an API call would be routed
    /// through a proxy nobody asked for - the same trap the Python SDK closes
    /// with `ProxyHandler({})`. A redirect is not followed because the request
    /// carries the key in a header; a `3xx` comes back as an error with its
    /// status.
    pub(super) struct UreqTransport {
        agent: ureq::Agent,
    }

    impl UreqTransport {
        pub(super) fn new(timeout: Duration) -> Self {
            let config = ureq::Agent::config_builder()
                .proxy(None)
                .timeout_global(Some(timeout))
                .http_status_as_error(false)
                .max_redirects(0)
                .build();
            UreqTransport {
                agent: config.into(),
            }
        }
    }

    impl super::Transport for UreqTransport {
        fn send(
            &self,
            method: &str,
            url: &str,
            headers: &[(String, String)],
            body: Option<&[u8]>,
        ) -> io::Result<(u16, Vec<u8>)> {
            let result = match method {
                "GET" | "DELETE" => {
                    let mut request = if method == "GET" {
                        self.agent.get(url)
                    } else {
                        self.agent.delete(url)
                    };
                    for (name, value) in headers {
                        request = request.header(name, value);
                    }
                    request.call()
                }
                "POST" | "PUT" => {
                    let mut request = if method == "POST" {
                        self.agent.post(url)
                    } else {
                        self.agent.put(url)
                    };
                    for (name, value) in headers {
                        request = request.header(name, value);
                    }
                    request.send(body.unwrap_or_default())
                }
                other => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("method {other} is not used by this client"),
                    ))
                }
            };
            let mut response = result.map_err(to_io)?;
            let status = response.status().as_u16();
            let mut raw = Vec::new();
            response
                .body_mut()
                .as_reader()
                .read_to_end(&mut raw)
                .map_err(|error| match error.kind() {
                    io::ErrorKind::Other => to_io_text(&error.to_string()),
                    _ => error,
                })?;
            Ok((status, raw))
        }
    }

    fn to_io(error: ureq::Error) -> io::Error {
        match error {
            ureq::Error::Io(error) => error,
            ureq::Error::Timeout(_) => io::Error::new(io::ErrorKind::TimedOut, "timed out"),
            other => to_io_text(&other.to_string()),
        }
    }

    fn to_io_text(text: &str) -> io::Error {
        if text.contains("timeout") || text.contains("timed out") {
            io::Error::new(io::ErrorKind::TimedOut, text.to_string())
        } else {
            io::Error::other(text.to_string())
        }
    }
}
