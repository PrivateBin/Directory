use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::header::{HeaderValue, CONNECTION, USER_AGENT};
use hyper::Uri;
use hyper::{Method, Request, Response};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::time::timeout;
use url::{Position, Url};

static HTTP_CLIENT: OnceLock<Client<HttpsConnector<HttpConnector>, Full<Bytes>>> = OnceLock::new();

// cache frequently used header values
pub static CLOSE: HeaderValue = HeaderValue::from_static("close");
pub static KEEPALIVE: HeaderValue = HeaderValue::from_static("keep-alive");
static USER_AGENT_STRING: OnceLock<String> = OnceLock::new();
static USER_AGENT_VALUE: OnceLock<HeaderValue> = OnceLock::new();

pub async fn request(
    url: &str,
    method: Method,
    connection: &HeaderValue,
    body: Bytes,
) -> Result<Response<Incoming>, String> {
    // parse URL to convert IDN into punycode
    let parsed_url = match Url::parse(url) {
        Ok(parsed_url) => parsed_url,
        Err(_) => return Err(format!("Host or domain of URL {url} is not supported.")),
    };
    let parsed_host = match parsed_url.host_str() {
        Some(host) => host,
        None => return Err(format!("Unable to parse host from URL {url}.")),
    };
    let authority = match parsed_url.port() {
        Some(port) => format!("{}:{}", parsed_host, port),
        None => parsed_host.to_owned(),
    };
    let uri = match Uri::builder()
        .scheme(parsed_url.scheme())
        .authority(authority)
        .path_and_query(&parsed_url[Position::BeforePath..])
        .build()
    {
        Ok(uri) => uri,
        Err(_) => return Err(format!("Host or domain of URL {url} is not supported.")),
    };

    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header(CONNECTION, connection)
        .header(
            USER_AGENT,
            USER_AGENT_VALUE.get_or_init(|| {
                HeaderValue::from_static(USER_AGENT_STRING.get_or_init(|| {
                    format!(
                        "PrivateBinDirectoryBot/{} (+https://privatebin.info/directory/about)",
                        env!("CARGO_PKG_VERSION")
                    )
                }))
            }),
        )
        .body(Full::from(body))
        .unwrap();
    match timeout(
        Duration::from_secs(15),
        HTTP_CLIENT
            .get_or_init(init_connection)
            .clone()
            .request(request),
    )
    .await
    {
        Ok(result) => match result {
            Ok(result) => Ok(result),
            Err(_) => Err(format!("Web server on URL {url} is not responding.")),
        },
        Err(_) => Err(format!(
            "Web server on URL {url} is not responding within 15s."
        )),
    }
}

pub async fn request_get(url: &str) -> Result<Response<Incoming>, String> {
    request(url, Method::GET, &KEEPALIVE, Bytes::new()).await
}

pub async fn request_head(url: &str) -> Result<Response<Incoming>, String> {
    request(url, Method::HEAD, &KEEPALIVE, Bytes::new()).await
}

pub fn init_connection() -> Client<HttpsConnector<HttpConnector>, Full<Bytes>> {
    let https_connector = HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .build();
    Client::builder(TokioExecutor::new()).build(https_connector)
}
