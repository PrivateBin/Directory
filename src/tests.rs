use super::rocket;
use rocket::local::Client;
use rocket::http::ContentType;
use rocket::http::Status;

#[test]
fn index() {
    let client = Client::new(rocket()).expect("valid rocket instance");
    let mut response = client.get("/").dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response.body_string().map_or(false, |s| s.contains(&"Welcome!")));
}

#[test]
fn add_get() {
    let client = Client::new(rocket()).expect("valid rocket instance");
    let mut response = client.get("/add").dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response.body_string().map_or(false, |s| s.contains(&"Add instance")));
}

#[test]
fn add_post_success() {
    let client = Client::new(rocket()).expect("valid rocket instance");
    let mut response = client.post("/add")
        .body("url=http://example.com")
        .header(ContentType::Form)
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response.body_string().map_or(false, |s| s.contains(&"Successfully added URL: ")));
}

#[test]
fn add_post_error() {
    let client = Client::new(rocket()).expect("valid rocket instance");
    let mut response = client.post("/add")
        .body("url=example.com")
        .header(ContentType::Form)
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response.body_string().map_or(false, |s| s.contains(&"Not a valid URL: example.com")));
}