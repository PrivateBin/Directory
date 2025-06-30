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

/// # Errors
///
/// Will return `Err` if request to `url` fails for any reason.
///
/// # Panics
///
/// May panic in `Request::builder().[...].unwrap()`.
pub async fn request(
    url: &str,
    method: Method,
    connection: &HeaderValue,
    body: Bytes,
) -> Result<Response<Incoming>, String> {
    // parse URL to convert IDN into punycode
    let Ok(parsed_url) = Url::parse(url) else {
        return Err(format!("Host or domain of URL {url} is not supported."));
    };
    let Some(parsed_host) = parsed_url.host_str() else {
        return Err(format!("Unable to parse host from URL {url}."));
    };
    let authority = match parsed_url.port() {
        Some(port) => format!("{parsed_host}:{port}"),
        None => parsed_host.to_owned(),
    };
    let Ok(parsed_uri) = Uri::builder()
        .scheme(parsed_url.scheme())
        .authority(authority)
        .path_and_query(&parsed_url[Position::BeforePath..])
        .build()
    else {
        return Err(format!("Host or domain of URL {url} is not supported."));
    };

    let request = Request::builder()
        .method(method)
        .uri(parsed_uri)
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

/// # Errors
///
/// Will return `Err` if request to `url` fails for any reason.
pub async fn request_get(url: &str) -> Result<Response<Incoming>, String> {
    request(url, Method::GET, &KEEPALIVE, Bytes::new()).await
}

/// # Errors
///
/// Will return `Err` if request to `url` fails for any reason.
pub async fn request_head(url: &str) -> Result<Response<Incoming>, String> {
    request(url, Method::HEAD, &KEEPALIVE, Bytes::new()).await
}

/// # Errors
///
/// Will return `Err` if request to `url` fails for any reason.
pub async fn request_post(url: &str) -> Result<Response<Incoming>, String> {
    request(url, Method::POST, &KEEPALIVE, Bytes::new()).await
}

#[must_use]
pub fn init_connection() -> Client<HttpsConnector<HttpConnector>, Full<Bytes>> {
    let https_connector = HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .build();
    Client::builder(TokioExecutor::new()).build(https_connector)
}
