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
const LATEST_PRIVATEBIN_VERSION: &str = "1.3.4";

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
            up: up,
            instance_id: instance_id,
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
            return String::from("\u{2714}")
        } else {
            return String::from("\u{2718}")
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
        return country_code_points.iter().cloned().collect::<String>()
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
    pub fn new(url: String, version: String, https: bool, https_redirect: bool, country_code: String, attachments: bool) -> InstanceNew {
        InstanceNew {
            url: url,
            version: version,
            https: https,
            https_redirect: https_redirect,
            country_id: country_code,
            attachments: attachments,
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
            return Err(format!("Not a valid URL: {}", url).to_string())
        }

        let mut check_url = url;
        // remove trailing slash, but only for web root, not for paths
        // https://example.com/ -> https://example.com BUT NOT https://example.com/path/
        if check_url.matches("/").count() == 3 {
            check_url = check_url.trim_end_matches('/');
        }

        let mut client = Client::with_connector(
            HttpsConnector::new(
                hyper_sync_rustls::TlsClient::new()
            )
        );
        client.set_redirect_policy(RedirectPolicy::FollowNone);

        // check for HTTPS redirect
        let mut https = false;
        let mut https_redirect = false;
        let mut http_url = check_url.clone();

        if check_url.starts_with("https://") {
            https = true;
            http_url.replace_range(..5, "http");
        }
        let result = client.head(&http_url)
            .header(Connection::keep_alive())
            .header(Self::get_user_agent())
            .send();
        if result.is_err() {
            // only emit an error if this server is reported as HTTP,
            // HTTPS-only webservers are allowed, though uncommon
            if check_url.starts_with("http://") {
                return Err(format!("Web server on URL {} is not responding.", http_url).to_string())
            }
        } else {
            let res = result.unwrap();
            if res.status.class() == StatusClass::Redirection {
                // check header
                let location = res.headers.get::<Location>();
                if location.is_some() {
                    let location_str = location.unwrap().to_string();
                    if location_str.starts_with("https://") {
                        https_redirect = true;
                    }
                    if check_url.starts_with("http://") && https_redirect {
                        // if the given URL was HTTP, but we got redirected to https,
                        // check & store the HTTPS URL instead
                        check_url = location_str;
                        // and trim trailing slashes again, only for web root
                        if check_url.matches("/").count() == 3 {
                            check_url = check_url.trim_end_matches('/');
                        }
                        https = true;
                    }
                }
            }
        }

        // check country via geo IP database lookup
        let mut country_code = "AQ".to_string();
        let hostname_regex = Regex::new(
            r"^https?://((([[:alnum:]]|[[:alnum:]][[:alnum:]-]*[[:alnum:]])\.)*([[:alnum:]]|[[:alnum:]][[:alnum:]-]*[[:alnum:]])+)/?.*$"
        ).unwrap();
        let hostname_matches = hostname_regex.captures(&check_url);
        if hostname_matches.is_some() {
            let ips = lookup_host(&hostname_matches.unwrap()[1]);
            if ips.is_err() {
                return Err(format!("Host or domain of URL {} is not supported.", check_url).to_string())
            }

            let geoip_mmdb = std::env::var("GEOIP_MMDB").expect("environment variable GEOIP_MMDB needs to be set");
            let reader = maxminddb::Reader::open_readfile(&geoip_mmdb);
            if reader.is_err() {
                return Err(
                    format!(
                        "Error opening geo IP database {} (defined in environment variable GEOIP_MMDB).",
                        geoip_mmdb
                    ).to_string()
                )
            }
            let country: Country = reader.unwrap().lookup(ips.unwrap()[0]).unwrap();
            country_code = country.country.unwrap().iso_code.unwrap();
        }

        // check version of privatebin / zerobin JS library
        let result = client.get(&check_url)
            .header(Connection::close())
            .header(Self::get_user_agent())
            .send();
        if result.is_err() {
            return Err(format!("Web server on URL {} is not responding.", check_url).to_string())
        }
        let res = result.unwrap();
        if res.status != StatusCode::Ok {
            return Err(format!("Web server responded with status code {}.", res.status).to_string())
        }

        let mut version = String::new();
        let mut attachments = false;
        let version_regexen = [
            Regex::new(r"js/privatebin.js\?(\d+\.\d+\.*\d*)").unwrap(),
            Regex::new(r"js/zerobin.js\?Alpha%20(\d+\.\d+\.*\d*)").unwrap()
        ];
        let buffer = BufReader::new(res);
        for line in buffer.lines() {
            let line_str = line.unwrap();
            if line_str.contains(" id=\"attachment\" ") {
                attachments = true;
                if version.len() > 0 {
                    // we got both version and attachment, stop parsing
                    break;
                }
            }
            if version.len() > 0 {
                // we got the version already, keep looking for the attachment
                continue;
            }
            for version_regex in version_regexen.iter() {
                let matches = version_regex.captures(&line_str);
                if matches.is_some() {
                    version = matches.unwrap()[1].to_string();
                    // we got the version, skip the other regex, if there is one
                    continue;
                }
            }
        }
        if version.len() > 0 {
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
        return Err(format!("The URL {} doesn't seem to be a PrivateBin instance.", check_url).to_string())
    }

    fn get_user_agent() -> UserAgent {
        return UserAgent(
            format!(
                "PrivateBinDirectoryBot/{} (+https://privatebin.info/directory/about)",
                env!("CARGO_PKG_VERSION")
            ).to_owned()
        )
    }
}

#[test]
fn test_privatebin() {
    let url = String::from("https://privatebin.net");
    let privatebin = PrivateBin::new(url.clone()).unwrap();
    assert_eq!(privatebin.instance.url, url);
    assert_eq!(privatebin.instance.version, LATEST_PRIVATEBIN_VERSION);
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
    let url = String::from("https://sebsauvage.net/paste/");
    let privatebin = PrivateBin::new(url.clone()).unwrap();
    assert_eq!(privatebin.instance.url, url);
    assert_eq!(privatebin.instance.version, "0.19.2");
    assert_eq!(privatebin.instance.attachments, false);
    assert_eq!(privatebin.instance.country_id, "IE");
}

#[test]
fn test_privatebin_http() {
    let url = String::from("http://zerobin-test.dssr.ch/");
    let privatebin = PrivateBin::new(url.clone()).unwrap();
    assert_eq!(privatebin.instance.url, url);
    assert_eq!(privatebin.instance.version, LATEST_PRIVATEBIN_VERSION);
    assert_eq!(privatebin.instance.https, false);
    assert_eq!(privatebin.instance.https_redirect, false);
    assert_eq!(privatebin.instance.attachments, true);
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
            topic: topic,
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
        let error_string   = error.unwrap_or(String::new());
        let success_string = success.unwrap_or(String::new());
        StatusPage {
            title: String::from(TITLE),
            topic: topic,
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
            topic: topic,
            tables: tables
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct HtmlTable {
    pub title: String,
    pub header: [String; 7],
    pub body: Vec<[String; 7]>,
}

#[derive(Debug, FromForm)]
pub struct AddForm {
    pub url: String
}
