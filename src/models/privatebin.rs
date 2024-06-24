use super::*;
use super::super::functions::strip_url;
use http_body_util::BodyExt; // BodyExt provides the Iterator trait
use hyper::body::{Buf, Bytes}; // Body provides the size_hint() trait, Buf provides the reader() trait
use hyper::header::CONTENT_SECURITY_POLICY;
use hyper::{Method, StatusCode};
use regex::Regex;
use std::sync::OnceLock;

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
static TEMPLATE_EXP: OnceLock<Regex> = OnceLock::new();
static VERSION_EXP: OnceLock<Regex> = OnceLock::new();

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
    assert_eq!(privatebin.instance.variant, InstanceVariant::PrivateBin);
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
    assert_eq!(privatebin.instance.variant, InstanceVariant::PrivateBin);
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
