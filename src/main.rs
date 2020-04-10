#![forbid(unsafe_code)]
#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use] extern crate diesel;
#[macro_use] extern crate diesel_migrations;
#[macro_use] extern crate rocket;
#[macro_use] extern crate rocket_contrib;
use diesel::prelude::*;
use diesel::dsl::sql_query;
use rocket::fairing::AdHoc;
use rocket::response::Redirect;
use rocket::request::Form;
use rocket::{Rocket, State};
//use rocket_contrib::databases::diesel; not working with current diesel
use rocket_contrib::serve::StaticFiles;
use rocket_contrib::templates::Template;
use std::fmt::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::SystemTime;

pub mod schema;
use schema::instances::dsl::*;
use schema::checks::dsl::{checks, updated};
pub mod models;
use models::*;
#[cfg(test)] mod tests;

const CRON_INTERVAL: u64 = 900; // 15 minutes
const CHECKS_TO_STORE: u64 = 100; // amount of checks to keep
const MAX_FAILURES: u64 = 90; // remove instances that failed this many times

#[get("/")]
fn index(conn: DirectoryDbConn, cache: State<InstancesCache>) -> Template {

    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
    if now >= cache.timeout.load(Ordering::Relaxed) {
        let instances_result = sql_query(
                "SELECT instances.id, url, version, https, https_redirect, attachments, \
                country_id, (100 * SUM(checks.up) / COUNT(checks.up)) AS uptime \
                FROM instances JOIN checks ON instances.id = checks.instance_id \
                GROUP BY instances.id \
                ORDER BY version DESC, https DESC, https_redirect DESC, \
                attachments DESC, uptime DESC, url ASC \
                LIMIT 100"
            )
            .load::<Instance>(&*conn);
        match instances_result {
            // flush cache
            Ok(instances_live) => {
                cache.timeout.store(now + CRON_INTERVAL, Ordering::Relaxed);
                let mut instances_cache = cache.instances.write().unwrap();
                *instances_cache = instances_live;
            },
            // database might be write-locked, try it again in a minute
            Err(_) => cache.timeout.store(now + 60, Ordering::Relaxed)
        }
    }

    let header = [
        String::from("Address"),
        String::from("Version"),
        String::from("HTTPS"),
        String::from("HTTPS enforced"),
        String::from("File upload"),
        String::from("Uptime"),
        String::from("Country")
    ];
    let mut tables = vec![];
    let mut body = vec![];
    let (mut major, mut minor) = (0, 0);
    for instance in &*cache.instances.read().unwrap() {
        // parse the major and minor bits of the version
        let mmp: Vec<u16> = instance.version.split('.')
            .filter_map(|s| s.parse::<u16>().ok())
            .collect();
        if mmp.is_empty() {
            continue;
        }
        let (instance_major, instance_minor) = (mmp[0] as u16, mmp[1] as u16);

        if minor == 0 {
            // this is the first instance in the list
            major = instance_major;
            minor = instance_minor;
        } else if major != instance_major || minor != instance_minor {
            // close table
            tables.push(
                HtmlTable {
                    title: format!("Version {}.{}", major, minor).to_string(),
                    header: header.clone(),
                    body: body.clone()
                }
            );
            // start a new one
            major = instance_major;
            minor = instance_minor;
            body.clear();
        }

        // format current instance for table display
        body.push([
            format!("opacity{}", instance.uptime / 25),
            instance.url.clone(),
            instance.version.clone(),
            Instance::format(instance.https),
            Instance::format(instance.https_redirect),
            Instance::format(instance.attachments),
            format!("{}%", instance.uptime),
            Instance::format_country(instance.country_id.clone())
        ]);
    }
    tables.push(
        HtmlTable {
            title: format!("Version {}.{}", major, minor),
            header,
            body
        }
    );

    let page = TablePage::new(
        String::from("Welcome!"),
        tables
    );
    Template::render("list", &page)
}

const ADD_TITLE: &str = "Add instance";

#[get("/about")]
fn about() -> Template {
    let page = StatusPage::new(
        format!("About the {}", TITLE), None, None
    );
    Template::render("about", &page)
}

#[get("/add")]
fn add() -> Template {
    let page = StatusPage::new(
        String::from(ADD_TITLE), None, None
    );
    Template::render("add", &page)
}

#[post("/add", data = "<form>")]
fn save(conn: DirectoryDbConn, form: Form<AddForm>, cache: State<InstancesCache>) -> Template {
    let form = form.into_inner();
    let add_url = form.url.trim();

    let page: StatusPage;
    let privatebin = PrivateBin::new(add_url.to_string());
    match privatebin {
        Ok(privatebin) => {
            let db_result = diesel::insert_into(instances)
                .values(&privatebin.instance)
                .execute(&*conn);
            match db_result {
                Ok(_) => {
                    // need to store at least one check, or the JOIN in /index produces NULL
                    let instance: i32 = instances.select(id)
                        .filter(url.eq(privatebin.instance.url.clone()))
                        .limit(1)
                        .first(&*conn)
                        .expect("we just inserted the instance with no error, so selecting it is expected to work");
                    diesel::insert_into(checks)
                        .values(CheckNew::new(true, instance))
                        .execute(&*conn)
                        .expect("inserting first check on a newly created instance");

                    page = StatusPage::new(
                        String::from(ADD_TITLE),
                        None,
                        Some(format!("Successfully added URL: {}", privatebin.instance.url))
                    );
                    // flush cache
                    cache.timeout.store(0, Ordering::Relaxed);
                },
                Err(e) => page = StatusPage::new(
                    String::from(ADD_TITLE),
                    Some(format!("Error adding URL {}, due to: {:?}", add_url, e)),
                    None
                )
            }
        },
        Err(e) => page = StatusPage::new(
            String::from(ADD_TITLE),
            Some(e),
            None
        )
    }
    Template::render("add", &page)
}

#[get("/update/<key>")]
fn cron(key: String, conn: DirectoryDbConn, cache: State<InstancesCache>) -> String {
    if key != std::env::var("CRON_KEY").expect("environment variable CRON_KEY needs to be set") {
        return String::from("Wrong key, no update was triggered.\n");
    }

    let mut result = String::new();
    let mut instance_checks = vec![];
    for instance in &*cache.instances.read().unwrap() {
        // record instance being up
        instance_checks.push(
            CheckNew::new(instance.check_up(), instance.id)
        );
        writeln!(
            &mut result,
            "Instance {} checked",
            instance.url.clone()
        ).unwrap();
    }

    // store checks
    let check_insert_result = diesel::insert_into(checks)
        .values(&instance_checks)
        .execute(&*conn);
    match check_insert_result {
        Ok(_) => {
            result.push_str("stored uptime checks\n");

            // delete checks older then:
            // now - ((CHECKS_TO_STORE - 1) * CRON_INTERVAL)
            let cutoff = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs() - (
                    (CHECKS_TO_STORE - 1) * CRON_INTERVAL
                );
            let check_delete_result = diesel::delete(checks)
                .filter(
                    updated.lt(
                        diesel::dsl::sql(
                            &format!("datetime({}, 'unixepoch')", cutoff)
                        )
                    )
                )
                .execute(&*conn);
            match check_delete_result {
                Ok(_) => {
                    writeln!(
                        &mut result,
                        "cleaned up checks stored before {}",
                        cutoff
                    ).unwrap();
                },
                Err(e) => {
                    writeln!(
                        &mut result,
                        "failed to cleanup checks stored before {}, with error: {}",
                        cutoff, e
                    ).unwrap();
                }
            }
        },
        Err(e) => {
            writeln!(
                &mut result,
                "failed to store uptime checks with error: {}",
                e
            ).unwrap();
        }
    }

    result
}

#[get("/update/<key>/full")]
fn cron_full(key: String, conn: DirectoryDbConn, cache: State<InstancesCache>) -> String {
    if key != std::env::var("CRON_KEY").expect("environment variable CRON_KEY needs to be set") {
        return String::from("Wrong key, no update was triggered.\n");
    }

    let mut result = String::new();
    let mut instances_updated = false;
    let mut instance_checks = vec![];
    for instance in &*cache.instances.read().unwrap() {
        let privatebin = PrivateBin::new(instance.url.clone());
        match privatebin {
            Ok(privatebin) => {
                // record instance being up
                instance_checks.push(
                    CheckNew::new(true, instance.id)
                );

                // compare result with cache
                let instance_options = [
                    (
                        "version",
                        instance.version.clone(),
                        privatebin.instance.version.clone()
                    ),
                    (
                        "https",
                        format!("{:?}", instance.https.clone()),
                        format!("{:?}", privatebin.instance.https.clone())
                    ),
                    (
                        "https_redirect",
                        format!("{:?}", instance.https_redirect.clone()),
                        format!("{:?}", privatebin.instance.https_redirect.clone())
                    ),
                    (
                        "attachments",
                        format!("{:?}", instance.attachments.clone()),
                        format!("{:?}", privatebin.instance.attachments.clone())
                    ),
                    (
                        "country_id",
                        instance.country_id.clone(),
                        privatebin.instance.country_id.clone()
                    ),
                ];
                if  instance_options.iter().any(|x| x.1 != x.2) {
                    let db_result = diesel::update(
                            instances.filter(
                                id.eq(instance.id)
                            )
                        )
                        .set((
                            version.eq(privatebin.instance.version),
                            https.eq(privatebin.instance.https),
                            https_redirect.eq(privatebin.instance.https_redirect),
                            attachments.eq(privatebin.instance.attachments),
                            country_id.eq(privatebin.instance.country_id)
                        ))
                        .execute(&*conn);
                    match db_result {
                        Ok(_) => {
                            instances_updated = true;
                            writeln!(
                                &mut result,
                                "Instance {} checked and updated:",
                                instance.url.clone()
                            ).unwrap();
                            for (label, old, new) in instance_options.iter() {
                                if old != new {
                                    writeln!(
                                        &mut result,
                                        "    {} was {}, updated to {}",
                                        label, old, new
                                    ).unwrap();
                                }
                            }
                        },
                        Err(e) => {
                            writeln!(
                                &mut result,
                                "Instance {} failed to be updated with error: {:?}",
                                instance.url.clone(), e
                            ).unwrap();
                        }
                    }
                } else {
                    writeln!(
                        &mut result,
                        "Instance {} checked, no update required",
                        instance.url.clone()
                    ).unwrap();
                }
            },
            Err(e) => {
                instance_checks.push(
                    CheckNew::new(false, instance.id)
                );
                writeln!(
                    &mut result,
                    "Instance {} failed to be checked with error: {}",
                    instance.url.clone(), e
                ).unwrap();
            }
        }
    }

    // delete checks and instances that failed too many times
    match sql_query(
            &format!(
                "DELETE FROM instances \
                WHERE id in ( \
                    SELECT instance_id \
                    FROM checks \
                    WHERE up = 0 \
                    GROUP BY instance_id \
                    HAVING COUNT(up) >= {} \
                );",
                MAX_FAILURES
            )
        )
        .execute(&*conn)
    {
        Ok(_) => result.push_str("removed instances that failed too many times\n"),
        Err(e) => {
            writeln!(
                &mut result,
                "error removing instances failing too many times: {}",
                e
            ).unwrap();
        }
    }

    // if any entry had to be updated, invalidate the cache
    if instances_updated {
        cache.timeout.store(0, Ordering::Relaxed);
        result.push_str("cache flushed\n");
    }

    result
}

#[get("/favicon.ico")]
fn favicon() -> Redirect {
    Redirect::permanent("/img/favicon.ico")
}

#[database("directory")]
struct DirectoryDbConn(diesel::SqliteConnection);
embed_migrations!();

fn run_db_migrations(rocket: Rocket) -> Result<Rocket, Rocket> {
    let conn = DirectoryDbConn::get_one(&rocket).expect("database connection");
    match embedded_migrations::run(&*conn) {
        Ok(()) => Ok(rocket),
        Err(e) => {
            println!("Failed to run database migrations: {:?}", e);
            Err(rocket)
        }
    }
}

fn rocket() -> Rocket {
    rocket::ignite()
        .mount("/", routes![index, about, add, cron, cron_full, save, favicon])
        .mount("/img", StaticFiles::from("/img"))
        .mount("/css", StaticFiles::from("/css"))
        .attach(DirectoryDbConn::fairing())
        .attach(Template::fairing())
        .manage(InstancesCache {
            timeout: AtomicU64::new(0),
            instances: RwLock::new(vec![])
        })
}

fn main() {
    rocket()
        .attach(AdHoc::on_attach("Database Migrations", run_db_migrations))
        .launch();
}
