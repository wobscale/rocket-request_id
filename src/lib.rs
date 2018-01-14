#![cfg_attr(test, feature(plugin, custom_derive))]
#![cfg_attr(test, plugin(rocket_codegen))]

#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
extern crate rand;
extern crate rocket;

use rocket::request::Request;
use rocket::http::Status;
use rocket::response::Response;
use rocket::request::FromRequest;
use rocket::request::Outcome as ReqOutcome;
use rocket::Outcome;
use rand::{thread_rng, Rng};
use std::collections::hash_map;
use std::sync::Mutex;

// yes, this is global state. Let's go over how we got here and other possible options:
//
// First of all, rocket provides no good interface for associating arbitrary data with a request.
// This is what state gets associated:
// https://github.com/SergioBenitez/Rocket/blob/v0.3.6/lib/src/request/request.rs#L20-L29
//
// Ideally, this would be managed state, but as far as I can tell, there's no good way to add state
// in a fairing or request guard (which makes sense because state is meant to be used via `.manage`
// on the main rocket instance, and then the same copy is passed out repeatedly).
//
// In addition, there's no good way to identify a request (or else we'd be done already, huh?), but
// `FromRequest` can be called an arbitrary number of times for the same request.. and we need it
// to return the same ids each time.
//
// That leaves us with the possible solutions which work:
// 1. Add a cookie or url hash or something to indicate the ID, make a request guard which reads
//    from that, or if it doesn't exist initializes it
//
//    This mutates the request the application's rocket handler would see in surprising ways, and
//    was thus deemed bad.
//
// 2. Use low level hackery to locate a request id either before or after the request in memory,
//    otherwise behave as above
//
//    This would be really cool, but unfortunately I don't know of a way to do that and also have
//    that hidden data get freed when a request is freed... so I'd still need a fairing to find it
//    and free it, so it's no betterthan 3.
//
// 3. Keep a static map of currently know requests as identified by their memory address, add and
//    remove ids as requests come in and leave via a fairing.
//
//    This is the approach I've gone with. It's really what 2 would be, but less hacky.
//
// 4. Ask upstream to add a request id, or a way to associate arbitrary context with a request
//    (like go's context).
//
//    ... This is probably the best idea, but hasn't been done yet.
lazy_static!{
    static ref REQUEST_IDS: Mutex<hash_map::HashMap<usize, u64, hash_map::RandomState>> =
        Mutex::new(hash_map::HashMap::new());
}

///
/// A `Fairing` that must be attached to a rocket instance before a `RequestID` request guard may
/// be used.
///
/// It should be attached like so:
/// ```
/// use rocket_request_id;
///
/// rocket::ignite()
///     .attach(rocket_request_id::RequestIDFairing)
///     .launch();
/// ```
///
pub struct RequestIDFairing;

impl<'r> rocket::fairing::Fairing for RequestIDFairing {
    fn info(&self) -> rocket::fairing::Info {
        rocket::fairing::Info {
            kind: rocket::fairing::Kind::Request | rocket::fairing::Kind::Response,
            name: "request id",
        }
    }

    fn on_request(&self, request: &mut Request, _: &rocket::Data) {
        REQUEST_IDS
            .lock()
            .unwrap()
            .insert(request as *const Request as usize, thread_rng().gen());
    }
    fn on_response(&self, request: &Request, _: &mut Response) {
        REQUEST_IDS
            .lock()
            .unwrap()
            .remove(&(request as *const Request as usize));
    }
}

///
/// A unique ID for a given rocket request.
/// This ID should be retrieved via its `FromRequest` implementation; that is to say, add an
/// argument of the type `RequestID` to a rocket handler. That argument can then be dereferenced to
/// access the ID.
///
/// If multiple parameters of this type are requested, each will have the same ID.
/// This property holds even if they are instantiated by other request guards.
///
/// For example, the following is a typical usage:
/// ```
/// use rocket_request_id;
///
/// #[get("/")]
/// fn test_req_id(id: rocket_request_id::RequestID) -> String {
///     format!("Hello, your request had ID {}", *id)
/// }
/// ```
#[derive(Debug, PartialEq, Eq)]
pub struct RequestID {
    id: u64,
}

impl From<RequestID> for u64 {
    fn from(r: RequestID) -> u64 {
        r.id
    }
}

impl<'a, 'r> FromRequest<'a, 'r> for RequestID {
    type Error = ();

    fn from_request(request: &'a Request<'r>) -> ReqOutcome<Self, Self::Error> {
        match REQUEST_IDS
            .lock()
            .unwrap()
            .get(&(request as *const Request as usize))
        {
            Some(id) => Outcome::Success(RequestID { id: id.clone() }),
            None => {
                error!("unable to get request id: did you forget to attach the fairing?");
                Outcome::Failure((Status::InternalServerError, ()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use rocket;
    use super::*;
    use rocket::local::Client;

    #[get("/")]
    fn req_id(id: RequestID) -> String {
        format!("{}", u64::from(id))
    }

    #[test]
    fn unique_ids() {
        let rkt = rocket::ignite()
            .attach(RequestIDFairing)
            .mount("/", routes![req_id]);
        let c = Client::new(rkt).unwrap();

        let mut resp1 = c.get("/").dispatch();
        let mut resp2 = c.get("/").dispatch();

        assert_eq!(resp1.status(), Status::Ok);
        assert_eq!(resp2.status(), Status::Ok);
        assert_ne!(resp1.body_string(), resp2.body_string());
    }

    #[test]
    fn doesnt_leak() {
        let rkt = rocket::ignite()
            .attach(RequestIDFairing)
            .mount("/", routes![req_id]);
        let c = Client::new(rkt).unwrap();

        assert_eq!(c.get("/").dispatch().status(), Status::Ok);
        assert_eq!(c.get("/").dispatch().status(), Status::Ok);

        assert_eq!(REQUEST_IDS.lock().unwrap().len(), 0);
    }

    #[get("/")]
    fn multiple(id1: RequestID, id2: RequestID) -> String {
        assert_eq!(id1, id2);
        "".to_string()
    }

    struct TestGuard {
        id: RequestID,
    }

    impl<'a, 'r> FromRequest<'a, 'r> for TestGuard {
        type Error = ();

        fn from_request(request: &'a Request<'r>) -> ReqOutcome<Self, Self::Error> {
            Outcome::Success(TestGuard {
                id: request.guard().unwrap(),
            })
        }
    }

    #[get("/")]
    fn multiple_with_guard(id1: RequestID, guard2: TestGuard) -> String {
        assert_eq!(id1, guard2.id);
        "".to_string()
    }

    #[test]
    fn same_in_same_request() {
        let rkt = rocket::ignite()
            .attach(RequestIDFairing)
            .mount("/", routes![multiple_with_guard]);
        let c = Client::new(rkt).unwrap();

        assert_eq!(c.get("/").dispatch().status(), Status::Ok);
    }
}
