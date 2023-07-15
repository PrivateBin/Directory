use super::rocket;
use rocket::http::ContentType;
use rocket::http::Status;
use rocket::local::blocking::Client;
use std::thread::sleep;
use std::time::Duration;

#[test]
fn index() {
    let client = Client::untracked(rocket()).expect("valid rocket instance");
    let response = client.get("/").dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response
        .into_string()
        .map_or(false, |s| s.contains("Welcome!")));
}

#[test]
fn about() {
    let client = Client::untracked(rocket()).expect("valid rocket instance");
    let response = client.get("/about").dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response
        .into_string()
        .map_or(false, |s| s.contains("About")));
}

#[test]
fn add_get() {
    let client = Client::untracked(rocket()).expect("valid rocket instance");
    let response = client.get("/add").dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response
        .into_string()
        .map_or(false, |s| s.contains("Add instance")));
}

#[test]
fn add_post_error() {
    let client = Client::untracked(rocket()).expect("valid rocket instance");
    let response = client
        .post("/add")
        .body("url=privatebin.info")
        .header(ContentType::Form)
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response
        .into_string()
        .map_or(false, |s| s.contains("Not a valid URL: privatebin.info")));

    let response = client
        .post("/add")
        .body("url=privatebin.info")
        .header(ContentType::Form)
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response.into_string().map_or(false, |s| s.contains(
        "Error adding URL privatebin.info, due to a failed scan within the last 5 minutes."
    )));

    sleep(Duration::from_secs(2));
    let response = client
        .post("/add")
        .body("url=privatebin.info")
        .header(ContentType::Form)
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response
        .into_string()
        .map_or(false, |s| s.contains("Not a valid URL: privatebin.info")));
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
        .contains("Successfully added URL: https:&#x2F;&#x2F;privatebin.net")));
}

#[test]
fn check_get() {
    let client = Client::untracked(rocket()).expect("valid rocket instance");
    let response = client.get("/check").dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response
        .into_string()
        .map_or(false, |s| s.contains("Check instance")));
}

#[test]
fn check_post_error() {
    let client = Client::untracked(rocket()).expect("valid rocket instance");
    let response = client
        .post("/check")
        .body("url=privatebin.info")
        .header(ContentType::Form)
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response
        .into_string()
        .map_or(false, |s| s.contains("Not a valid URL: privatebin.info")));

    let response = client
        .post("/check")
        .body("url=privatebin.info")
        .header(ContentType::Form)
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response.into_string().map_or(false, |s| s.contains(
        "Error scanning URL privatebin.info, due to a failed scan within the last 5 minutes."
    )));

    sleep(Duration::from_secs(2));
    let response = client
        .post("/check")
        .body("url=privatebin.info")
        .header(ContentType::Form)
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response
        .into_string()
        .map_or(false, |s| s.contains("Not a valid URL: privatebin.info")));
}

#[test]
fn check_post_success() {
    let client = Client::untracked(rocket()).expect("valid rocket instance");
    let response = client
        .post("/check")
        .body("url=https://privatebin.net")
        .header(ContentType::Form)
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response.into_string().map_or(false, |s| s
        .contains("Results of checking https:&#x2F;&#x2F;privatebin.net")));
}

#[test]
fn forward_me() {
    let client = Client::untracked(rocket()).expect("valid rocket instance");
    let response = client.get("/forward-me").dispatch();
    assert_eq!(response.status(), Status::SeeOther);
    assert!(response.headers().contains("Location"));
}
