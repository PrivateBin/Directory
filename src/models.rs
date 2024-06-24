use super::connections::*;
use super::functions::{rating_to_percent, strip_url};
use super::schema::checks;
use super::schema::instances;
use super::schema::scans;
use diesel::SqliteConnection;
use diesel::{
    backend::Backend,
    deserialize::{self, FromSql},
    dsl::AsExprOf,
    expression::{AsExpression},
    serialize::{self, Output, ToSql},
    sql_types::*,
    *,
};
use http_body_util::BodyExt; // BodyExt provides the Iterator trait
use hyper::body::{Body, Buf, Bytes}; // Body provides the size_hint() trait, Buf provides the reader() trait
use hyper::header::{CONTENT_SECURITY_POLICY, LOCATION};
use hyper::{Method, StatusCode};
use maxminddb::geoip2::Country;
use regex::Regex;
use rocket::serde::{json, Deserialize, Serialize};
use std::collections::HashMap;
use std::env::var;
use std::net::{IpAddr, ToSocketAddrs}; // ToSocketAddrs provides the to_socket_addrs() trait
use std::str::from_utf8;
use std::sync::atomic::AtomicU64;
use std::sync::OnceLock;
use std::sync::RwLock;
use url::Url;

pub const CSP_RECOMMENDATION: &str = "default-src 'none'; base-uri 'self'; \
    form-action 'none'; manifest-src 'self'; connect-src * blob:; \
    script-src 'self' 'unsafe-eval'; style-src 'self'; font-src 'self'; \
    frame-ancestors 'none'; img-src 'self' data: blob:; media-src blob:; \
    object-src blob:; sandbox allow-same-origin allow-scripts allow-forms \
    allow-popups allow-modals allow-downloads";
static CSP_MAP: &[(&str, &str)] = &[
    (
        "1.3.5",
        "default-src 'none'; manifest-src 'self'; connect-src * blob:; \
        script-src 'self' 'unsafe-eval' resource:; style-src 'self'; \
        font-src 'self'; img-src 'self' data: blob:; media-src blob:; \
        object-src blob:; sandbox allow-same-origin allow-scripts allow-forms \
        allow-popups allow-modals allow-downloads",
    ),
    (
        "1.3.",
        "default-src 'none'; manifest-src 'self'; connect-src * blob:; \
        script-src 'self' 'unsafe-eval'; style-src 'self'; font-src 'self'; \
        img-src 'self' data: blob:; media-src blob:; object-src blob:; sandbox \
        allow-same-origin allow-scripts allow-forms allow-popups allow-modals",
    ),
    (
        "1.3",
        "default-src 'none'; manifest-src 'self'; connect-src *; \
        script-src 'self' 'unsafe-eval'; style-src 'self'; font-src 'self'; \
        img-src 'self' data: blob:; media-src blob:; object-src blob:; sandbox \
        allow-same-origin allow-scripts allow-forms allow-popups allow-modals",
    ),
    (
        "1.2",
        "default-src 'none'; manifest-src 'self'; connect-src *; \
        script-src 'self'; style-src 'self'; font-src 'self'; img-src 'self' \
        data:; media-src data:; object-src data:; Referrer-Policy: 'no-referrer'; \
        sandbox allow-same-origin allow-scripts allow-forms allow-popups \
        allow-modals",
    ),
    (
        "1.1",
        "default-src 'none'; manifest-src 'self'; connect-src *; \
        script-src 'self'; style-src 'self'; font-src 'self'; \
        img-src 'self' data:; referrer no-referrer;",
    ),
    // since 1.7.2, with bootstrap5
    (
        "1.7.",
        "default-src 'self'; base-uri 'self'; form-action 'none'; \
        manifest-src 'self'; connect-src * blob:; script-src 'self' \
        'unsafe-eval'; style-src 'self'; font-src 'self'; \
        frame-ancestors 'none'; img-src 'self' data: blob:; media-src blob:; \
        object-src blob:; sandbox allow-same-origin allow-scripts allow-forms \
        allow-modals allow-downloads",
    ),
    // since 1.4
    ("1.", CSP_RECOMMENDATION),
];
const OBSERVATORY_API: &str = "https://http-observatory.security.mozilla.org/api/v1/analyze?host=";
const OBSERVATORY_MAX_CONTENT_LENGTH: u64 = 10240;
const MAX_LINE_COUNT: u16 = 1024;
pub const TITLE: &str = "Instance Directory";
static TEMPLATE_EXP: OnceLock<Regex> = OnceLock::new();
static VERSION_EXP: OnceLock<Regex> = OnceLock::new();

#[derive(Queryable)]
pub struct Check {
    pub id: i32,
    pub updated: u64,
    pub up: bool,
    pub instance_id: i32,
}

#[derive(Insertable)]
#[diesel(table_name = checks)]
pub struct CheckNew {
    pub up: bool,
    pub instance_id: i32,
}

#[database("directory")]
pub struct DirectoryDbConn(SqliteConnection);

#[derive(Clone, QueryableByName, Queryable, Serialize)]
#[serde(crate = "rocket::serde")]
#[diesel(table_name = instances)]
pub struct Instance {
    pub id: i32,
    pub url: String,
    pub version: String,
    pub https: bool,
    pub https_redirect: bool,
    pub country_id: String,
    pub attachments: bool,
    pub csp_header: bool,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub uptime: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub rating_mozilla_observatory: String,
}

impl Instance {
    // check country via geo IP database lookup
    pub async fn check_country(url: &str) -> Result<String, String> {
        let mut country_code = "AQ".into();
        if let Ok(parsed_url) = Url::parse(url) {
            let ip: IpAddr;
            if let Some(host) = parsed_url.domain() {
                let sockets = (host, 0).to_socket_addrs();
                if sockets.is_err() {
                    return Err(format!("Host or domain of URL {url} is not supported."));
                }
                let socket = sockets.unwrap().next();
                if socket.is_none() {
                    return Err(format!("Host or domain of URL {url} is not supported."));
                }
                ip = socket.unwrap().ip();
            } else if let Some(host) = parsed_url.host_str() {
                match host.parse() {
                    Ok(parsed_ip) => ip = parsed_ip,
                    Err(_) => return Ok(country_code),
                }
            } else {
                return Ok(country_code);
            }

            let geoip_mmdb =
                var("GEOIP_MMDB").expect("environment variable GEOIP_MMDB needs to be set");
            let opener = maxminddb::Reader::open_readfile(&geoip_mmdb);
            if opener.is_err() {
                return Err(
                    format!(
                        "Error opening geo IP database {geoip_mmdb} (defined in environment variable GEOIP_MMDB)."
                    )
                );
            }
            let reader = opener.unwrap();
            let country: Country = reader.lookup(ip).unwrap();
            country_code = country.country.unwrap().iso_code.unwrap().into();
        }
        Ok(country_code)
    }

    // check for HTTP to HTTPS redirect
    pub async fn check_http(url: &str) -> Result<(bool, bool, String), String> {
        let mut https = false;
        let mut https_redirect = false;
        let mut http_url = url.to_string();
        let mut resulting_url = url.into();

        if url.starts_with("https://") {
            https = true;
            http_url.replace_range(..5, "http");
        }
        match request_head(&http_url).await {
            Ok(res) => {
                let redirection_codes = [
                    StatusCode::MULTIPLE_CHOICES,
                    StatusCode::MOVED_PERMANENTLY,
                    StatusCode::FOUND,
                    StatusCode::SEE_OTHER,
                    StatusCode::NOT_MODIFIED,
                    StatusCode::TEMPORARY_REDIRECT,
                    StatusCode::PERMANENT_REDIRECT,
                ];
                if redirection_codes.contains(&res.status()) && res.headers().contains_key(LOCATION)
                {
                    // check Location header
                    if let Ok(location) = res.headers()[LOCATION].to_str() {
                        if location.starts_with("https://") {
                            https_redirect = true;
                        }
                        if !https && https_redirect {
                            // if the given URL was HTTP, but we got redirected to https,
                            // check & store the HTTPS URL instead
                            resulting_url = strip_url(location.into());
                            https = true;
                        }
                    }
                }
            }
            Err(message) => {
                // only emit an error if this server is reported as HTTP,
                // HTTPS-only webservers, though uncommon, do enforce HTTPS
                if url.starts_with("http://") {
                    return Err(message);
                } else {
                    https_redirect = true;
                }
            }
        }
        Ok((https, https_redirect, resulting_url))
    }

    // check rating at mozilla observatory
    pub async fn check_rating_mozilla_observatory(url: &str) -> ScanNew {
        if let Ok(parsed_url) = Url::parse(url) {
            if let Some(host) = parsed_url.host_str() {
                let observatory_url = format!("{OBSERVATORY_API}{host}");
                if let Ok(res) = request_get(&observatory_url).await {
                    if res.status() == StatusCode::OK {
                        let response_content_length = match res.body().size_hint().upper() {
                            Some(length) => length,
                            None => OBSERVATORY_MAX_CONTENT_LENGTH + 1, // protect from malicious response
                        };
                        if response_content_length >= OBSERVATORY_MAX_CONTENT_LENGTH {
                            Default::default()
                        }
                        let body_bytes = res.collect().await.unwrap().aggregate();
                        let api_response = json::from_slice::<ObservatoryScan>(body_bytes.chunk());
                        if let Ok(api_response) = api_response {
                            if "FINISHED" == api_response.state {
                                return ScanNew::new("mozilla_observatory", api_response.grade, 0);
                            }
                        }
                        // initiate a rescan
                        let _ = request(
                            &observatory_url,
                            Method::POST,
                            &KEEPALIVE,
                            Bytes::from_static(b"hidden=true"),
                        )
                        .await;
                    }
                }
            }
        }
        Default::default()
    }

    // check robots.txt, if one exists, and bail if server doesn't want us to index the instance
    pub async fn check_robots(url: &str) -> Result<bool, String> {
        let robots_url = if url.ends_with('/') {
            format!("{url}robots.txt")
        } else {
            format!("{url}/robots.txt")
        };
        if let Ok(res) = request_get(&robots_url).await {
            if res.status() == StatusCode::OK {
                let mut rule_for_us = false;
                let body = match res.collect().await {
                    Ok(body) => body,
                    Err(_) => return Ok(true),
                };
                let reader = LineReader {
                    reader: body.aggregate().reader(),
                    line_count: 0,
                };
                for line in reader {
                    let line_str = match line {
                        Ok(string) => string,
                        _ => break,
                    };

                    if rule_for_us {
                        if line_str.starts_with("Disallow: /") {
                            return Err(format!(
                                "Web server on URL {url} doesn't want to get added to the directory."
                            ));
                        }
                        break;
                    } else if line_str.starts_with("User-agent: PrivateBinDirectoryBot") {
                        rule_for_us = true;
                    }
                }
            }
        }
        Ok(true)
    }

    pub async fn check_up(&self) -> bool {
        match request_head(&self.url).await {
            Ok(res) => res.status() == StatusCode::OK,
            Err(_) => false,
        }
    }

    pub fn format(flag: bool) -> String {
        if flag {
            "\u{2714}".into() // Heavy Check Mark
        } else {
            "\u{2718}".into() // Heavy Ballot X
        }
    }
}

#[derive(Insertable)]
#[diesel(table_name = instances)]
pub struct InstanceNew {
    #[diesel(deserialize_as = i32)]
    pub id: Option<i32>,
    pub url: String,
    pub version: String,
    pub https: bool,
    pub https_redirect: bool,
    pub country_id: String,
    pub attachments: bool,
    pub csp_header: bool,
    pub variant: InstanceVariant,
}

// sorted alphabetically for readability, values set by historical order
#[repr(i16)]
#[derive(PartialEq, FromSqlRow)]
pub enum InstanceVariant {
    Jitsi = 1,
    PrivateBin = 0,
}

impl<DB> ToSql<SmallInt, DB> for InstanceVariant
where
    DB: Backend,
    i16: ToSql<SmallInt, DB>,
{
    fn to_sql(&self, out: &mut Output<DB>) -> serialize::Result {
        (*self as i16).to_sql(out)
    }
}

impl AsExpression<SmallInt> for InstanceVariant {
    type Expression = AsExprOf<i16, SmallInt>;

    fn as_expression(self) -> Self::Expression {
        <i16 as AsExpression<SmallInt>>::as_expression(self as i16)
    }
}

impl<DB> FromSql<SmallInt, DB> for InstanceVariant
where
    DB: Backend,
    i16: FromSql<SmallInt, DB>,
{
    fn from_sql(bytes: DB::RawValue<'_>) -> deserialize::Result<Self> {
        match i16::from_sql(bytes)? {
            0 => Ok(InstanceVariant::PrivateBin),
            1 => Ok(InstanceVariant::Jitsi),
            int => Err(format!("Invalid variant {}", int).into()),
        }
    }
}

pub struct InstancesCache {
    pub timeout: AtomicU64,
    pub instances: RwLock<Vec<Instance>>,
    pub negative_lookups: RwLock<HashMap<String, u64>>,
}

struct LineReader<R> {
    reader: R,
    line_count: u16,
}

impl<R: std::io::BufRead> Iterator for LineReader<R> {
    type Item = Result<String, String>;

    fn next(&mut self) -> Option<Self::Item> {
        self.line_count += 1;
        if self.line_count == MAX_LINE_COUNT {
            return None;
        }
        let mut bytes_to_consume = 0;
        let result = match self.reader.fill_buf() {
            Ok(buffer) => {
                if buffer.is_empty() {
                    return None;
                }
                bytes_to_consume = match buffer.iter().position(|&c| c == b'\n') {
                    Some(newline_position) => {
                        let mut bytes = buffer.len();
                        if newline_position < bytes {
                            bytes = newline_position + 1;
                        }
                        bytes
                    }
                    None => buffer.len(),
                };
                match from_utf8(&buffer[..bytes_to_consume]) {
                    Ok(string) => Some(Ok(string.to_owned())),
                    Err(_) => Some(Err("Error reading the web server response. Invalid UTF-8 sequence detected in response body.".to_owned())),
                }
            }
            Err(e) => Some(Err(format!("Error reading the web server response. {e:?}"))),
        };
        if bytes_to_consume > 0 {
            self.reader.consume(bytes_to_consume);
        }
        result
    }
}

pub struct Jitsi {
    pub instance: InstanceNew,
    pub scans: Vec<ScanNew>,
}

impl Jitsi {
    pub async fn new(url: String) -> Result<Jitsi, String> {
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(format!("Not a valid URL: {url}"));
        }

        let check_url = strip_url(url);
        let (https, https_redirect, check_url) = Instance::check_http(&check_url).await?;
        // don't proceed if the robots.txt tells us not to index the instance
        Instance::check_robots(&check_url).await?;

        // remaining checks may run in parallel
        let check_properties = Self::check_properties(&check_url);
        let check_country = Instance::check_country(&check_url);
        let check_rating = Instance::check_rating_mozilla_observatory(&check_url);

        // collect results of async checks
        let version = check_properties.await?;
        let country_code = check_country.await?;
        let scans = vec![check_rating.await];

        if !version.is_empty() {
            return Ok(Jitsi {
                instance: InstanceNew {
                    id: None,
                    url: check_url,
                    version,
                    https,
                    https_redirect,
                    country_id: country_code,
                    attachments: false,
                    csp_header: false,
                    variant: InstanceVariant::Jitsi,
                },
                scans,
            });
        }
        Err(format!(
            "The URL {check_url} doesn't seem to be a Jitsi instance."
        ))
    }

    // check version of jitsi meet JS library
    async fn check_properties(url: &str) -> Result<String, String> {
        let res = request(url, Method::GET, &CLOSE, Bytes::new()).await?;
        let status = res.status();
        if status != StatusCode::OK {
            return Err(format!("Web server responded with status code {status}."));
        }

        let mut version = String::new();
        let body = match res.collect().await {
            Ok(body) => body,
            Err(_) => return Err("Error reading the web server response.".to_owned()),
        };
        let reader = LineReader {
            reader: body.aggregate().reader(),
            line_count: 0,
        };
        for line in reader {
            let line_str = match line {
                Ok(string) => string,
                Err(e) => e,
            };

            if version.is_empty() {
                if let Some(matches) = VERSION_EXP
                    .get_or_init(|| {
                        Regex::new(r"libs/lib-jitsi-meet.min.js\?v=(\d+\.*\d*)")
                            .unwrap()
                    })
                    .captures(&line_str)
                {
                    version = matches[3].into();
                }
            }
        }
        Ok(version)
    }
}

#[tokio::test]
async fn test_jitsi() {
    let url = "https://meet.dssr.ch".to_owned();
    let test_url = url.to_owned();
    let jitsi: = Jitsi::new(test_url).await.unwrap();
    assert_eq!(jitsi.instance.url, url);
    assert!(jitsi.instance.https);
    assert!(jitsi.instance.https_redirect);
    assert_eq!(jitsi.instance.country_id, "CH");
}

#[tokio::test]
async fn test_non_jitsi() {
    let jitsi: Result<Jitsi, String> = Jitsi::new("https://directory.rs".into()).await;
    assert!(jitsi.is_err());
    let jitsi: Result<Jitsi, String> = Jitsi::new("https://privatebin.net".into()).await;
    assert!(jitsi.is_err());
}

#[derive(Deserialize)]
#[serde(crate = "rocket::serde")]
struct ObservatoryScan<'r> {
    grade: &'r str,
    state: &'r str,
}

pub struct PrivateBin {
    pub instance: InstanceNew,
    pub scans: Vec<ScanNew>,
}

impl PrivateBin {
    pub async fn new(url: String) -> Result<PrivateBin, String> {
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(format!("Not a valid URL: {url}"));
        }

        let check_url = strip_url(url);
        let (https, https_redirect, check_url) = Instance::check_http(&check_url).await?;
        // don't proceed if the robots.txt tells us not to index the instance
        Instance::check_robots(&check_url).await?;

        // remaining checks may run in parallel
        let check_properties = Self::check_properties(&check_url);
        let check_country = Instance::check_country(&check_url);
        let check_rating = Instance::check_rating_mozilla_observatory(&check_url);

        // collect results of async checks
        let (version, attachments, csp_header) = check_properties.await?;
        let country_code = check_country.await?;
        let scans = vec![check_rating.await];

        if !version.is_empty() {
            return Ok(PrivateBin {
                instance: InstanceNew {
                    id: None,
                    url: check_url,
                    version,
                    https,
                    https_redirect,
                    country_id: country_code,
                    attachments,
                    csp_header,
                    variant: InstanceVariant::PrivateBin,
                },
                scans,
            });
        }
        Err(format!(
            "The URL {check_url} doesn't seem to be a PrivateBin instance."
        ))
    }

    // check version of privatebin / zerobin JS library, attachment support & CSP header
    async fn check_properties(url: &str) -> Result<(String, bool, bool), String> {
        let mut csp_header = false;
        let res = request(url, Method::GET, &CLOSE, Bytes::new()).await?;
        let status = res.status();
        if status != StatusCode::OK {
            return Err(format!("Web server responded with status code {status}."));
        }

        // collect Content-Security-Policy header
        let mut policy = String::new();
        if res.headers().contains_key(CONTENT_SECURITY_POLICY) {
            if let Ok(csp) = res.headers()[CONTENT_SECURITY_POLICY].to_str() {
                csp.clone_into(&mut policy);
            }
        }

        let mut version = String::new();
        let mut attachments = false;
        let mut template = PrivateBinTemplate::Unknown;
        let body = match res.collect().await {
            Ok(body) => body,
            Err(_) => return Err("Error reading the web server response.".to_owned()),
        };
        let reader = LineReader {
            reader: body.aggregate().reader(),
            line_count: 0,
        };
        for line in reader {
            let line_str = match line {
                Ok(string) => string,
                Err(e) => e,
            };

            if !attachments && line_str.contains(" id=\"attachment\" ") {
                attachments = true;
                if !version.is_empty() && template != PrivateBinTemplate::Unknown {
                    // we got version, template and attachment, stop parsing
                    break;
                }
            }
            if template == PrivateBinTemplate::Unknown {
                if let Some(matches) = TEMPLATE_EXP
                    .get_or_init(|| Regex::new(r"css/bootstrap(\d*)/").unwrap())
                    .captures(&line_str)
                {
                    template = if matches[1].is_empty() {
                        PrivateBinTemplate::Bootstrap3
                    } else {
                        PrivateBinTemplate::Bootstrap5
                    };
                }
            }
            if version.is_empty() {
                if let Some(matches) = VERSION_EXP
                    .get_or_init(|| {
                        Regex::new(r"js/(privatebin|zerobin).js\?(Alpha%20)?(\d+\.\d+\.*\d*)")
                            .unwrap()
                    })
                    .captures(&line_str)
                {
                    matches[3].clone_into(&mut version);
                }
            }
        }
        // check Content-Security-Policy header
        if !policy.is_empty() {
            for rule in CSP_MAP {
                if version.starts_with(rule.0)
                    && ((template == PrivateBinTemplate::Bootstrap3 // Bootstrap3 templates do not need popups
                        && policy.eq(&rule.1.to_string().replace(" allow-popups", "")))
                        || policy.eq(rule.1))
                {
                    csp_header = true;
                    break;
                }
            }
            // versions before 1.0 didn't come with a CSP, so if we get to here, we'll give you a brownie point for trying
            if version.starts_with("0.") {
                csp_header = true;
            }
        }
        Ok((version, attachments, csp_header))
    }
}

#[tokio::test]
async fn test_privatebin() {
    let url = "https://privatebin.net".to_owned();
    let test_url = url.to_owned();
    let privatebin = PrivateBin::new(test_url).await.unwrap();
    assert_eq!(privatebin.instance.url, url);
    assert!(privatebin.instance.https);
    assert!(privatebin.instance.https_redirect);
    assert!(privatebin.instance.csp_header);
    assert!(privatebin.instance.attachments);
    assert_eq!(privatebin.instance.country_id, "CH");
}

#[tokio::test]
async fn test_url_rewrites() {
    let components = ["https", "http"].iter().flat_map(|schema| {
        ["/", "/?foo", "/#foo", "//", "/index.php"]
            .iter()
            .map(move |suffix| (schema, suffix))
    });
    for (schema, suffix) in components {
        let url = format!("{schema}://privatebin.net{suffix}");
        let privatebin = PrivateBin::new(url).await.unwrap();
        assert_eq!(
            privatebin.instance.url,
            "https://privatebin.net".to_string()
        );
    }
}

#[tokio::test]
async fn test_non_privatebin() {
    let privatebin = PrivateBin::new("https://directory.rs".into()).await;
    assert!(privatebin.is_err());
    let privatebin = PrivateBin::new("https://meet.dssr.ch".into()).await;
    assert!(privatebin.is_err());
}

#[tokio::test]
async fn test_robots_txt() {
    let privatebin = PrivateBin::new("http://zerobin-test.dssr.ch".into()).await;
    assert!(privatebin.is_err());
}

#[tokio::test]
async fn test_zerobin() {
    let url = "http://zerobin-legacy.dssr.ch/".to_string();
    let test_url = url.trim_end_matches('/').to_owned();
    let privatebin = PrivateBin::new(url).await.unwrap();
    assert_eq!(privatebin.instance.url, test_url);
    assert!(!privatebin.instance.https);
    assert!(!privatebin.instance.https_redirect);
    assert!(privatebin.instance.csp_header);
    assert_eq!(privatebin.instance.version, "0.20");
    assert!(!privatebin.instance.attachments);
    assert_eq!(privatebin.instance.country_id, "CH");
}

/* disabled test, instance no longer exists and I couldn't find another one configured like this:
$ curl --header "Accept: application/json" "https://privatebin.info/directory/api?https_redirect=true&https=true&top=100" | jq -r .[].url | sed 's#https://#http://#' | xargs curl -sv -o/dev/null 2>&1 | grep "Failed to connect"
#[tokio::test]
async fn test_no_http() {
    let url = "https://pasta.lysergic.dev".to_string();
    let privatebin = PrivateBin::new(url.to_owned()).await.unwrap();
    assert_eq!(privatebin.instance.url, url);
    assert!(privatebin.instance.https);
    assert!(privatebin.instance.https_redirect);
} */

#[tokio::test]
async fn test_idn() {
    let privatebin = PrivateBin::new("https://zerobin-täst.dssr.ch".into()).await;
    assert!(privatebin.is_err());
}

#[derive(PartialEq)]
enum PrivateBinTemplate {
    Bootstrap3,
    Bootstrap5,
    Unknown,
}

#[derive(Queryable)]
pub struct Scan {
    pub id: i32,
    pub scanner: String,
    pub rating: String,
    pub percent: i32,
    pub instance_id: i32,
}

#[derive(Insertable, Clone)]
#[diesel(table_name = scans)]
pub struct ScanNew {
    pub scanner: String,
    pub rating: String,
    pub percent: i32,
    pub instance_id: i32,
}

impl ScanNew {
    pub fn new(scanner: &str, rating: &str, instance_id: i32) -> ScanNew {
        let percent: i32 = rating_to_percent(rating).into();
        ScanNew {
            scanner: scanner.into(),
            rating: rating.into(),
            percent,
            instance_id,
        }
    }
}

impl Default for ScanNew {
    fn default() -> Self {
        ScanNew::new("mozilla_observatory", "-", 0)
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(crate = "rocket::serde")]
pub struct Page {
    pub title: String,
    pub topic: String,
}

impl Page {
    pub fn new(topic: String) -> Page {
        Page {
            title: TITLE.into(),
            topic,
        }
    }
}

#[derive(Serialize)]
#[serde(crate = "rocket::serde")]
pub struct InstancePage {
    pub title: String,
    pub topic: String,
    pub csp_recommendation: String,
    pub instance: Option<Instance>,
    pub error: String,
}

impl InstancePage {
    pub fn new(topic: String, instance: Option<Instance>, error: Option<String>) -> InstancePage {
        let error_string = error.unwrap_or_default();
        InstancePage {
            title: TITLE.into(),
            topic,
            csp_recommendation: CSP_RECOMMENDATION.into(),
            instance,
            error: error_string,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(crate = "rocket::serde")]
pub struct StatusPage {
    pub title: String,
    pub topic: String,
    pub error: String,
    pub success: String,
}

impl StatusPage {
    pub fn new(topic: String, error: Option<String>, success: Option<String>) -> StatusPage {
        let error_string = error.unwrap_or_default();
        let success_string = success.unwrap_or_default();
        StatusPage {
            title: TITLE.into(),
            topic,
            error: error_string,
            success: success_string,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(crate = "rocket::serde")]
pub struct TablePage {
    pub title: String,
    pub topic: String,
    pub tables: Vec<HtmlTable>,
}

impl TablePage {
    pub fn new(topic: String, tables: Vec<HtmlTable>) -> TablePage {
        TablePage {
            title: TITLE.into(),
            topic,
            tables,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(crate = "rocket::serde")]
pub struct HtmlTable {
    pub title: String,
    pub header: [String; 9],
    pub body: Vec<[String; 10]>,
}

#[derive(Debug, FromForm)]
pub struct AddForm {
    pub url: String,
}
