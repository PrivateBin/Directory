use super::{cron, cron_full, rocket, DirectoryDbConn};
use diesel::prelude::*;
use rocket::http::ContentType;
use rocket::http::Status;
use rocket::local::Client;
use std::fmt::Write;
use std::time::SystemTime;

#[test]
fn index() {
    let client = Client::new(rocket()).expect("valid rocket instance");
    let mut response = client.get("/").dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response
        .body_string()
        .map_or(false, |s| s.contains(&"Welcome!")));
}

#[test]
fn about() {
    let client = Client::new(rocket()).expect("valid rocket instance");
    let mut response = client.get("/about").dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response
        .body_string()
        .map_or(false, |s| s.contains(&"About")));
}

#[test]
fn add_get() {
    let client = Client::new(rocket()).expect("valid rocket instance");
    let mut response = client.get("/add").dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response
        .body_string()
        .map_or(false, |s| s.contains(&"Add instance")));
}

#[test]
fn add_post_error() {
    let client = Client::new(rocket()).expect("valid rocket instance");
    let mut response = client
        .post("/add")
        .body("url=example.com")
        .header(ContentType::Form)
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response
        .body_string()
        .map_or(false, |s| s.contains(&"Not a valid URL: example.com")));
}

#[test]
// incorporate add POST success test, as update depends on it running first
fn add_and_update() {
    use super::schema::checks::dsl::*;

    let rocket = rocket();
    let conn = DirectoryDbConn::get_one(&rocket).expect("database connection");
    let client = Client::new(rocket).expect("valid rocket instance");
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // insert an instance (tests run in parallel, so add_post_success() may not be ready)
    let mut add_response = client
        .post("/add")
        .body("url=https://privatebin.net")
        .header(ContentType::Form)
        .dispatch();
    assert_eq!(add_response.status(), Status::Ok);
    assert!(add_response
        .body_string()
        .map_or(false, |s| s.contains(&"Successfully added URL: ")));

    // insert checks
    let mut query = "INSERT INTO checks (updated, up, instance_id) VALUES (".to_string();
    let mut instance_checks = vec![];
    for interval in 0..(super::CHECKS_TO_STORE + 1) {
        instance_checks.push(format!(
            "datetime({}, 'unixepoch'), 1, 1",
            now - (interval * super::CRON_INTERVAL)
        ));
    }
    write!(&mut query, "{})", instance_checks.join("), (")).unwrap();
    conn.execute(&query)
        .expect("inserting test checks for instance ID 1");
    let oldest_update = now - (super::CHECKS_TO_STORE * super::CRON_INTERVAL);
    let oldest_check: Vec<i32> = checks
        .select(instance_id)
        .filter(updated.eq(diesel::dsl::sql(&format!(
            "datetime({}, 'unixepoch')",
            oldest_update
        ))))
        .load(&*conn)
        .expect("selecting oldest check");
    assert_eq!(vec![1], oldest_check);

    cron(DirectoryDbConn::get_one(&super::rocket()).expect("database connection"));
    let oldest_check: Vec<i32> = checks
        .select(instance_id)
        .filter(updated.eq(diesel::dsl::sql(&format!("{}", oldest_update))))
        .load(&*conn)
        .expect("selecting oldest check, now deleted");
    let empty: Vec<i32> = vec![]; // need to do this, so Rust can infer the type of the empty vector
    assert_eq!(empty, oldest_check);

    // prevent the same instance getting inserted again with a different query
    let mut add_response = client
        .post("/add")
        .body("url=https://privatebin.net/?foo")
        .header(ContentType::Form)
        .dispatch();
    assert_eq!(add_response.status(), Status::Ok);
    assert!(add_response
        .body_string()
        .map_or(false, |s| s.contains(&"Error adding URL ")));

    // prevent the same instance getting inserted again with a different hash
    let mut add_response = client
        .post("/add")
        .body("url=https://privatebin.net/#foo")
        .header(ContentType::Form)
        .dispatch();
    assert_eq!(add_response.status(), Status::Ok);
    assert!(add_response
        .body_string()
        .map_or(false, |s| s.contains(&"Error adding URL ")));

    // prevent the same instance getting inserted again with a different protocol
    let mut add_response = client
        .post("/add")
        .body("url=http://privatebin.net/")
        .header(ContentType::Form)
        .dispatch();
    assert_eq!(add_response.status(), Status::Ok);
    assert!(add_response
        .body_string()
        .map_or(false, |s| s.contains(&"Error adding URL ")));

    // prevent the same instance getting inserted again with a different path
    let mut add_response = client
        .post("/add")
        .body("url=https://privatebin.net//")
        .header(ContentType::Form)
        .dispatch();
    assert_eq!(add_response.status(), Status::Ok);
    assert!(add_response
        .body_string()
        .map_or(false, |s| s.contains(&"Error adding URL ")));
}

#[test]
// incorporate add POST success test, as the delete requires it to exist first
fn add_and_delete() {
    use super::schema::checks::dsl::*;
    use super::schema::instances;

    let rocket = rocket();
    let conn = DirectoryDbConn::get_one(&rocket).expect("database connection");
    let client = Client::new(rocket).expect("valid rocket instance");
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // insert an instance (tests run in parallel, so add_post_success() may not be ready)
    let mut add_response = client
        .post("/add")
        .body("url=http://zerobin-legacy.dssr.ch")
        .header(ContentType::Form)
        .dispatch();
    assert_eq!(add_response.status(), Status::Ok);
    assert!(add_response
        .body_string()
        .map_or(false, |s| s.contains(&"Successfully added URL: ")));

    // insert checks
    let mut query = "INSERT INTO checks (updated, up, instance_id) VALUES (".to_string();
    let mut instance_checks = vec![];
    for interval in 0..super::MAX_FAILURES {
        instance_checks.push(format!(
            "datetime({}, 'unixepoch'), 0, 1",
            now - (interval * super::CRON_INTERVAL)
        ));
    }
    write!(&mut query, "{})", instance_checks.join("), (")).unwrap();
    conn.execute(&query)
        .expect("inserting test checks for instance ID 1");

    cron_full(DirectoryDbConn::get_one(&super::rocket()).expect("database connection"));
    let deleted_check: Vec<i32> = checks
        .select(instance_id)
        .filter(instance_id.eq(1))
        .load(&*conn)
        .expect("selecting check for instance 1, now deleted");
    let empty: Vec<i32> = vec![]; // need to do this, so Rust can infer the type of the empty vector
    assert_eq!(empty, deleted_check);
    let deleted_instance: Vec<i32> = instances::table
        .select(instances::id)
        .filter(instances::id.eq(1))
        .load(&*conn)
        .expect("selecting instance 1, now deleted");
    assert_eq!(empty, deleted_instance);
}
