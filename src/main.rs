#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use] extern crate rocket;
use rocket::response::Redirect;
use rocket_contrib::serve::StaticFiles;
use rocket_contrib::templates::Template;

mod models;
#[cfg(test)] mod tests;

#[get("/")]
fn index() -> Template {
    let page = models::Page {
        title: String::from("Instance Directory"),
        topic: String::from("Welcome!"),
        table: models::Table {
            title: String::from("Version 1.3"),
            header: [String::from("Instance"), String::from("HTTPS"), String::from("Country")],
            body: vec![
                [String::from("foo"), String::from("\u{2714}"), String::from("\u{1F1E6}\u{1F1F6}")],
                [String::from("bar"), String::from("\u{2714}"), String::from("\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}")],
                [String::from("baz"), String::from("\u{2718}"), String::from("\u{1F3F4}\u{200D}\u{2620}\u{FE0F}")],
            ]
        }
    };
    Template::render("list", &page)
}

#[get("/favicon.ico")]
fn favicon() -> Redirect {
    Redirect::permanent("/img/favicon.ico")
}

fn rocket() -> rocket::Rocket {
    rocket::ignite()
        .mount("/", routes![index, favicon])
        .mount("/img", StaticFiles::from("/img"))
        .attach(Template::fairing())
}

fn main() {
    rocket().launch();
}
