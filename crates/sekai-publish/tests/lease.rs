//! The lease of the site of a bucket, against a local server that follows conditional writes.

#![cfg(feature = "s3")]

use std::thread;
use std::time::{Duration, Instant};

use sekai_publish::lease::{Holder, Periods};
use sekai_publish::s3::{Bucket, Credentials, S3};
use sekai_testkit::s3::{ACCESS_KEY, REGION, SECRET_KEY, Server};

/// Short periods, so that keeping and waiting happen within a test. The age of a lease comes from
/// HTTP dates, which count whole seconds, so the lease lasts a few of them: a lease written just
/// before a second turns is then still held when it is read just after.
const QUICK: Periods = Periods {
    lease: Duration::from_secs(3),
    keep: Duration::from_millis(20),
    wait: Duration::from_millis(20),
};

/// The name of the lease file in the bucket.
const LEASE_NAME: &str = "out/state/lease.json";

fn client(server: &Server) -> S3 {
    S3::new(
        Bucket {
            endpoint: server.endpoint().to_owned(),
            region: REGION.to_owned(),
            prefix: "out/".to_owned(),
            connections: 3,
            listings: 3,
        },
        Credentials {
            access_key_id: ACCESS_KEY.to_owned(),
            secret_access_key: SECRET_KEY.to_owned(),
        },
    )
    .unwrap()
}

/// The holder that the lease file of `server` names.
fn holder(server: &Server) -> Option<Holder> {
    let bytes = server.object(LEASE_NAME)?;
    Some(serde_json::from_slice(&bytes).unwrap())
}

/// A lease file of another run.
fn other_run() -> Vec<u8> {
    let holder = serde_json::json!({
        "format": "site-lease",
        "version": 1,
        "run": "0123456789abcdef",
        "operation": "watch jp",
        "host": "elsewhere",
        "since": "2026-01-01T00:00:00Z",
    });
    serde_json::to_vec(&holder).unwrap()
}

fn never_waits(holder: &Holder) {
    panic!("waited for {holder:?}");
}

#[test]
fn a_run_holds_the_lease_while_it_works_and_gives_it_up_after() {
    let server = Server::start();
    let s3 = client(&server);
    let during = s3
        .while_held_for(&QUICK, "update jp", &mut never_waits, || {
            thread::sleep(QUICK.keep * 3);
            holder(&server)
        })
        .unwrap()
        .unwrap();
    assert_eq!(during.operation, "update jp");
    assert_eq!(during.format, "site-lease");
    assert!(server.object(LEASE_NAME).is_none());
}

#[test]
fn a_run_waits_while_another_holds_the_lease() {
    let server = Server::start();
    server.insert(LEASE_NAME, other_run());
    let s3 = client(&server);
    let mut seen = Vec::new();
    let mut waiting = |holder: &Holder| {
        seen.push(holder.operation.clone());
        // The other run gives the lease up.
        server.remove(LEASE_NAME);
    };
    let worked = s3.while_held_for(&QUICK, "update cn", &mut waiting, || holder(&server));
    assert_eq!(worked.unwrap().unwrap().operation, "update cn");
    assert_eq!(seen, ["watch jp"]);
}

#[test]
fn a_lease_that_was_not_kept_is_taken_over() {
    let server = Server::start();
    server.insert(LEASE_NAME, other_run());
    server.backdate(LEASE_NAME, QUICK.lease * 2);
    let s3 = client(&server);
    let during = s3.while_held_for(&QUICK, "gc", &mut never_waits, || holder(&server));
    assert_eq!(during.unwrap().unwrap().operation, "gc");
    assert!(server.object(LEASE_NAME).is_none());
}

#[test]
fn a_run_whose_lease_was_taken_over_writes_no_state_and_leaves_the_lease_alone() {
    let server = Server::start();
    let s3 = client(&server);
    let refused = s3
        .while_held_for(&QUICK, "update jp", &mut never_waits, || {
            s3.write_state("before.json", b"{}").unwrap();
            // Another run takes the lease over; the next keeping finds it, and from then on no
            // state is written.
            server.insert(LEASE_NAME, other_run());
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if let Err(e) = s3.write_state("after.json", b"{}") {
                    return Some(e);
                }
                thread::sleep(QUICK.keep);
            }
            None
        })
        .unwrap();
    assert!(refused.unwrap().to_string().contains("lease"));
    assert!(server.object("out/state/before.json").is_some());
    assert_eq!(holder(&server).unwrap().operation, "watch jp");
}
