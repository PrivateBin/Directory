#![forbid(unsafe_code)]

#[macro_use]
extern crate diesel;
#[macro_use]
extern crate diesel_migrations;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate rocket;
#[macro_use]
extern crate rocket_sync_db_pools;

use diesel::prelude::*;
use rocket::fairing::AdHoc;
use rocket::form::Form;
use rocket::response::Redirect;
use rocket::serde::json::Json;
use rocket::{Build, Error, Rocket, State};
use rocket_dyn_templates::Template;
use std::num::NonZeroU8;
use std::sync::atomic::Ordering::Relaxed;

pub mod connections;
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
const CHECK_TITLE: &str = "Check instance";

#[get("/")]
async fn index(db: DirectoryDbConn, cache: &State<InstancesCache>) -> Template {
    update_instance_cache(db, cache).await;

    let header = [
        String::from("Address"),
        String::from("Version"),
        String::from("HTTPS"),
        String::from("HTTPS enforced"),
        String::from("recommended CSP"),
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
                title: format!("Version {major}.{minor}").to_string(),
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
            Instance::format(instance.csp_header),
            instance.rating_mozilla_observatory.clone(),
            Instance::format(instance.attachments),
            format!("{}%", instance.uptime),
            instance.country_id.clone(),
        ]);
    }
    tables.push(HtmlTable {
        title: format!("Version {major}.{minor}"),
        header,
        body,
    });

    let page = TablePage::new(String::from("Welcome!"), tables);
    Template::render("list", &page)
}

#[get("/about")]
fn about() -> Template {
    let page = StatusPage::new(
        format!("About the {TITLE}"),
        Some(CSP_RECOMMENDATION.to_string()),
        Some(env!("CARGO_PKG_VERSION").to_string()),
    );
    Template::render("about", &page)
}

#[get("/add")]
fn add() -> Template {
    let page = StatusPage::new(String::from(ADD_TITLE), None, None);
    Template::render("form", &page)
}

#[post("/add", data = "<form>")]
async fn save(db: DirectoryDbConn, form: Form<AddForm>, cache: &State<InstancesCache>) -> Template {
    let form = form.into_inner();
    let add_url = form.url.trim();
    let privatebin_result = PrivateBin::new(add_url.to_string()).await;
    let (do_cache_flush, page) = match privatebin_result {
        Ok(privatebin) => {
            db.run(move |conn| {
                use schema::instances::dsl::*;
                match diesel::insert_into(instances)
                    .values(&privatebin.instance)
                    .execute(conn)
                {
                    Ok(_) => {
                        // need to store at least one check and scan, or the JOIN in /index produces NULL
                        let instance_id: i32 = instances
                            .select(id)
                            .filter(url.eq(privatebin.instance.url.clone()))
                            .limit(1)
                            .first(&*conn)
                            .expect("selecting the just inserted the instance");
                        diesel::insert_into(checks)
                            .values(CheckNew {
                                up: true,
                                instance_id,
                            })
                            .execute(&*conn)
                            .expect("inserting first check on a newly created instance");
                        diesel::insert_into(scans)
                            .values(ScanNew::new("mozilla_observatory", "-", instance_id))
                            .execute(&*conn)
                            .expect("inserting first scan on a newly created instance");

                        let add_url = privatebin.instance.url;
                        (
                            true,
                            StatusPage::new(
                                String::from(ADD_TITLE),
                                None,
                                Some(format!("Successfully added URL: {add_url}")),
                            ),
                        )
                    }
                    Err(e) => {
                        let add_url = form.url.trim();
                        (
                            false,
                            StatusPage::new(
                                String::from(ADD_TITLE),
                                Some(format!("Error adding URL {add_url}, due to: {e:?}")),
                                None,
                            ),
                        )
                    }
                }
            })
            .await
        }
        Err(e) => (
            false,
            StatusPage::new(String::from(ADD_TITLE), Some(e), None),
        ),
    };
    if do_cache_flush {
        cache.timeout.store(0, Relaxed);
    }
    Template::render("form", &page)
}

#[get("/check")]
fn check() -> Template {
    let page = StatusPage::new(String::from(CHECK_TITLE), None, None);
    Template::render("form", &page)
}

#[post("/check", data = "<form>")]
async fn report(
    db: DirectoryDbConn,
    form: Form<AddForm>,
    cache: &State<InstancesCache>,
) -> Template {
    let form = form.into_inner();
    let form_url = form.url.trim().to_string();
    let check_url = strip_url(form_url.clone());
    let check_success_title = format!("Results of checking {check_url}");
    let existing_instance = db
        .run(move |conn| {
            use diesel::sql_types::{Integer, Text};
            use schema::instances::dsl::*;
            instances
                .select((
                    id,
                    url,
                    version,
                    https,
                    https_redirect,
                    country_id,
                    attachments,
                    csp_header,
                    diesel::dsl::sql::<Integer>("(100 * SUM(checks.up) / COUNT(checks.up))"),
                    diesel::dsl::sql::<Text>("scans.rating"),
                ))
                .inner_join(checks)
                .inner_join(scans)
                .filter(url.eq(check_url))
                .first(&*conn)
        })
        .await;
    let page = match existing_instance {
        Ok(instance) => InstancePage::new(check_success_title, Some(instance), None),
        Err(_) => match PrivateBin::new(form_url.clone()).await {
            Ok(privatebin) => {
                let instance = Instance {
                    id: 0,
                    url: privatebin.instance.url,
                    version: privatebin.instance.version,
                    https: privatebin.instance.https,
                    https_redirect: privatebin.instance.https_redirect,
                    country_id: privatebin.instance.country_id,
                    attachments: privatebin.instance.attachments,
                    csp_header: privatebin.instance.csp_header,
                    uptime: 0,
                    rating_mozilla_observatory: privatebin
                        .scans
                        .last()
                        .unwrap_or(&ScanNew::default())
                        .rating
                        .clone(),
                };
                InstancePage::new(check_success_title, Some(instance), None)
            }
            Err(e) => InstancePage::new(
                String::from(CHECK_TITLE),
                None,
                Some(format!("Error scanning URL {form_url}, due to: {e:?}")),
            ),
        },
    };
    if page.error.is_empty() {
        return Template::render("check", &page);
    }
    Template::render("form", &StatusPage::new(page.topic, Some(page.error), None))
}

#[get(
    "/api?<top>&<attachments>&<country>&<csp_header>&<https>&<https_redirect>&<version>&<min_uptime>&<min_rating>",
    format = "json"
)]
async fn api(
    top: Option<NonZeroU8>,
    attachments: Option<bool>,
    country: Option<String>,
    csp_header: Option<bool>,
    https: Option<bool>,
    https_redirect: Option<bool>,
    version: Option<String>,
    min_uptime: Option<u8>,
    min_rating: Option<String>,
    db: DirectoryDbConn,
    cache: &State<InstancesCache>,
) -> Json<Vec<Instance>> {
    use rand::seq::SliceRandom;
    let mut instance_list: Vec<Instance> = vec![];
    update_instance_cache(db, cache).await;

    // unwrap & validate arguments
    let mut top: u8 = top.unwrap_or_else(|| NonZeroU8::new(10).unwrap()).into();
    if top > 100 {
        top = 100;
    }

    let is_attachments_set = attachments.is_some();
    let attachments = attachments.unwrap_or(false);

    let is_country_set = country.is_some();
    let country = country.unwrap_or_default();

    let is_csp_header_set = csp_header.is_some();
    let csp_header = csp_header.unwrap_or(false);

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
        if (is_attachments_set && instance.attachments != attachments)
            || (is_country_set && instance.country_id != country)
            || (is_csp_header_set && instance.csp_header != csp_header)
            || (is_https_set && instance.https != https)
            || (is_https_redirect_set && instance.https_redirect != https_redirect)
            || (is_version_set && !instance.version.starts_with(&version))
            || (instance.uptime < min_uptime)
            || (is_min_rating_set
                && rating_to_percent(&instance.rating_mozilla_observatory) < min_rating)
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

#[rocket::main]
async fn main() -> Result<(), Error> {
    let rocket = rocket();
    if let Ok(cron_env) = std::env::var("CRON") {
        if cron_env == "FULL" {
            check_full(rocket).await;
        } else {
            check_up(rocket).await;
        }
        return Ok(());
    }
    rocket
        .attach(AdHoc::on_ignite("Diesel Migrations", run_db_migrations))
        .launch()
        .await
        .map(|_| ())
}
