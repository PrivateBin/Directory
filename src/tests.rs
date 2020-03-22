use super::rocket;
use rocket::local::Client;
use rocket::http::Status;

#[test]
fn hello_world() {
    let client = Client::new(rocket()).expect("valid rocket instance");
    let mut response = client.get("/").dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response.body_string().map_or(false, |s| s.contains(&"Hello, world!")));
}