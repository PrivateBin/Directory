#![feature(proc_macro_hygiene, decl_macro, option_result_contains)]

#[macro_use] extern crate rocket;
use rocket::response::Redirect;
use rocket_contrib::serve::StaticFiles;
use rocket_contrib::templates::Template;
use serde::Serialize;

#[derive(Debug, PartialEq, Eq, Serialize)]
struct Page {
    title: String,
    body: String,
}

#[get("/")]
fn index() -> Template {
    let page = Page {
        title: String::from("Hello, world!"),
        body: String::from("This is a sample page"),
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

#[cfg(test)] mod tests;

fn main() {
    rocket().launch();
}
