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

use diesel::{insert_into, prelude::*};
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
        "Address".into(),
        "Version".into(),
        "HTTPS".into(),
        "HTTPS enforced".into(),
        "recommended CSP".into(),
        "Observatory Rating".into(),
        "File upload".into(),
        "Uptime".into(),
        "Country".into(),
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
                title: format!("Version {major}.{minor}"),
                header: header.to_owned(),
                body: body.to_owned(),
            });
            // start a new one
            major = instance_major;
            minor = instance_minor;
            body.clear();
        }

        // format current instance for table display
        body.push([
            format!("opacity{}", instance.uptime / 25),
            instance.url.to_owned(),
            instance.version.to_owned(),
            Instance::format(instance.https),
            Instance::format(instance.https_redirect),
            Instance::format(instance.csp_header),
            instance.rating_mozilla_observatory.to_owned(),
            Instance::format(instance.attachments),
            format!("{}%", instance.uptime),
            instance.country_id.to_owned(),
        ]);
    }
    tables.push(HtmlTable {
        title: format!("Version {major}.{minor}"),
        header,
        body,
    });

    let page = TablePage::new("Welcome!".into(), tables);
    Template::render("list", &page)
}

#[get("/about")]
fn about() -> Template {
    let page = StatusPage::new(
        format!("About the {TITLE}"),
        Some(CSP_RECOMMENDATION.into()),
        Some(env!("CARGO_PKG_VERSION").into()),
    );
    Template::render("about", &page)
}

#[get("/add")]
fn add() -> Template {
    let page = StatusPage::new(ADD_TITLE.into(), None, None);
    Template::render("form", &page)
}

#[post("/add", data = "<form>")]
async fn save(db: DirectoryDbConn, form: Form<AddForm>, cache: &State<InstancesCache>) -> Template {
    let form = form.into_inner();
    let add_url = form.url.trim();

    // check in negative lookup cache, prevent unnecessary lookups
    if is_cached(&cache.negative_lookups, add_url) {
        return Template::render(
            "form",
            &StatusPage::new(
                ADD_TITLE.into(),
                Some(format!(
                    "Error adding URL {add_url}, due to a failed scan within the last 5 minutes."
                )),
                None,
            ),
        );
    }

    // scan the new instance
    let privatebin_result = PrivateBin::new(add_url.into()).await;
    let (do_cache_flush, page) = match privatebin_result {
        Ok(privatebin) => {
            db.run(move |conn| {
                use schema::instances::dsl::*;
                match insert_into(instances)
                    .values(&privatebin.instance)
                    .execute(conn)
                {
                    Ok(_) => {
                        // need to store at least one check and scan, or the JOIN in /index produces NULL
                        let instance_id: i32 = instances
                            .select(id)
                            .filter(url.eq(privatebin.instance.url.to_owned()))
                            .limit(1)
                            .first(conn)
                            .expect("selecting the just inserted the instance");
                        insert_into(checks)
                            .values(CheckNew {
                                up: true,
                                instance_id,
                            })
                            .execute(conn)
                            .expect("inserting first check on a newly created instance");
                        insert_into(scans)
                            .values(ScanNew::new("mozilla_observatory", "-", instance_id))
                            .execute(conn)
                            .expect("inserting first scan on a newly created instance");

                        let add_url = privatebin.instance.url;
                        (
                            true,
                            StatusPage::new(
                                ADD_TITLE.into(),
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
                                ADD_TITLE.into(),
                                Some(format!("Error adding URL {add_url}, due to: {e:?}")),
                                None,
                            ),
                        )
                    }
                }
            })
            .await
        }
        Err(e) => {
            // don't query this site again, for another 5 minutes
            set_cached(&cache.negative_lookups, add_url);
            (false, StatusPage::new(ADD_TITLE.into(), Some(e), None))
        }
    };
    if do_cache_flush {
        cache.timeout.store(0, Relaxed);
    }
    Template::render("form", &page)
}

#[get("/check")]
fn check() -> Template {
    let page = StatusPage::new(CHECK_TITLE.into(), None, None);
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
    let check_url = strip_url(form_url.to_owned());
    let lookup_url = check_url.to_owned();
    let check_success_title = format!("Results of checking {check_url}");

    // check in negative lookup cache, prevent unnecessary lookups
    if is_cached(&cache.negative_lookups, &check_url) {
        return Template::render("form", &StatusPage::new(
            CHECK_TITLE.into(),
            Some(format!("Error scanning URL {form_url}, due to a failed scan within the last 5 minutes.")),
            None)
        );
    }

    // check in database
    let page = match db
        .run(move |conn| {
            use diesel::dsl::sql;
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
                    sql::<Integer>("(100 * SUM(checks.up) / COUNT(checks.up))"),
                    sql::<Text>("scans.rating"),
                ))
                .inner_join(checks)
                .inner_join(scans)
                .filter(url.eq(lookup_url))
                .first(conn)
        })
        .await
    {
        Ok(instance) => InstancePage::new(check_success_title, Some(instance), None),
        Err(_) => match PrivateBin::new(form_url.to_owned()).await {
            // scan unknown instance
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
                        .to_owned(),
                };
                InstancePage::new(check_success_title, Some(instance), None)
            }
            Err(e) => {
                // don't query this site again, for another 5 minutes
                set_cached(&cache.negative_lookups, &check_url);
                InstancePage::new(
                    CHECK_TITLE.into(),
                    None,
                    Some(format!("Error scanning URL {form_url}, due to: {e:?}")),
                )
            }
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
    let min_rating = rating_to_percent(&min_rating.unwrap_or_else(|| "F".into()));

    // prepare list according to arguments
    for instance in &*cache.instances.read().unwrap() {
        if (is_csp_header_set && instance.csp_header != csp_header)
            || (is_https_set && instance.https != https)
            || (is_https_redirect_set && instance.https_redirect != https_redirect)
            || (is_attachments_set && instance.attachments != attachments)
            || (instance.uptime < min_uptime)
            || (is_version_set && !instance.version.starts_with(&version))
            || (is_country_set && instance.country_id != country)
            || (is_min_rating_set
                && rating_to_percent(&instance.rating_mozilla_observatory) < min_rating)
        {
            continue;
        }

        instance_list.push(instance.to_owned());
        top -= 1;
        if top == 0 {
            break;
        }
    }
    let mut rng = rand::thread_rng();
    instance_list.shuffle(&mut rng);
    Json(instance_list)
}

#[get("/forward-me?<attachments>&<country>&<version>")]
async fn forward_me(
    attachments: Option<bool>,
    country: Option<String>,
    version: Option<String>,
    db: DirectoryDbConn,
    cache: &State<InstancesCache>,
) -> Redirect {
    use rand::seq::SliceRandom;
    let mut instance_list: Vec<Instance> = vec![];
    update_instance_cache(db, cache).await;

    // unwrap & validate arguments
    let is_attachments_set = attachments.is_some();
    let attachments = attachments.unwrap_or(false);

    let is_country_set = country.is_some();
    let country = country.unwrap_or_default();

    let is_version_set = version.is_some();
    let version = version.unwrap_or_default();

    // prepare list according to arguments and hardcoded filter criteria
    let mut instance_version = String::new();
    for instance in &*cache.instances.read().unwrap() {
        if !instance.https
            || !instance.https_redirect
            || (!instance.csp_header && !is_version_set) // don't enforce CSP for older versions, most wont have it
            || instance.uptime < 100
            || rating_to_percent(&instance.rating_mozilla_observatory) < 90
            || (is_version_set && !instance.version.starts_with(&version))
            || (is_attachments_set && instance.attachments != attachments)
            || (is_country_set && instance.country_id != country)
        {
            continue;
        }
        // by default, we only consider the latest version - cached instances are sorted on these
        if instance_version.is_empty() {
            instance_version = instance.version.to_owned();
        } else if instance.version != instance_version {
            break;
        }
        instance_list.push(instance.to_owned());
    }

    let mut rng = rand::thread_rng();
    instance_list.shuffle(&mut rng);
    if instance_list.is_empty() {
        // safe fallback - likely we have some connectivity issues (no instance
        // with uptime of 100%) or work on an empty database, so redirect to
        // demo instance running on the same host
        return Redirect::to("https://privatebin.net");
    }
    // note the use of 303 See Other instead of 307 Temporary to ensure redirect
    // is changed to a GET method, if necessary, to avoid leaking request details
    Redirect::to(instance_list[0].url.to_owned())
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
