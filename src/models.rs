use super::connections::*;
use super::functions::{rating_to_percent, strip_url};
use super::schema::checks;
use super::schema::instances;
use super::schema::scans;
use diesel::SqliteConnection;
use diesel::{
    backend::Backend,
    deserialize::{self, FromSql},
    expression::AsExpression,
    serialize::{self, Output, ToSql},
    sql_types::*,
    *,
};
use http_body_util::BodyExt; // BodyExt provides the Iterator trait
use hyper::body::{Body, Buf, Bytes}; // Body provides the size_hint() trait, Buf provides the reader() trait
use hyper::header::LOCATION;
use hyper::{Method, StatusCode};
use maxminddb::geoip2::Country;
use rocket::serde::{json, Deserialize, Serialize};
use std::collections::HashMap;
use std::env::var;
use std::net::{IpAddr, ToSocketAddrs}; // ToSocketAddrs provides the to_socket_addrs() trait
use std::str::from_utf8;
use std::sync::atomic::AtomicU64;
use std::sync::RwLock;
use url::Url;

pub mod jistsi;
pub mod privatebin;

const OBSERVATORY_API: &str = "https://http-observatory.security.mozilla.org/api/v1/analyze?host=";
const OBSERVATORY_MAX_CONTENT_LENGTH: u64 = 10240;
const MAX_LINE_COUNT: u16 = 1024;
pub const TITLE: &str = "Instance Directory";

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
#[repr(i32)]
#[derive(AsExpression, Debug, PartialEq, FromSqlRow)]
#[diesel(sql_type = Integer)]
pub enum InstanceVariant {
    Jitsi = 1,
    PrivateBin = 0,
}

impl<DB> ToSql<Integer, DB> for InstanceVariant
where
    DB: Backend,
    i32: ToSql<Integer, DB>,
{
    fn to_sql(&self, out: &mut Output<DB>) -> serialize::Result {
        match self {
            InstanceVariant::Jitsi => 1.to_sql(out),
            InstanceVariant::PrivateBin => 0.to_sql(out),
        }
    }
}

impl<DB> FromSql<Integer, DB> for InstanceVariant
where
    DB: Backend,
    i32: FromSql<Integer, DB>,
{
    fn from_sql(bytes: DB::RawValue<'_>) -> deserialize::Result<Self> {
        match i32::from_sql(bytes)? {
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

#[derive(Deserialize)]
#[serde(crate = "rocket::serde")]
struct ObservatoryScan<'r> {
    grade: &'r str,
    state: &'r str,
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
            csp_recommendation: privatebin::CSP_RECOMMENDATION.into(),
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
