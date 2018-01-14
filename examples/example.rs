#![feature(plugin, custom_derive)]
#![plugin(rocket_codegen)]

extern crate rocket;
extern crate rocket_request_id;

use rocket_request_id::{RequestID, RequestIDFairing};

#[get("/")]
fn get(req_id: RequestID) -> String {
    let id: u64 = req_id.into(); // or u64::from(req_id)
    format!("My id is {}", id)
}

fn main() {
    rocket::ignite()
        .attach(RequestIDFairing)
        .mount("/", routes![get])
        .launch();
}
