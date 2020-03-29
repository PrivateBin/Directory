#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use] extern crate diesel;
#[macro_use] extern crate rocket;
#[macro_use] extern crate rocket_contrib;
use diesel::prelude::*;
use rocket::response::Redirect;
use rocket::request::Form;
use rocket::State;
//use rocket_contrib::databases::diesel; not working with current diesel
use rocket_contrib::serve::StaticFiles;
use rocket_contrib::templates::Template;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::SystemTime;

pub mod models;
pub mod schema;
#[cfg(test)] mod tests;
use models::*;

const CACHE_LIFETIME: u64 = 300;

#[get("/")]
fn index(conn: DirectoryDbConn, cache: State<InstancesCache>) -> Template {
    use schema::instances::dsl::*;

    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
    if now >= cache.timeout.load(Ordering::Relaxed) {
        // flush cache
        let mut instances_cache = cache.instances.write().unwrap();
        cache.timeout.store(now + CACHE_LIFETIME, Ordering::Relaxed);
        *instances_cache = instances.order((
                version.desc(),
                https.desc(),
                https_redirect.desc(),
                attachments.desc(),
                url.asc()
            ))
            .limit(100)
            .load::<Instance>(&*conn)
            .unwrap();
    }

    let header = [
        String::from("Address"),
        String::from("Version"),
        String::from("HTTPS"),
        String::from("HTTPS enforced"),
        String::from("File upload"),
        String::from("Country")
    ];
    let mut tables = vec![];
    let mut table_body = vec![];
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
                    body: table_body.clone()
                }
            );
            // start a new one
            major = instance_major;
            minor = instance_minor;
            table_body.clear();
        }

        // format current instance for table display
        table_body.push([
            instance.url.clone(),
            instance.version.clone(),
            Instance::format(instance.https),
            Instance::format(instance.https_redirect),
            Instance::format(instance.attachments),
            Instance::format_country(instance.country_id.clone())
        ]);
    }
    tables.push(
        HtmlTable {
            title: format!("Version {}.{}", major, minor).to_string(),
            header: header,
            body: table_body
        }
    );

    let page = TablePage::new(
        String::from("Welcome!"),
        tables
    );
    Template::render("list", &page)
}

const ADD_TITLE: &str = "Add instance";

#[get("/add")]
fn add() -> Template {
    let page = StatusPage::new(
        String::from(ADD_TITLE),
        String::new(),
        String::new()
    );
    Template::render("add", &page)
}

#[post("/add", data = "<form>")]
fn save(conn: DirectoryDbConn, form: Form<AddForm>, cache: State<InstancesCache>) -> Template {
    use schema::instances::dsl::*;

    let form = form.into_inner();
    let mut add_url = form.url.trim();

    // remove trailing slash, but only for web root, not for paths
    // https://example.com/ -> https://example.com BUT NOT https://example.com/path/
    if add_url.matches("/").count() == 3 {
        add_url = add_url.trim_end_matches('/');
    }

    let page: StatusPage;
    let privatebin = PrivateBin::new(add_url.to_string());
    match privatebin {
        Ok(privatebin) => {
            let db_result = diesel::insert_into(instances)
                .values(&privatebin.instance)
                .execute(&*conn);
            match db_result {
                Ok(_msg) => {
                    page = StatusPage::new(
                        String::from(ADD_TITLE),
                        String::from(""),
                        format!("Successfully added URL: {}", add_url)
                    );
                    // flush cache
                    cache.timeout.store(0, Ordering::Relaxed);
                },
                Err(e) => page = StatusPage::new(
                    String::from(ADD_TITLE),
                    format!("Error adding URL {}, due to: {:?}", add_url, e),
                    String::new()
                )
            }
        },
        Err(e) => page = StatusPage::new(
            String::from(ADD_TITLE),
            e,
            String::new()
        )
    }
    Template::render("add", &page)
}

#[get("/favicon.ico")]
fn favicon() -> Redirect {
    Redirect::permanent("/img/favicon.ico")
}

#[database("directory")]
struct DirectoryDbConn(diesel::SqliteConnection);

fn rocket() -> rocket::Rocket {
    rocket::ignite()
        .mount("/", routes![index, add, save, favicon])
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
    rocket().launch();
}
