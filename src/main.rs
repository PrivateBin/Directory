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
    "/api?<top>&<attachments>&<country>&<https>&<https_redirect>&<version>&<min_uptime>&<min_rating>",
    format = "json"
)]
fn api(
    top: Option<NonZeroU8>,
    attachments: Option<bool>,
    country: Option<String>,
    https: Option<bool>,
    https_redirect: Option<bool>,
    version: Option<String>,
    min_uptime: Option<u8>,
    min_rating: Option<String>,
    conn: DirectoryDbConn,
    cache: State<InstancesCache>,
) -> Json<Vec<Instance>> {
    use rand::seq::SliceRandom;
    let mut instance_list: Vec<Instance> = vec![];
    update_instance_cache(conn, &cache);

    // unwrap & validate arguments
    let mut top: u8 = top.unwrap_or_else(|| NonZeroU8::new(10).unwrap()).into();
    if top > 100 {
        top = 100;
    }

    let is_attachments_set = attachments.is_some();
    let attachments = attachments.unwrap_or(false);

    let is_country_set = country.is_some();
    let country = country.unwrap_or_default();

    let is_https_set = https.is_some();
    let https = https.unwrap_or(false);

    let is_https_redirect_set = https_redirect.is_some();
    let https_redirect = https_redirect.unwrap_or(false);

    let is_version_set = version.is_some();
    let version = version.unwrap_or_default();

    let mut min_uptime: i32 = min_uptime.unwrap_or(0).into();
    if min_uptime > 100 {
        min_uptime = 100;
    }

    let is_min_rating_set = min_rating.is_some();
    let min_rating = rating_to_percent(&min_rating.unwrap_or_else(|| "F".to_string()));

    // prepare list according to arguments
    for instance in &*cache.instances.read().unwrap() {
        if is_attachments_set && instance.attachments != attachments {
            continue;
        }
        if is_country_set && instance.country_id != country {
            continue;
        }
        if is_https_set && instance.https != https {
            continue;
        }
        if is_https_redirect_set && instance.https_redirect != https_redirect {
            continue;
        }
        if is_version_set && instance.version.starts_with(&version) {
            continue;
        }
        if instance.uptime < min_uptime {
            continue;
        }
        if is_min_rating_set && min_rating < rating_to_percent(&instance.rating_mozilla_observatory)
        {
            continue;
        }

        instance_list.push(instance.clone());
        top -= 1;
        if top == 0 {
            break;
        }
    }
    let mut rng = rand::thread_rng();
    instance_list.shuffle(&mut rng);
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
