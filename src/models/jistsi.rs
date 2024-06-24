use super::*;
use http_body_util::BodyExt; // BodyExt provides the Iterator trait
use hyper::body::{Buf, Bytes}; // Body provides the size_hint() trait, Buf provides the reader() trait
use hyper::{Method, StatusCode};
use regex::Regex;
use std::sync::OnceLock;

static VERSION_EXP: OnceLock<Regex> = OnceLock::new();
pub struct Jitsi {
    pub instance: InstanceNew,
    pub scans: Vec<ScanNew>,
}

impl Jitsi {
    pub async fn new(url: String) -> Result<Jitsi, String> {
        let (https, https_redirect, check_url) = Instance::check_http(&url).await?;

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
                    version = matches[1].into();
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
    let jitsi = Jitsi::new(test_url).await.unwrap();
    assert_eq!(jitsi.instance.url, url);
    assert!(jitsi.instance.https);
    assert!(jitsi.instance.https_redirect);
    assert_eq!(jitsi.instance.country_id, "CH");
    assert_eq!(jitsi.instance.variant, InstanceVariant::Jitsi);
}

#[tokio::test]
async fn test_non_jitsi() {
    let jitsi: Result<Jitsi, String> = Jitsi::new("https://directory.rs".into()).await;
    assert!(jitsi.is_err());
    let jitsi: Result<Jitsi, String> = Jitsi::new("https://privatebin.net".into()).await;
    assert!(jitsi.is_err());
}
