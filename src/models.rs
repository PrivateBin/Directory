extern crate hyper;
use hyper::{Body, Client};
use hyper::body::HttpBody;
use hyper_tls::HttpsConnector;
use regex::Regex;
use serde::Serialize;

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
    pub async fn new(url: String) -> Result<PrivateBin, String> {
        let validation = Self::validate(&url).await;
        if validation.is_ok() {
            let (country_code, version) = validation.unwrap();
            return Ok(
                PrivateBin {
                    instance: Instance::new(url, version, country_code),
                }
            )
        }
        Err(validation.unwrap().0.to_string())
    }

    async fn validate(url: &String) -> Result<(&'static str, String), Box<dyn std::error::Error + Send + Sync>> {
        let https = HttpsConnector::new();
        let client = Client::builder().build::<_, Body>(https);
        let uri = url.parse()?;
        let mut res = client.get(uri).await?;
        if res.status() != 200 {
            return Err(format!("Web server responded with status code {}.", res.status()).into())
        }

        let privatebin_version_re = Regex::new(r"js/privatebin.js\?(\d+\.\d+\.*\d*)").unwrap();
        let zerobin_version_re = Regex::new(r"js/zerobin.js\?Alpha%20(\d+\.\d+\.*\d*)").unwrap();
        while let Some(chunk) = res.body_mut().data().await {
            let a_chunk = &chunk?;
            let chunk_str = std::str::from_utf8(&a_chunk).unwrap();
            let privatebin_matches = privatebin_version_re.captures(chunk_str);
            if privatebin_matches.is_some() {
                return Ok(("AQ", privatebin_matches.unwrap()[1].to_string()))
            }
            let zerobin_matches = zerobin_version_re.captures(chunk_str);
            if zerobin_matches.is_some() {
                return Ok(("AQ", zerobin_matches.unwrap()[1].to_string()))
            }
        }
        Err(format!("The URL {} doesn't seem to be a PrivateBin instance.", url).into())
    }
}

#[tokio::test]
async fn test_privatebin() {
    let url = String::from("https://privatebin.net");
    let privatebin = PrivateBin::new(url.clone()).await.unwrap();
    assert_eq!(privatebin.instance.url, url);
    assert_eq!(privatebin.instance.version, LATEST_PRIVATEBIN_VERSION);
    assert_eq!(privatebin.instance.country_id, ['A' as u8, 'Q' as u8]);
}

#[tokio::test]
async fn test_zerobin() {
    let url = String::from("https://sebsauvage.net/paste/");
    let privatebin = PrivateBin::new(url.clone()).await.unwrap();
    assert_eq!(privatebin.instance.url, url);
    assert_eq!(privatebin.instance.version, "0.19.2");
    assert_eq!(privatebin.instance.country_id, ['A' as u8, 'Q' as u8]);
}

#[tokio::test]
async fn test_privatebin_http() {
    let url = String::from("http://zerobin-test.dssr.ch/");
    let privatebin = PrivateBin::new(url.clone()).await.unwrap();
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
