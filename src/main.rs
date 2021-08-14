#![forbid(unsafe_code)]
#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use]
extern crate diesel;
#[macro_use]
extern crate diesel_migrations;
#[macro_use]
extern crate lazy_static;
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
use rocket_contrib::json::Json;
use rocket_contrib::serve::StaticFiles;
use rocket_contrib::templates::Template;
use std::num::NonZeroU8;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

pub mod functions;
use functions::*;
pub mod models;
use models::*;
pub mod schema;
use schema::checks::dsl::checks;
use schema::scans::dsl::scans;
pub mod tasks;
use tasks::*;
#[cfg(test)]
mod tests;

const ADD_TITLE: &str = "Add instance";

#[get("/")]
fn index(conn: DirectoryDbConn, cache: State<InstancesCache>) -> Template {
    update_instance_cache(conn, &cache);

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
            instance.country_id.clone(),
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

#[get("/about")]
fn about() -> Template {
    let page = StatusPage::new(
        format!("About the {}", TITLE),
        None,
        Some(env!("CARGO_PKG_VERSION").to_string()),
    );
    Template::render("about", &page)
}

#[get("/add")]
fn add() -> Template {
    let page = StatusPage::new(String::from(ADD_TITLE), None, None);
    Template::render("add", &page)
}

#[post("/add", data = "<form>")]
fn save(conn: DirectoryDbConn, form: Form<AddForm>, cache: State<InstancesCache>) -> Template {
    use schema::instances::dsl::*;
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

#[get(
    "/api?<top>&<attachments>&<country>&<https>&<https_redirect>&<version>",
    format = "json"
)]
fn api(
    top: Option<NonZeroU8>,
    attachments: Option<bool>,
    country: Option<String>,
    https: Option<bool>,
    https_redirect: Option<bool>,
    version: Option<String>,
    conn: DirectoryDbConn,
    cache: State<InstancesCache>,
) -> Json<Vec<Instance>> {
    let mut instance_list: Vec<Instance> = vec![];
    update_instance_cache(conn, &cache);

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
            // start a new one
            major = instance_major;
            minor = instance_minor;
        }
        instance_list.push(instance.clone());
    }
    Json(instance_list)
}

#[get("/favicon.ico")]
fn favicon() -> Redirect {
    Redirect::permanent("/img/favicon.ico")
}

#[database("directory")]
pub struct DirectoryDbConn(diesel::SqliteConnection);
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
        .mount("/", routes![index, about, add, api, save, favicon])
        .mount("/img", StaticFiles::from("/img"))
        .mount("/css", StaticFiles::from("/css"))
        .attach(DirectoryDbConn::fairing())
        .attach(Template::custom(|engines| {
            engines.tera.register_filter("country", filter_country);
        }))
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
            check_full(conn);
        } else {
            check_up(conn);
        }
    } else {
        rocket
            .attach(AdHoc::on_attach("Database Migrations", run_db_migrations))
            .launch();
    }
}
