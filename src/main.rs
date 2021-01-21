#![forbid(unsafe_code)]
#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use]
extern crate diesel;
#[macro_use]
extern crate diesel_migrations;
#[macro_use]
extern crate rocket;
#[macro_use]
extern crate rocket_contrib;
use diesel::dsl::sql_query;
use diesel::prelude::*;
use rocket::fairing::AdHoc;
use rocket::request::Form;
use rocket::response::Redirect;
use rocket::{Rocket, State};
//use rocket_contrib::databases::diesel; not working with current diesel
use rocket_contrib::serve::StaticFiles;
use rocket_contrib::templates::Template;
use std::fmt::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::thread;
use std::time::{Instant, SystemTime};

pub mod schema;
use schema::checks::dsl::{checks, updated};
use schema::instances::dsl::*;
use schema::scans::dsl::{percent, rating, scanner, scans};
pub mod models;
use models::*;
#[cfg(test)]
mod tests;

const CRON_INTERVAL: u64 = 900; // 15 minutes
const CHECKS_TO_STORE: u64 = 100; // amount of checks to keep
const MAX_FAILURES: u64 = 90; // remove instances that failed this many times

#[get("/")]
fn index(conn: DirectoryDbConn, cache: State<InstancesCache>) -> Template {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    if now >= cache.timeout.load(Ordering::Relaxed) {
        match get_instances().load::<Instance>(&*conn) {
            // flush cache
            Ok(instances_live) => {
                cache.timeout.store(now + CRON_INTERVAL, Ordering::Relaxed);
                let mut instances_cache = cache.instances.write().unwrap();
                *instances_cache = instances_live;
            }
            // database might be write-locked, try it again in a minute
            Err(_) => cache.timeout.store(now + 60, Ordering::Relaxed),
        }
    }

    let header = [
        String::from("Address"),
        String::from("Version"),
        String::from("HTTPS"),
        String::from("HTTPS enforced"),
        String::from("Observatory Rating"),
        String::from("File upload"),
        String::from("Uptime"),
        String::from("Country"),
    ];
    let mut tables = vec![];
    let mut body = vec![];
    let (mut major, mut minor) = (0, 0);
    for instance in &*cache.instances.read().unwrap() {
        // parse the major and minor bits of the version
        let mmp: Vec<u16> = instance
            .version
            .split('.')
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
            tables.push(HtmlTable {
                title: format!("Version {}.{}", major, minor).to_string(),
                header: header.clone(),
                body: body.clone(),
            });
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
            instance.rating_mozilla_observatory.clone(),
            Instance::format(instance.attachments),
            format!("{}%", instance.uptime),
            Instance::format_country(instance.country_id.clone()),
        ]);
    }
    tables.push(HtmlTable {
        title: format!("Version {}.{}", major, minor),
        header,
        body,
    });

    let page = TablePage::new(String::from("Welcome!"), tables);
    Template::render("list", &page)
}

const ADD_TITLE: &str = "Add instance";

#[get("/about")]
fn about() -> Template {
    let page = StatusPage::new(format!("About the {}", TITLE), None, None);
    Template::render("about", &page)
}

#[get("/add")]
fn add() -> Template {
    let page = StatusPage::new(String::from(ADD_TITLE), None, None);
    Template::render("add", &page)
}

#[post("/add", data = "<form>")]
fn save(conn: DirectoryDbConn, form: Form<AddForm>, cache: State<InstancesCache>) -> Template {
    let form = form.into_inner();
    let add_url = form.url.trim();

    let page: StatusPage;
    match PrivateBin::new(add_url.to_string()) {
        Ok(privatebin) => {
            match diesel::insert_into(instances)
                .values(&privatebin.instance)
                .execute(&*conn)
            {
                Ok(_) => {
                    // need to store at least one check and scan, or the JOIN in /index produces NULL
                    let instance: i32 = instances
                        .select(id)
                        .filter(url.eq(privatebin.instance.url.clone()))
                        .limit(1)
                        .first(&*conn)
                        .expect("selecting the just inserted the instance");
                    diesel::insert_into(checks)
                        .values(CheckNew::new(true, instance))
                        .execute(&*conn)
                        .expect("inserting first check on a newly created instance");
                    diesel::insert_into(scans)
                        .values(ScanNew::new("mozilla_observatory", "-", instance))
                        .execute(&*conn)
                        .expect("inserting first scan on a newly created instance");

                    page = StatusPage::new(
                        String::from(ADD_TITLE),
                        None,
                        Some(format!(
                            "Successfully added URL: {}",
                            privatebin.instance.url
                        )),
                    );
                    // flush cache
                    cache.timeout.store(0, Ordering::Relaxed);
                }
                Err(e) => {
                    page = StatusPage::new(
                        String::from(ADD_TITLE),
                        Some(format!("Error adding URL {}, due to: {:?}", add_url, e)),
                        None,
                    )
                }
            }
        }
        Err(e) => page = StatusPage::new(String::from(ADD_TITLE), Some(e), None),
    }
    Template::render("add", &page)
}

fn cron(conn: DirectoryDbConn) {
    match get_instances().load::<Instance>(&*conn) {
        Ok(instance_list) => {
            let mut instance_checks = vec![];
            let children: Vec<_> = instance_list
                .into_iter()
                .map(|instance| {
                    thread::spawn(move || {
                        // measure instance being up or down
                        let timer = Instant::now();
                        let check_result = CheckNew::new(instance.check_up(), instance.id);
                        (instance.url, check_result, timer.elapsed())
                    })
                })
                .collect(); // this starts the threads, then iterate over the results
            children.into_iter().for_each(|h| {
                let (instance_url, instance_check, elapsed) = h.join().unwrap();
                instance_checks.push(instance_check);
                println!("Instance {} checked ({:?})", instance_url, elapsed);
            });

            // store checks
            let timer = Instant::now();
            match diesel::insert_into(checks)
                .values(&instance_checks)
                .execute(&*conn)
            {
                Ok(_) => {
                    println!("stored uptime checks ({:?})", timer.elapsed());
                    let timer = Instant::now();

                    // delete checks older then:
                    // now - ((CHECKS_TO_STORE - 1) * CRON_INTERVAL)
                    let cutoff = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                        - ((CHECKS_TO_STORE - 1) * CRON_INTERVAL);
                    match diesel::delete(checks)
                        .filter(updated.lt(diesel::dsl::sql(&format!(
                            "datetime({}, 'unixepoch')",
                            cutoff
                        ))))
                        .execute(&*conn)
                    {
                        Ok(_) => {
                            println!(
                                "cleaned up checks stored before {} ({:?})",
                                cutoff,
                                timer.elapsed()
                            );
                        }
                        Err(e) => {
                            println!(
                                "failed to cleanup checks stored before {}, with error: {}",
                                cutoff, e
                            );
                        }
                    }
                }
                Err(e) => {
                    println!("failed to store uptime checks with error: {}", e);
                }
            }
        }
        Err(e) => {
            println!(
                "failed retrieving instances from database with error: {}",
                e
            );
        }
    }
}

fn cron_full(conn: DirectoryDbConn) {
    match get_instances().load::<Instance>(&*conn) {
        Ok(instance_list) => {
            let mut instance_update_queries = vec![];
            let mut scan_update_queries = vec![];
            let children: Vec<_> = instance_list
                .into_iter()
                .map(|instance| {
                    thread::spawn(move || {
                        let timer = Instant::now();
                        let mut result = String::new();
                        let mut instance_options = [
                            ("version", instance.version.clone(), String::new()),
                            (
                                "https",
                                format!("{:?}", instance.https.clone()),
                                String::new(),
                            ),
                            (
                                "https_redirect",
                                format!("{:?}", instance.https_redirect.clone()),
                                String::new(),
                            ),
                            (
                                "attachments",
                                format!("{:?}", instance.attachments.clone()),
                                String::new(),
                            ),
                            ("country_id", instance.country_id.clone(), String::new()),
                        ];
                        let mut scan: ScanNew;
                        let mut instance_update = None;
                        let mut instance_update_success = String::new();
                        let mut scan_update = None;
                        let mut scan_update_success = String::new();
                        match PrivateBin::new(instance.url.clone()) {
                            Ok(privatebin) => {
                                instance_options[0].2 = privatebin.instance.version.clone();
                                instance_options[1].2 =
                                    format!("{:?}", privatebin.instance.https.clone());
                                instance_options[2].2 =
                                    format!("{:?}", privatebin.instance.https_redirect.clone());
                                instance_options[3].2 =
                                    format!("{:?}", privatebin.instance.attachments.clone());
                                instance_options[4].2 = privatebin.instance.country_id.clone();
                                if instance_options.iter().any(|x| x.1 != x.2) {
                                    instance_update = Some(
                                        diesel::update(instances.filter(id.eq(instance.id))).set((
                                            version.eq(privatebin.instance.version),
                                            https.eq(privatebin.instance.https),
                                            https_redirect.eq(privatebin.instance.https_redirect),
                                            attachments.eq(privatebin.instance.attachments),
                                            country_id.eq(privatebin.instance.country_id),
                                        )),
                                    );
                                    let _ = writeln!(
                                        &mut instance_update_success,
                                        "Instance {} checked and updated ({:?}):",
                                        instance.url,
                                        timer.elapsed()
                                    );
                                    for (label, old, new) in instance_options.iter() {
                                        if old != new {
                                            let _ = writeln!(
                                                &mut instance_update_success,
                                                "    {} was {}, updated to {}",
                                                label, old, new
                                            );
                                        }
                                    }
                                } else {
                                    let _ = writeln!(
                                        &mut result,
                                        "Instance {} checked, no update required ({:?})",
                                        instance.url,
                                        timer.elapsed()
                                    );
                                }

                                let timer = Instant::now();
                                // retrieve latest scan
                                scan = privatebin.scans[0].clone();
                                // if missing, wait for the scan to conclude and poll again
                                if scan.rating == "-" {
                                    thread::sleep(std::time::Duration::from_secs(5));
                                    scan = models::PrivateBin::get_rating_mozilla_observatory(
                                        &instance.url,
                                    );
                                }
                                if scan.rating != "-"
                                    && scan.rating != instance.rating_mozilla_observatory
                                {
                                    scan_update = Some(
                                        diesel::update(
                                            scans
                                                .filter(
                                                    schema::scans::dsl::instance_id.eq(instance.id),
                                                )
                                                .filter(scanner.eq(scan.scanner)),
                                        )
                                        .set((
                                            rating.eq(scan.rating.clone()),
                                            percent.eq(scan.percent),
                                        )),
                                    );
                                    let _ = writeln!(
                                        &mut scan_update_success,
                                        "Instance {} rating updated to: {} ({:?})",
                                        instance.url,
                                        scan.rating,
                                        timer.elapsed()
                                    );
                                } else {
                                    let _ = writeln!(
                                        &mut scan_update_success,
                                        "Instance {} rating remains unchanged at: {} ({:?})",
                                        instance.url,
                                        scan.rating,
                                        timer.elapsed()
                                    );
                                }
                            }
                            Err(e) => {
                                let _ = writeln!(
                                    &mut result,
                                    "Instance {} failed to be checked with error: {}",
                                    instance.url, e
                                );
                            }
                        }

                        (
                            result,
                            scan_update,
                            scan_update_success,
                            instance,
                            instance_update,
                            instance_update_success,
                        )
                    })
                })
                .collect(); // this starts the threads, then iterate over the results
            children.into_iter().for_each(|h| {
                let (
                    thread_result,
                    scan_update,
                    scan_update_success,
                    instance,
                    instance_update,
                    instance_update_success,
                ) = h.join().unwrap();
                print!("{}", thread_result);

                // robots.txt must have changed or site no longer an instance, delete it immediately
                if thread_result.ends_with("doesn't want to get added to the directory.")
                    || thread_result.ends_with("doesn't seem to be a PrivateBin instance.")
                {
                    match sql_query(&format!(
                        "DELETE FROM instances \
                        WHERE id LIKE {};",
                        instance.id
                    ))
                    .execute(&*conn)
                    {
                        Ok(_) => println!("    removed the instance, due to: {}", thread_result),
                        Err(e) => {
                            println!("    error removing the instance: {}", e);
                        }
                    }
                    return;
                }

                if let Some(update_query) = scan_update {
                    scan_update_queries.push((
                        update_query,
                        scan_update_success,
                        instance.url.clone(),
                    ));
                }
                if let Some(update_query) = instance_update {
                    instance_update_queries.push((
                        update_query,
                        instance_update_success,
                        instance.url,
                    ));
                }
            });

            let timer = Instant::now();
            for (query, query_success, instance_url) in instance_update_queries {
                match query.execute(&*conn) {
                    Ok(_) => {
                        println!("{}", query_success);
                    }
                    Err(e) => {
                        println!(
                            "Instance {} failed to be updated with error: {:?}",
                            instance_url, e
                        );
                    }
                }
            }
            println!(
                "all instance update queries concluded ({:?})",
                timer.elapsed()
            );

            let timer = Instant::now();
            for (query, query_success, instance_url) in scan_update_queries {
                match query.execute(&*conn) {
                    Ok(_) => {
                        println!("{}", query_success);
                    }
                    Err(e) => {
                        println!(
                            "Instance {} failed to be updated with error: {:?}",
                            instance_url, e
                        );
                    }
                }
            }
            println!("all scan update queries concluded ({:?})", timer.elapsed());

            // delete checks and instances that failed too many times
            let timer = Instant::now();
            match sql_query(&format!(
                "DELETE FROM instances \
                WHERE id in ( \
                    SELECT instance_id \
                    FROM checks \
                    WHERE up = 0 \
                    GROUP BY instance_id \
                    HAVING COUNT(up) >= {} \
                );",
                MAX_FAILURES
            ))
            .execute(&*conn)
            {
                Ok(_) => println!(
                    "removed instances that failed too many times ({:?})",
                    timer.elapsed()
                ),
                Err(e) => {
                    println!("error removing instances failing too many times: {}", e);
                }
            }
        }
        Err(e) => {
            println!(
                "failed retrieving instances from database with error: {}",
                e
            );
        }
    }
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

fn get_instances() -> diesel::query_builder::SqlQuery {
    sql_query(
        "SELECT instances.id, url, version, https, https_redirect, attachments, \
            country_id, (100 * SUM(checks.up) / COUNT(checks.up)) AS uptime, \
            mozilla_observatory.rating AS rating_mozilla_observatory \
            FROM instances \
            JOIN checks ON instances.id = checks.instance_id \
            JOIN ( \
                SELECT rating, percent, instance_id \
                FROM scans WHERE scanner = \"mozilla_observatory\" \
            ) AS mozilla_observatory ON instances.id = mozilla_observatory.instance_id \
            GROUP BY instances.id \
            ORDER BY version DESC, https DESC, https_redirect DESC, \
            mozilla_observatory.percent DESC, attachments DESC, uptime DESC, url ASC \
            LIMIT 100",
    )
}

fn rocket() -> Rocket {
    rocket::ignite()
        .mount("/", routes![index, about, add, save, favicon])
        .mount("/img", StaticFiles::from("/img"))
        .mount("/css", StaticFiles::from("/css"))
        .attach(DirectoryDbConn::fairing())
        .attach(Template::fairing())
        .manage(InstancesCache {
            timeout: AtomicU64::new(0),
            instances: RwLock::new(vec![]),
        })
}

fn main() {
    let rocket = rocket();
    if let Ok(cron_env) = std::env::var("CRON") {
        let conn = DirectoryDbConn::get_one(&rocket).expect("database connection");
        if cron_env == "FULL" {
            cron_full(conn);
        } else {
            cron(conn);
        }
    } else {
        rocket
            .attach(AdHoc::on_attach("Database Migrations", run_db_migrations))
            .launch();
    }
}
