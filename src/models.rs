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

const TITLE: &str = "Instance Directory";
const LATEST_PRIVATEBIN_VERSION: &str = "1.3.4";

pub struct Instance {
    pub url: String,
    pub version: String,
    pub https_redirect: bool,
    pub country_id: [u8; 2],
}

impl Instance {
    pub fn new(url: String, version: String, https_redirect: bool, country_code: &str) -> Instance {
        let mut country_chars = country_code.chars();
        Instance {
            url: url,
            version: version,
            https_redirect: https_redirect,
            country_id: [
                country_chars.next().unwrap() as u8,
                country_chars.next().unwrap() as u8
            ],
        }
        // to convert u8 to char: 65u8 as char -> A
    }
}

pub struct PrivateBin {
    pub instance: Instance,
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
        let mut client = Client::with_connector(
            HttpsConnector::new(
                hyper_sync_rustls::TlsClient::new()
            )
        );
        client.set_redirect_policy(RedirectPolicy::FollowNone);

        // check for HTTPS redirect
        let mut https_redirect = false;
        let mut http_url = url.clone();
        let mut check_url = url;
        if check_url.starts_with("https://") {
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
                return Err(format!("Web server on URL {} is not responding.", check_url).to_string())
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

            let reader = maxminddb::Reader::open_readfile(env!("GEOIP_MMDB"));
            if reader.is_err() {
                return Err(
                    format!(
                        "Error opening geo IP database {} (defined in environment variable GEOIP_MMDB).",
                        env!("GEOIP_MMDB")
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

        let version_regexen = [
            Regex::new(r"js/privatebin.js\?(\d+\.\d+\.*\d*)").unwrap(),
            Regex::new(r"js/zerobin.js\?Alpha%20(\d+\.\d+\.*\d*)").unwrap()
        ];
        let buffer = BufReader::new(res);
        for line in buffer.lines() {
            let line_str = line.unwrap();
            for version_regex in version_regexen.iter() {
                let matches = version_regex.captures(&line_str);
                if matches.is_some() {
                    return Ok(
                        PrivateBin {
                            instance: Instance::new(
                                check_url,
                                matches.unwrap()[1].to_string(),
                                https_redirect,
                                &country_code,
                            ),
                        }
                    )
                }
            }
        }
        return Err(format!("The URL {} doesn't seem to be a PrivateBin instance.", check_url).to_string())
    }

    fn get_user_agent() -> UserAgent {
        return UserAgent(
            format!(
                "PrivateBinDirectoryBot/{} (+https://privatebin.info/directory/)",
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
    assert_eq!(privatebin.instance.https_redirect, true);
    assert_eq!(privatebin.instance.country_id, ['C' as u8, 'H' as u8]);
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
    assert_eq!(privatebin.instance.country_id, ['I' as u8, 'E' as u8]);
}

#[test]
fn test_privatebin_http() {
    let url = String::from("http://zerobin-test.dssr.ch/");
    let privatebin = PrivateBin::new(url.clone()).unwrap();
    assert_eq!(privatebin.instance.url, url);
    assert_eq!(privatebin.instance.version, LATEST_PRIVATEBIN_VERSION);
    assert_eq!(privatebin.instance.https_redirect, false);
    assert_eq!(privatebin.instance.country_id, ['C' as u8, 'H' as u8]);
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
    pub fn new(topic: String, error: String, success: String) -> StatusPage {
        StatusPage {
            title: String::from(TITLE),
            topic: topic,
            error: error,
            success: success,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct TablePage {
    pub title: String,
    pub topic: String,
    pub table: Table,
}

impl TablePage {
    pub fn new(topic: String, table: Table) -> TablePage {
        TablePage {
            title: String::from(TITLE),
            topic: topic,
            table: table
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Table {
    pub title: String,
    pub header: [String; 3],
    pub body: Vec<[String; 3]>,
}

#[derive(Debug, FromForm)]
pub struct AddForm {
    pub url: String
}
