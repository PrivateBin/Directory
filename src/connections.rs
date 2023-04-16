use hyper::client::connect::HttpConnector;
use hyper::header::{HeaderValue, CONNECTION, USER_AGENT};
use hyper::{Body, Client, Method, Request, Response, Uri};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use url::{Position, Url};

lazy_static! {
    static ref HTTP_CLIENT: Arc<Client<HttpsConnector<HttpConnector>, Body>> = Arc::new({
        let https_connector = HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .build();
        Client::builder().build(https_connector)
    });
    static ref USER_AGENT_STRING: String = format!(
        "PrivateBinDirectoryBot/{} (+https://privatebin.info/directory/about)",
        env!("CARGO_PKG_VERSION")
    );
}

// cache frequently used header values
pub static CLOSE: HeaderValue = HeaderValue::from_static("close");
pub static KEEPALIVE: HeaderValue = HeaderValue::from_static("keep-alive");

pub async fn request(
    url: &str,
    method: Method,
    connection: &HeaderValue,
    body: Body,
) -> Result<Response<Body>, String> {
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
        .header(USER_AGENT, HeaderValue::from_static(&USER_AGENT_STRING))
        .body(body)
        .expect("request");
    let result = match timeout(
        Duration::from_secs(15),
        HTTP_CLIENT.to_owned().request(request),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            return Err(format!(
                "Web server on URL {url} is not responding within 15s."
            ))
        }
    };
    match result {
        Ok(response) => Ok(response),
        Err(_) => Err(format!("Web server on URL {url} is not responding.")),
    }
}

pub async fn request_get(url: &str) -> Result<Response<Body>, String> {
    request(url, Method::GET, &KEEPALIVE, Body::empty()).await
}

pub async fn request_head(url: &str) -> Result<Response<Body>, String> {
    request(url, Method::HEAD, &KEEPALIVE, Body::empty()).await
}
