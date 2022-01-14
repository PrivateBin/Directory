use super::{
    check_full, check_up, get_epoch, rocket, CHECKS_TO_STORE, CRON_INTERVAL, MAX_FAILURES,
};
use diesel::prelude::*;
use rocket::http::ContentType;
use rocket::http::Status;
use rocket::local::blocking::Client;
use rocket_sync_db_pools::Config;
use std::fmt::Write;

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

#[tokio::test]
// incorporate add POST success test, as update depends on it running first
async fn add_update_and_delete() {
    use super::schema::checks::dsl::*;
    use super::schema::instances;

    let built_rocket = rocket();
    let directory_config =
        Config::from("directory", &built_rocket).expect("configuration of directory database");
    let conn = SqliteConnection::establish(&directory_config.url)
        .expect("connection to directory database");
    let client = Client::untracked(built_rocket).expect("valid rocket instance");
    let empty: Vec<i32> = vec![]; // needs an explicit type, as it can't be inferred from an immutable, empty vector
    let now = get_epoch();

    // insert an instance
    let add_response = client
        .post("/add")
        .body("url=https://privatebin.net")
        .header(ContentType::Form)
        .dispatch();
    assert_eq!(add_response.status(), Status::Ok);
    assert!(add_response
        .into_string()
        .map_or(false, |s| s.contains(&"Successfully added URL: ")));

    // insert checks
    let mut query = "INSERT INTO checks (updated, up, instance_id) VALUES (".to_string();
    let mut instance_checks = vec![];
    for interval in 0..(CHECKS_TO_STORE + 1) {
        let interval_update = now - (interval * CRON_INTERVAL);
        instance_checks.push(format!("datetime({interval_update}, 'unixepoch'), 1, 1"));
    }
    let _ = write!(&mut query, "{})", instance_checks.join("), ("));
    conn.execute(&query)
        .expect("inserting test checks for instance ID 1");
    let oldest_update = now - (CHECKS_TO_STORE * CRON_INTERVAL);
    let oldest_check: Vec<i32> = checks
        .select(instance_id)
        .filter(updated.eq(diesel::dsl::sql(&format!(
            "datetime({oldest_update}, 'unixepoch')"
        ))))
        .load(&conn)
        .expect("selecting oldest check");
    assert_eq!(vec![1], oldest_check);

    check_up(rocket()).await;
    let oldest_check: Vec<i32> = checks
        .select(instance_id)
        .filter(updated.eq(diesel::dsl::sql(&format!("{oldest_update}"))))
        .load(&conn)
        .expect("selecting oldest check, now deleted");
    assert_eq!(empty, oldest_check);

    // insert another instance, subsequently to be deleted
    let add_response = client
        .post("/add")
        .body("url=http://zerobin-legacy.dssr.ch")
        .header(ContentType::Form)
        .dispatch();
    assert_eq!(add_response.status(), Status::Ok);
    assert!(add_response
        .into_string()
        .map_or(false, |s| s.contains(&"Successfully added URL: ")));

    // insert checks
    let mut query = "INSERT INTO checks (updated, up, instance_id) VALUES (".to_string();
    let mut instance_checks = vec![];
    for interval in 0..MAX_FAILURES {
        let interval_update = now - (interval * CRON_INTERVAL);
        instance_checks.push(format!("datetime({interval_update}, 'unixepoch'), 0, 2"));
    }
    let _ = write!(&mut query, "{})", instance_checks.join("), ("));
    conn.execute(&query)
        .expect("inserting test checks for instance ID 2");

    check_full(rocket()).await;
    let deleted_check: Vec<i32> = checks
        .select(instance_id)
        .filter(instance_id.eq(2))
        .load(&conn)
        .expect("selecting check for instance 2, now deleted");
    assert_eq!(empty, deleted_check);
    let deleted_instance: Vec<i32> = instances::table
        .select(instances::id)
        .filter(instances::id.eq(2))
        .load(&conn)
        .expect("selecting instance 2, now deleted");
    assert_eq!(empty, deleted_instance);

    // check immediate removal of sites that are no longer PrivateBin instances
    let query = "UPDATE instances SET url = 'https://privatebin.info' WHERE id = 1".to_string();
    conn.execute(&query)
        .expect("manipulating instance ID 1 to point to a non-PrivateBin URL");
    check_full(rocket()).await;
    let deleted_check: Vec<i32> = checks
        .select(instance_id)
        .filter(instance_id.eq(1))
        .load(&conn)
        .expect("selecting check for instance 1, now deleted");
    assert_eq!(empty, deleted_check);
    let deleted_instance: Vec<i32> = instances::table
        .select(instances::id)
        .filter(instances::id.eq(1))
        .load(&conn)
        .expect("selecting instance 1, now deleted");
    assert_eq!(empty, deleted_instance);
}
