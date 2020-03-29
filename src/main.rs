#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use] extern crate diesel;
#[macro_use] extern crate rocket;
#[macro_use] extern crate rocket_contrib;
use diesel::prelude::*;
use rocket::response::Redirect;
use rocket::request::Form;
//use rocket_contrib::databases::diesel; not working with current diesel
use rocket_contrib::serve::StaticFiles;
use rocket_contrib::templates::Template;

pub mod models;
pub mod schema;
#[cfg(test)] mod tests;
use models::*;

#[get("/")]
fn index(conn: DirectoryDbConn) -> Template {
    use schema::instances::dsl::*;

    let results = instances.order(
            (version.desc(), https.desc(), https_redirect.desc(), attachments.desc(), url.asc())
        )
        .limit(100)
        .load::<Instance>(&*conn)
        .unwrap();

    let mut table_body = vec![];
    for instance in results {
        table_body.push([
            instance.url,
            instance.version,
            Instance::format(instance.https),
            Instance::format(instance.https_redirect),
            Instance::format(instance.attachments),
            Instance::format_country(instance.country_id)
        ]);
    }

    let page = TablePage::new(
        String::from("Welcome!"),
        HtmlTable {
            title: String::from("Version 1.3"),
            header: [String::from("Address"), String::from("Version"), String::from("HTTPS"), String::from("HTTPS enforced"), String::from("File upload"), String::from("Country")],
            body: table_body
        }
    );
    Template::render("list", &page)
}

const ADD_TITLE: &str = "Add instance";

#[get("/add")]
fn add() -> Template {
    let page = StatusPage::new(String::from(ADD_TITLE), String::from(""), String::from(""));
    Template::render("add", &page)
}

#[post("/add", data = "<form>")]
fn save(conn: DirectoryDbConn, form: Form<AddForm>) -> Template {
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
                Ok(_msg) => page = StatusPage::new(String::from(ADD_TITLE), String::from(""), format!("Successfully added URL: {}", add_url)),
                Err(e) => page = StatusPage::new(String::from(ADD_TITLE), format!("Error adding URL {}, due to: {:?}", add_url, e), String::from(""))
            }
        },
        Err(e) => page = StatusPage::new(String::from(ADD_TITLE), e, String::from(""))
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
}

fn main() {
    rocket().launch();
}
