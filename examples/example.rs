#![feature(plugin, custom_derive)]
#![plugin(rocket_codegen)]

extern crate rocket;
extern crate rocket_request_id;

use rocket_request_id::{RequestID, RequestIDFairing};

#[get("/")]
fn get(id: RequestID) -> String {
    format!("My id is {}", *id)
}

fn main() {
    rocket::ignite()
        .attach(RequestIDFairing)
        .mount("/", routes![get])
        .launch();
}
