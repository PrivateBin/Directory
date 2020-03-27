extern crate hyper_sync_rustls;
use hyper::Client;
use hyper::header::Connection;
use hyper::net::HttpsConnector;
use hyper::status::StatusCode;
use regex::Regex;
use serde::Serialize;
use std::io::{BufReader, BufRead};

const TITLE: &str = "Instance Directory";
const LATEST_PRIVATEBIN_VERSION: &str = "1.3.4";

pub struct Instance {
    pub url: String,
    pub version: String,
    pub country_id: [u8; 2],
}

impl Instance {
    pub fn new(url: String, version: String, country_code: &str) -> Instance {
        let mut country_chars = country_code.chars();
        Instance {
            url: url,
            version: version,
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
        let client = Client::with_connector(
            HttpsConnector::new(
                hyper_sync_rustls::TlsClient::new()
            )
        );
        let result = client.get(&url)
            .header(Connection::close())
            .send();
        if result.is_err() {
            return Err(format!("Web server on URL {} is not responding.", url).to_string())
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
                            instance: Instance::new(url, matches.unwrap()[1].to_string(), "AQ"),
                        }
                    )
                }
            }
        }
        return Err(format!("The URL {} doesn't seem to be a PrivateBin instance.", url).to_string())
    }
}

#[test]
fn test_privatebin() {
    let url = String::from("https://privatebin.net");
    let privatebin = PrivateBin::new(url.clone()).unwrap();
    assert_eq!(privatebin.instance.url, url);
    assert_eq!(privatebin.instance.version, LATEST_PRIVATEBIN_VERSION);
    assert_eq!(privatebin.instance.country_id, ['A' as u8, 'Q' as u8]);
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
    assert_eq!(privatebin.instance.country_id, ['A' as u8, 'Q' as u8]);
}

#[test]
fn test_privatebin_http() {
    let url = String::from("http://zerobin-test.dssr.ch/");
    let privatebin = PrivateBin::new(url.clone()).unwrap();
    assert_eq!(privatebin.instance.url, url);
    assert_eq!(privatebin.instance.version, LATEST_PRIVATEBIN_VERSION);
    assert_eq!(privatebin.instance.country_id, ['A' as u8, 'Q' as u8]);
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
