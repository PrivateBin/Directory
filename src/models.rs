extern crate hyper;
use hyper::{Body, Client};
use hyper::body::HttpBody;
use hyper_tls::HttpsConnector;
use regex::Regex;
use serde::Serialize;

const TITLE: &str = "Instance Directory";

//type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub struct Instance {
    pub url: String,
    pub country_id: [u8; 2],
}

impl Instance {
    pub fn new(url: String, country_code: &str) -> Instance {
        let mut country_chars = country_code.chars();
        Instance {
            url: url,
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
        let (is_valid, country_code, validation_status) = Self::validate(&url).await.unwrap();
        if is_valid {
            return Ok(
                PrivateBin {
                    instance: Instance::new(url, country_code),
                }
            )
        }
        Err(validation_status)
    }

    async fn validate(url: &String) -> Result<(bool, &'static str, String), Box<dyn std::error::Error + Send + Sync>> {
        let https = HttpsConnector::new();
        let client = Client::builder().build::<_, Body>(https);
        let uri = url.parse()?;
        let mut res = client.get(uri).await?;
        if res.status() != 200 {
            return Ok((false, "UN", format!("Web server responded with status code {}.", res.status())))
        }

        let version_re = Regex::new(r"js/privatebin.js\?(\d+\.\d+\.*\d*)").unwrap();
        while let Some(chunk) = res.body_mut().data().await {
            let a_chunk = &chunk?;
            let chunk_str = std::str::from_utf8(&a_chunk).unwrap();
            let matches = version_re.captures(chunk_str);
            if matches.is_some() {
                println!("Version: {}", &matches.unwrap()[1]);
                return Ok((true, "AQ", String::from("")))
            }
        }
        return Ok((false, "UN", format!("The URL {} doesn't seem to be a PrivateBin instance.", url)))
    }
}

#[tokio::test]
async fn test_privatebin() {
    let url = String::from("https://privatebin.net");
    let privatebin = PrivateBin::new(url.clone()).await.unwrap();
    assert_eq!(privatebin.instance.url, url);
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
