use super::rocket;
use rocket::http::ContentType;
use rocket::http::Status;
use rocket::local::blocking::Client;

#[test]
fn index() {
    let client = Client::untracked(rocket()).expect("valid rocket instance");
    let response = client.get("/").dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response
        .into_string()
        .map_or(false, |s| s.contains(&"Welcome!")));
}

#[test]
fn about() {
    let client = Client::untracked(rocket()).expect("valid rocket instance");
    let response = client.get("/about").dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response
        .into_string()
        .map_or(false, |s| s.contains(&"About")));
}

#[test]
fn add_get() {
    let client = Client::untracked(rocket()).expect("valid rocket instance");
    let response = client.get("/add").dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response
        .into_string()
        .map_or(false, |s| s.contains(&"Add instance")));
}

#[test]
fn add_post_error() {
    let client = Client::untracked(rocket()).expect("valid rocket instance");
    let response = client
        .post("/add")
        .body("url=example.com")
        .header(ContentType::Form)
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response
        .into_string()
        .map_or(false, |s| s.contains(&"Not a valid URL: example.com")));
}

#[test]
fn add_post_success() {
    let client = Client::untracked(rocket()).expect("valid rocket instance");
    let response = client
        .post("/add")
        .body("url=https://privatebin.net")
        .header(ContentType::Form)
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response.into_string().map_or(false, |s| s
        .contains(&"Successfully added URL: https:&#x2F;&#x2F;privatebin.net")));
}
