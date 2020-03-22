#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use] extern crate rocket;
use rocket::response::Redirect;
use rocket_contrib::serve::StaticFiles;

#[get("/")]
fn index() -> &'static str {
    "Hello, world!"
}

#[get("/favicon.ico")]
fn favicon() -> Redirect {
    Redirect::permanent("/img/favicon.ico")
}

fn rocket() -> rocket::Rocket {
    rocket::ignite()
        .mount("/", routes![index, favicon])
        .mount("/img", StaticFiles::from("/img"))
}

#[cfg(test)] mod tests;

fn main() {
    rocket().launch();
}
