use super::connections::*;
use super::functions::{rating_to_percent, strip_url};
use super::schema::checks;
use super::schema::instances;
use super::schema::scans;
use diesel::SqliteConnection;
use dns_lookup::lookup_host;
use hyper::body::{aggregate, to_bytes, Buf}; // Buf provides the reader() trait
use hyper::header::{CONTENT_SECURITY_POLICY, LOCATION};
use hyper::{Body, Method, StatusCode};
use maxminddb::geoip2::Country;
use regex::Regex;
use rocket::serde::{json, Deserialize, Serialize};
use std::collections::HashMap;
use std::env::var;
use std::io::BufRead; // provides the lines() trait
use std::net::IpAddr;
use std::sync::atomic::AtomicU64;
use std::sync::RwLock;
use url::Url;

pub const CSP_RECOMMENDATION: &str = "default-src 'none'; base-uri 'self'; \
    form-action 'none'; manifest-src 'self'; connect-src * blob:; \
    script-src 'self' 'unsafe-eval'; style-src 'self'; font-src 'self'; \
    frame-ancestors 'none'; img-src 'self' data: blob:; media-src blob:; \
    object-src blob:; sandbox allow-same-origin allow-scripts allow-forms \
    allow-popups allow-modals allow-downloads";
const OBSERVATORY_API: &str = "https://http-observatory.security.mozilla.org/api/v1/analyze?host=";
pub const TITLE: &str = "Instance Directory";

lazy_static! {
    static ref VERSION_EXP: Regex =
        Regex::new(r"js/(privatebin|zerobin).js\?(Alpha%20)?(\d+\.\d+\.*\d*)").unwrap();
}

#[derive(Queryable)]
pub struct Check {
    pub id: i32,
    pub updated: u64,
    pub up: bool,
    pub instance_id: i32,
}

#[derive(Insertable)]
#[table_name = "checks"]
pub struct CheckNew {
    pub up: bool,
    pub instance_id: i32,
}

#[database("directory")]
pub struct DirectoryDbConn(SqliteConnection);

#[derive(Clone, QueryableByName, Queryable, Serialize)]
#[serde(crate = "rocket::serde")]
#[table_name = "instances"]
pub struct Instance {
    pub id: i32,
    pub url: String,
    pub version: String,
    pub https: bool,
    pub https_redirect: bool,
    pub country_id: String,
    pub attachments: bool,
    pub csp_header: bool,
    #[sql_type = "diesel::sql_types::Integer"]
    pub uptime: i32,
    #[sql_type = "diesel::sql_types::Text"]
    pub rating_mozilla_observatory: String,
}

impl Instance {
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
#[table_name = "instances"]
pub struct InstanceNew {
    pub url: String,
    pub version: String,
    pub https: bool,
    pub https_redirect: bool,
    pub country_id: String,
    pub attachments: bool,
    pub csp_header: bool,
}

pub struct InstancesCache {
    pub timeout: AtomicU64,
    pub instances: RwLock<Vec<Instance>>,
    pub negative_lookups: RwLock<HashMap<String, u64>>,
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
        let (https, https_redirect, check_url) = Self::check_http(&check_url).await?;
        // don't proceed if the robots.txt tells us not to index the instance
        Self::check_robots(&check_url).await?;

        // remaining checks may run in parallel
        let check_properties = Self::check_properties(&check_url);
        let check_country = Self::check_country(&check_url);
        let check_rating = Self::check_rating_mozilla_observatory(&check_url);

        // collect results of async checks
        let (version, attachments, csp_header) = check_properties.await?;
        let country_code = check_country.await?;
        let scans = vec![check_rating.await];

        if !version.is_empty() {
            return Ok(PrivateBin {
                instance: InstanceNew {
                    url: check_url,
                    version,
                    https,
                    https_redirect,
                    country_id: country_code,
                    attachments,
                    csp_header,
                },
                scans,
            });
        }
        Err(format!(
            "The URL {check_url} doesn't seem to be a PrivateBin instance."
        ))
    }

    // check country via geo IP database lookup
    async fn check_country(url: &str) -> Result<String, String> {
        let mut country_code = "AQ".into();
        if let Ok(parsed_url) = Url::parse(url) {
            let ip: IpAddr;
            if let Some(host) = parsed_url.domain() {
                let ips = lookup_host(host);
                if ips.is_err() {
                    return Err(format!("Host or domain of URL {url} is not supported."));
                }
                ip = ips.unwrap()[0]
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
    async fn check_http(url: &str) -> Result<(bool, bool, String), String> {
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

    // check version of privatebin / zerobin JS library, attachment support & CSP header
    async fn check_properties(url: &str) -> Result<(String, bool, bool), String> {
        let mut csp_header = false;
        let result = request(url, Method::GET, &CLOSE, Body::empty()).await;
        let res = result?;
        let status = res.status();
        if status != StatusCode::OK {
            return Err(format!("Web server responded with status code {status}."));
        }

        // check Content-Security-Policy header
        if res.headers().contains_key(CONTENT_SECURITY_POLICY) {
            if let Ok(csp) = res.headers()[CONTENT_SECURITY_POLICY].to_str() {
                if csp.eq(CSP_RECOMMENDATION) {
                    csp_header = true;
                }
            }
        }

        let mut version = String::new();
        let mut attachments = false;
        let body = match aggregate(res).await {
            Ok(body) => body,
            Err(_) => return Err("Error reading the web server response.".to_owned()),
        };
        for line in body.reader().lines() {
            let line_str = line.unwrap();
            if line_str.contains(" id=\"attachment\" ") {
                attachments = true;
                if !version.is_empty() {
                    // we got both version and attachment, stop parsing
                    break;
                }
            }
            if !version.is_empty() {
                // we got the version already, keep looking for the attachment
                continue;
            }
            if let Some(matches) = VERSION_EXP.captures(&line_str) {
                version = matches[3].into();
            }
        }
        Ok((version, attachments, csp_header))
    }

    // check rating at mozilla observatory
    pub async fn check_rating_mozilla_observatory(url: &str) -> ScanNew {
        if let Ok(parsed_url) = Url::parse(url) {
            if let Some(host) = parsed_url.host_str() {
                let observatory_url = format!("{OBSERVATORY_API}{host}");
                if let Ok(res) = request_get(&observatory_url).await {
                    if res.status() == StatusCode::OK {
                        let body_bytes = to_bytes(res.into_body()).await.unwrap();
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
                            Body::from("hidden=true"),
                        )
                        .await;
                    }
                }
            }
        }
        Default::default()
    }

    // check robots.txt and bail if server doesn't want us to index the instance
    async fn check_robots(url: &str) -> Result<bool, String> {
        let robots_url = if url.ends_with('/') {
            format!("{url}robots.txt")
        } else {
            format!("{url}/robots.txt")
        };
        if let Ok(res) = request_get(&robots_url).await {
            if res.status() == StatusCode::OK {
                let mut rule_for_us = false;
                let body = aggregate(res).await.unwrap();
                for line in body.reader().lines() {
                    let line_str = line.unwrap();
                    if !rule_for_us && line_str.starts_with("User-agent: PrivateBinDirectoryBot") {
                        rule_for_us = true;
                        continue;
                    }
                    if rule_for_us {
                        if line_str.starts_with("Disallow: /") {
                            return Err(format!(
                                "Web server on URL {url} doesn't want to get added to the directory."
                            ));
                        }
                        break;
                    }
                }
            }
        }
        Ok(true)
    }
}

#[tokio::test]
async fn test_privatebin() {
    let url = "https://privatebin.net".to_owned();
    let test_url = url.to_owned();
    let privatebin = PrivateBin::new(test_url).await.unwrap();
    assert_eq!(privatebin.instance.url, url);
    assert_eq!(privatebin.instance.version, "1.4.0");
    assert_eq!(privatebin.instance.https, true);
    assert_eq!(privatebin.instance.https_redirect, true);
    assert_eq!(privatebin.instance.csp_header, true);
    assert_eq!(privatebin.instance.attachments, true);
    assert_eq!(privatebin.instance.country_id, "CH");
}

#[tokio::test]
async fn test_url_rewrites() {
    let components = ["https", "http"].iter().flat_map(|schema| {
        ["/", "/?foo", "/#foo", "//"]
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
    let privatebin = PrivateBin::new("https://privatebin.info".into()).await;
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
    assert_eq!(privatebin.instance.https, false);
    assert_eq!(privatebin.instance.https_redirect, false);
    assert_eq!(privatebin.instance.csp_header, true);
    assert_eq!(privatebin.instance.version, "0.20");
    assert_eq!(privatebin.instance.attachments, false);
    assert_eq!(privatebin.instance.country_id, "CH");
}

#[tokio::test]
async fn test_no_http() {
    let url = "https://pasta.lysergic.dev".to_string();
    let privatebin = PrivateBin::new(url.to_owned()).await.unwrap();
    assert_eq!(privatebin.instance.url, url);
    assert_eq!(privatebin.instance.https, true);
    assert_eq!(privatebin.instance.https_redirect, true);
}

#[tokio::test]
async fn test_idn() {
    let url = "https://тайны.миры-аномалии.рф".to_string();
    let privatebin = PrivateBin::new(url.to_owned()).await.unwrap();
    assert_eq!(privatebin.instance.url, url);
    assert_eq!(privatebin.instance.https, true);
    assert_eq!(privatebin.instance.https_redirect, true);
    assert_ne!(privatebin.instance.country_id, "AQ");
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
#[table_name = "scans"]
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
