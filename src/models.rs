extern crate hyper_sync_rustls;
use dns_lookup::lookup_host;
use hyper::Client;
use hyper::client::RedirectPolicy;
use hyper::header::{Connection, Location, UserAgent};
use hyper::net::HttpsConnector;
use hyper::status::{StatusClass, StatusCode};
use maxminddb::geoip2::Country;
use regex::Regex;
use serde::Serialize;
use std::io::{BufReader, BufRead};
use std::sync::atomic::AtomicU64;
use std::sync::RwLock;
use super::schema::instances;
use super::schema::checks;

pub const TITLE: &str = "Instance Directory";

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

impl CheckNew {
    pub fn new(up: bool, instance_id: i32) -> CheckNew {
        CheckNew {
            up,
            instance_id,
        }
    }
}

#[derive(QueryableByName, Queryable)]
#[table_name = "instances"]
pub struct Instance {
    pub id: i32,
    pub url: String,
    pub version: String,
    pub https: bool,
    pub https_redirect: bool,
    pub country_id: String,
    pub attachments: bool,
    #[sql_type = "diesel::sql_types::Integer"]
    pub uptime: i32,
}

impl Instance {
    pub fn format(flag: bool) -> String {
        if flag {
            String::from("\u{2714}")
        } else {
            String::from("\u{2718}")
        }
    }

    pub fn format_country(country_id: String) -> String {
        // 1F1E6 is the unicode code point for the "REGIONAL INDICATOR SYMBOL
        // LETTER A" and A is 65 in unicode and ASCII, so we can calculate the
        // the unicode flags as follows:
        let mut country_chars = country_id.chars();
        let country_code_points = [
            std::char::from_u32(0x1F1E6 - 65 + country_chars.next().unwrap() as u32).unwrap(),
            std::char::from_u32(0x1F1E6 - 65 + country_chars.next().unwrap() as u32).unwrap()
        ];
        country_code_points.iter().cloned().collect::<String>()
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
}

impl InstanceNew {
    pub fn new(url: String, version: String, https: bool, https_redirect: bool, country_id: String, attachments: bool) -> InstanceNew {
        InstanceNew {
            url,
            version,
            https,
            https_redirect,
            country_id,
            attachments,
        }
    }
}

pub struct InstancesCache {
    pub timeout: AtomicU64,
    pub instances: RwLock<Vec<Instance>>
}

pub struct PrivateBin {
    pub instance: InstanceNew,
}

impl PrivateBin {
    pub fn new(url: String) -> Result<PrivateBin, String> {
        let validation = Self::validate(url)?;
        Ok(validation)
    }

    fn validate(url: String) -> Result<PrivateBin, String> {
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(format!("Not a valid URL: {}", url))
        }

        let mut check_url = url;
        // remove trailing slash, but only for web root, not for paths:
        // - https://example.com/ -> https://example.com
        // - but https://example.com/path/ remains unchanged
        if check_url.matches('/').count() == 3 {
            check_url = check_url.trim_end_matches('/').to_string();
        }

        let mut client = Client::with_connector(
            HttpsConnector::new(
                hyper_sync_rustls::TlsClient::new()
            )
        );
        client.set_redirect_policy(RedirectPolicy::FollowNone);

        let (https, https_redirect, check_url) = Self::check_http(&check_url, &client)?;
        Self::check_robots(&check_url, &client)?;
        let country_code = Self::check_country(&check_url)?;
        let (version, attachments) = Self::check_version(&check_url, &client)?;

        if !version.is_empty() {
            return Ok(
                PrivateBin {
                    instance: InstanceNew::new(
                        check_url,
                        version,
                        https,
                        https_redirect,
                        country_code,
                        attachments,
                    ),
                }
            )
        }
        Err(format!("The URL {} doesn't seem to be a PrivateBin instance.", check_url))
    }

    // check country via geo IP database lookup
    fn check_country(url: &str) -> Result<String, String> {
        let mut country_code = "AQ".to_string();
        let hostname_regex = Regex::new(
            r"^https?://((([[:alnum:]]|[[:alnum:]][[:alnum:]-]*[[:alnum:]])\.)*([[:alnum:]]|[[:alnum:]][[:alnum:]-]*[[:alnum:]])+)/?.*$"
        ).unwrap();
        if let Some(hostname_matches) = hostname_regex.captures(url) {
            let ips = lookup_host(&hostname_matches[1]);
            if ips.is_err() {
                return Err(format!("Host or domain of URL {} is not supported.", url))
            }

            let geoip_mmdb = std::env::var("GEOIP_MMDB").expect("environment variable GEOIP_MMDB needs to be set");
            let reader = maxminddb::Reader::open_readfile(&geoip_mmdb);
            if reader.is_err() {
                return Err(
                    format!(
                        "Error opening geo IP database {} (defined in environment variable GEOIP_MMDB).",
                        geoip_mmdb
                    )
                )
            }
            let country: Country = reader.unwrap().lookup(ips.unwrap()[0]).unwrap();
            country_code = country.country.unwrap().iso_code.unwrap();
        }
        Ok(country_code)
    }

    // check for HTTP to HTTPS redirect
    fn check_http(url: &str, client: &Client) -> Result<(bool, bool, String), String> {
        let mut https = false;
        let mut https_redirect = false;
        let mut http_url = url.to_string();
        let mut resulting_url = url.to_string();

        if url.starts_with("https://") {
            https = true;
            http_url.replace_range(..5, "http");
        }
        match client.head(&http_url)
            .header(Connection::keep_alive())
            .header(Self::get_user_agent())
            .send()
        {
            Ok(res) => {
                if res.status.class() == StatusClass::Redirection {
                    // check header
                    if let Some(location) = res.headers.get::<Location>() {
                        let location_str = location.to_string();
                        if location_str.starts_with("https://") {
                            https_redirect = true;
                        }
                        if !https && https_redirect {
                            // if the given URL was HTTP, but we got redirected to https,
                            // check & store the HTTPS URL instead
                            resulting_url = location_str;
                            // and trim trailing slashes again, only for web root
                            if url.matches('/').count() == 3 {
                                resulting_url = resulting_url.trim_end_matches('/').to_string();
                            }
                            https = true;
                        }
                    }
                }
            },
            Err(_) => {
                // only emit an error if this server is reported as HTTP,
                // HTTPS-only webservers are allowed, though uncommon
                if url.starts_with("http://") {
                    return Err(format!("Web server on URL {} is not responding.", http_url))
                }
            }
        }
        Ok((https, https_redirect, resulting_url))
    }

    // check robots.txt and bail if server doesn't want us to index the instance
    fn check_robots(url: &str, client: &Client) -> Result<bool, String> {
        let robots_url = if url.ends_with('/') {
            format!("{}robots.txt", url)
        } else {
            format!("{}/robots.txt", url)
        };
        let result = client.get(&robots_url)
            .header(Connection::keep_alive())
            .header(Self::get_user_agent())
            .send();
        if let Ok(res) = result {
            if res.status == StatusCode::Ok {
                let mut rule_for_us = false;
                let buffer = BufReader::new(res);
                for line in buffer.lines() {
                    let line_str = line.unwrap();
                    if !rule_for_us && line_str.starts_with("User-agent: PrivateBinDirectoryBot") {
                        rule_for_us = true;
                        continue;
                    }
                    if rule_for_us {
                        if line_str.starts_with("Disallow: /") {
                            return Err(
                                format!(
                                    "Web server on URL {} doesn't want to get added to the directory.",
                                    url
                                )
                            )
                        }
                        break;
                    }
                }
            }
        }
        Ok(true)
    }

    // check version of privatebin / zerobin JS library & attachment support
    fn check_version(url: &str, client: &Client) -> Result<(String, bool), String> {
        let result = client.get(url)
            .header(Connection::close())
            .header(Self::get_user_agent())
            .send();
        if result.is_err() {
            return Err(format!("Web server on URL {} is not responding.", url))
        }
        let res = result.unwrap();
        if res.status != StatusCode::Ok {
            return Err(format!("Web server responded with status code {}.", res.status))
        }

        let mut version = String::new();
        let mut attachments = false;
        let version_regex = Regex::new(r"js/(privatebin|zerobin).js\?(Alpha%20)?(\d+\.\d+\.*\d*)").unwrap();
        let buffer = BufReader::new(res);
        for line in buffer.lines() {
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
            if let Some(matches) = version_regex.captures(&line_str) {
                version = matches[3].to_string();
            }
        }
        Ok((version, attachments))
    }

    fn get_user_agent() -> UserAgent {
        UserAgent(
            format!(
                "PrivateBinDirectoryBot/{} (+https://privatebin.info/directory/about)",
                env!("CARGO_PKG_VERSION")
            )
        )
    }
}

#[test]
fn test_privatebin() {
    let url = String::from("https://privatebin.net");
    let privatebin = PrivateBin::new(url.clone()).unwrap();
    assert_eq!(privatebin.instance.url, url);
    assert_eq!(privatebin.instance.version, "1.3.4");
    assert_eq!(privatebin.instance.https, true);
    assert_eq!(privatebin.instance.https_redirect, true);
    assert_eq!(privatebin.instance.attachments, false);
    assert_eq!(privatebin.instance.country_id, "CH");
}

#[test]
fn test_non_privatebin() {
    let url = String::from("https://privatebin.info");
    let privatebin = PrivateBin::new(url);
    assert!(privatebin.is_err());
}

#[test]
fn test_zerobin() {
    let url = String::from("http://zerobin-legacy.dssr.ch/");
    let privatebin = PrivateBin::new(url.clone()).unwrap();
    assert_eq!(privatebin.instance.url, url.trim_end_matches('/').to_string());
    assert_eq!(privatebin.instance.https, false);
    assert_eq!(privatebin.instance.https_redirect, false);
    assert_eq!(privatebin.instance.version, "0.20");
    assert_eq!(privatebin.instance.attachments, false);
    assert_eq!(privatebin.instance.country_id, "CH");
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Page {
    pub title: String,
    pub topic: String,
}

impl Page {
    pub fn new(topic: String) -> Page {
        Page {
            title: String::from(TITLE),
            topic,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct StatusPage {
    pub title: String,
    pub topic: String,
    pub error: String,
    pub success: String,
}

impl StatusPage {
    pub fn new(topic: String, error: Option<String>, success: Option<String>) -> StatusPage {
        let error_string   = error.unwrap_or_default();
        let success_string = success.unwrap_or_default();
        StatusPage {
            title: String::from(TITLE),
            topic,
            error: error_string,
            success: success_string,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct TablePage {
    pub title: String,
    pub topic: String,
    pub tables: Vec<HtmlTable>,
}

impl TablePage {
    pub fn new(topic: String, tables: Vec<HtmlTable>) -> TablePage {
        TablePage {
            title: String::from(TITLE),
            topic,
            tables
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct HtmlTable {
    pub title: String,
    pub header: [String; 7],
    pub body: Vec<[String; 8]>,
}

#[derive(Debug, FromForm)]
pub struct AddForm {
    pub url: String
}
