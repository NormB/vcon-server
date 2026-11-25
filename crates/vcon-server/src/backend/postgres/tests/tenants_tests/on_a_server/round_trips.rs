//! The round trips to the database of each call whose transactions read
//! the database's clock (`PgTx::clock`): the reading comes in the batch
//! that begins a transaction, with its first statement, so it adds none.
//! A call that read the clock before any row would send the begin with a
//! query of its own, one round trip more, and is seen here. Each count is
//! the one the call cost before its times came from the database's clock.

use super::{
    Actor, Caller, Catalogs, Key, Retention, TenantId, Tenants, conformance, packed_world, world,
};
use crate::access::{self, Scope, Scopes};
use crate::roles::{Role, Roles};
use crate::store::packer::PackPlan;
use crate::tenants::{KeyLife, Proof};

/// The round trips so far to the database of `catalogs`.
fn trips(catalogs: &Catalogs) -> usize {
    match catalogs {
        Catalogs::Postgres(database) => database.round_trips(),
        _ => panic!("the catalogs are a database's"),
    }
}

/// The round trips `call` cost.
fn counted<T>(catalogs: &Catalogs, call: impl FnOnce() -> T) -> (T, usize) {
    let start = trips(catalogs);
    let answer = call();
    (answer, trips(catalogs) - start)
}

fn who() -> Caller {
    Caller {
        key: Key::generate().expect("a key").id(),
    }
}

/// Stores vCon `n` through `tenants`.
fn store(tenants: &Tenants, id: TenantId, caller: &Caller, n: u8) -> uuid::Uuid {
    let (found, body) = conformance::vcon(
        &format!("01a07da8-c2bb-83e5-b9a2-279e0d16be{n:02x}"),
        "counted",
    );
    tenants
        .store(id)
        .expect("the store")
        .create(caller, &found, &body)
        .expect("stored");
    found.uuid
}

#[test]
fn a_key_check_costs_the_round_trips_it_did_before() {
    let w = world();
    let (catalogs, tenants) = w.serve();
    let id = tenants.create(Actor::Cli, "acme", None).expect("a tenant");
    let made = tenants
        .create_key_for(
            Actor::Cli,
            Some(id),
            Scopes::of(&[Scope::Read]),
            "counted",
            KeyLife::Days(1),
        )
        .expect("a key");
    let header = format!("Bearer {}", made.token);
    // The first use writes the last-used minute; the second, in the same
    // minute, only reads the key.
    access::authenticate(&tenants, Some(&header)).expect("accepted");
    let (answer, cost) = counted(&catalogs, || access::authenticate(&tenants, Some(&header)));
    answer.expect("accepted");
    assert_eq!(cost, KEY_CHECK_TRIPS, "a key check, in round trips");
}

#[test]
fn a_session_s_calls_cost_the_round_trips_they_did_before() {
    let w = world();
    let (catalogs, tenants) = w.serve();
    let id = tenants.create(Actor::Cli, "acme", None).expect("a tenant");
    let user = tenants
        .add_user(Actor::Cli, "Member", None)
        .expect("a user");
    tenants
        .grant_membership(Actor::Cli, user, Some(id), Roles::of(&[Role::Reader]))
        .expect("granted");
    let (started, start) = counted(&catalogs, || {
        tenants.start_session(user, Some(id), &["hwk"])
    });
    let started = started.expect("a session");
    let (seen, seen_cost) = counted(&catalogs, || {
        tenants.session(&started.id, Proof::Csrf(Some(&started.csrf)))
    });
    seen.expect("live");
    let (stepped, step_up) = counted(&catalogs, || {
        tenants.step_up_session(&started.id, &["hwk", "otp"])
    });
    let stepped = stepped.expect("stepped up");
    let (ended, end) = counted(&catalogs, || tenants.end_session(&stepped.id));
    ended.expect("ended");
    assert_eq!(
        (start, seen_cost, step_up, end),
        SESSION_TRIPS,
        "a start, a request, a step-up and an end, in round trips"
    );
}

#[test]
fn a_tenant_create_and_a_rekey_cost_the_round_trips_they_did_before() {
    let w = world();
    let (catalogs, tenants) = w.serve();
    tenants.create(Actor::Cli, "first", None).expect("a tenant");
    let (id, create) = counted(&catalogs, || tenants.create(Actor::Cli, "acme", None));
    let id = id.expect("a tenant");
    let (rekeyed, rekey) = counted(&catalogs, || tenants.rekey_tenant(Actor::Cli, id));
    rekeyed.expect("rotated");
    assert_eq!(
        (create, rekey),
        CREATE_AND_REKEY_TRIPS,
        "a tenant create and a tenant rekey, in round trips"
    );
}

#[test]
fn a_delete_with_its_purge_costs_the_round_trips_it_did_before() {
    let w = world();
    let (catalogs, tenants) = w.serve();
    let id = tenants.create(Actor::Cli, "acme", None).expect("a tenant");
    let caller = who();
    let uuid = store(&tenants, id, &caller, 1);
    let store_of = tenants.store(id).expect("the store");
    let (deleted, cost) = counted(&catalogs, || {
        store_of.delete(&caller, uuid, Retention::Days(0))
    });
    deleted.expect("deleted and purged");
    assert_eq!(
        cost, DELETE_AND_PURGE_TRIPS,
        "a delete and its purge, in round trips"
    );
}

#[test]
fn the_pack_jobs_cost_the_round_trips_they_did_before() {
    let w = packed_world();
    let (catalogs, tenants) = w.serve();
    let id = tenants.create(Actor::Cli, "acme", None).expect("a tenant");
    let caller = who();
    let plain = PackPlan {
        merge_below: 0,
        ..PackPlan::with_limit(1 << 20)
    };
    let merging = PackPlan::with_limit(1 << 20);
    let store_of = tenants.store(id).expect("the store");
    for n in 1..=2 {
        store(&tenants, id, &caller, n);
        store_of.pack(&plain).expect("packed");
    }
    assert_eq!(store_of.compact(&merging), Ok(1), "two files merged");
    let (published, publish) = counted(&catalogs, || store_of.publish());
    published.expect("published");
    let (removed, remove) = counted(&catalogs, || store_of.remove_expired(&merging));
    removed.expect("nothing removed inside the grace");
    let (pruned, prune) = counted(&catalogs, || {
        store_of.prune_manifests(Some(std::time::Duration::from_secs(900)))
    });
    pruned.expect("nothing pruned inside the grace");
    assert_eq!(
        (publish, remove, prune),
        PACK_JOB_TRIPS,
        "a manifest's publish, a removal of expired files and a pruning, in round trips"
    );
}

/// A key check that writes no last-used minute.
const KEY_CHECK_TRIPS: usize = 2;

/// A session's start, a request with it, its step-up and its end.
const SESSION_TRIPS: (usize, usize, usize, usize) = (10, 3, 4, 6);

/// A tenant's create (not the first), then its rekey.
const CREATE_AND_REKEY_TRIPS: (usize, usize) = (35, 72);

/// A delete under no retention, which purges the version at once.
const DELETE_AND_PURGE_TRIPS: usize = 23;

/// A publish of one manifest, a removal that finds the files inside the
/// grace, and a pruning that keeps the older manifest.
const PACK_JOB_TRIPS: (usize, usize, usize) = (7, 5, 2);
