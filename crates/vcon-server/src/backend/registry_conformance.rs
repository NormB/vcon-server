//! The registry conformance suite: what a caller of [`Registry`] and
//! [`RegistryTx`] can see, written once against the two traits and run on
//! every registry backend. A backend passes when every check does. Each
//! check takes a new, empty registry.
//!
//! The checks are named by kind: `ok_` for what must work, `no_` for what
//! must be refused or answer nothing, `edge_` for limits and odd inputs.
//! [`registry_conformance`] makes one test of each for a backend.
//!
//! One thing the suite does not hold a backend to: what a transaction can
//! still do after one of its writes failed. The local registry goes on; a
//! PostgreSQL transaction is refused until it ends. Every check drops a
//! transaction whose write failed.

use std::any::Any;

use time::OffsetDateTime;

use crate::audit::{Action, Actor, Entry, NO_ANSWER};
use crate::backend::{
    CommitError, KeyRow, MembershipRow, Registry, RegistryTx, SessionRow, TenantRow, UserRow,
};
use crate::ids::{KeyId, TenantId, UserId};
use crate::tenants::TenantError;

/// A new, empty registry, and what must outlive it (a directory, or a
/// database), dropped after it.
pub(crate) struct Fresh {
    pub(crate) registry: Box<dyn Registry>,
    pub(crate) _keep: Box<dyn Any>,
}

/// Makes one test of every check, each on the registry `$fresh()` gives.
macro_rules! registry_conformance {
    ($fresh:path) => {
        $crate::backend::registry_conformance::registry_conformance!(@each $fresh;
            ok_a_tenant_reads_back,
            ok_tenants_are_listed_by_name,
            ok_a_name_is_taken_once_inserted,
            ok_retention_metadata_and_role_cap_are_set_and_cleared,
            ok_a_pending_key_is_set_and_promoted,
            ok_master_wraps_are_counted_from_the_first,
            ok_a_key_reads_back,
            ok_keys_are_listed_by_creation_then_id,
            ok_keys_of_one_tenant_are_listed,
            ok_a_key_is_revoked_at_the_time_given,
            ok_a_key_is_touched_forward,
            ok_a_user_reads_back,
            ok_users_are_listed_oldest_first,
            ok_an_active_users_identity_is_taken,
            ok_a_user_is_deactivated_at_the_time_given,
            ok_a_membership_reads_back,
            ok_memberships_are_listed_by_user_then_time,
            ok_roles_are_set_on_one_membership,
            ok_memberships_are_deleted_one_by_user_and_by_tenant,
            ok_a_session_reads_back,
            ok_a_session_is_touched,
            ok_sessions_are_deleted_one_by_user_and_by_tenant,
            ok_ended_sessions_are_deleted,
            ok_settings_are_listed_by_name_and_replaced,
            ok_settings_are_deleted_one_and_by_tenant,
            ok_a_record_is_exported,
            ok_counts_in_one_minute_are_one_record,
            ok_count_many_adds_its_times,
            ok_record_times_are_read_by_action_and_status,
            ok_a_page_of_the_log_follows_a_seq_up_to_its_limit,
            ok_deleting_a_tenant_revokes_its_keys,
            ok_a_snapshot_reads_what_was_committed,
            no_tenant_has_an_unknown_id,
            no_name_is_taken_before_it_is_inserted,
            no_retention_is_set_on_an_unknown_tenant,
            no_metadata_is_set_on_an_unknown_tenant,
            no_role_cap_is_set_on_an_unknown_tenant,
            no_second_tenant_takes_a_taken_name,
            no_second_tenant_takes_a_taken_id,
            no_second_key_takes_a_taken_id,
            no_key_has_an_unknown_id,
            no_user_has_an_unknown_id,
            no_second_active_user_takes_an_identity,
            no_membership_is_held_by_an_unknown_pair,
            no_second_membership_of_a_user_in_a_tenant,
            no_second_operator_membership_of_a_user,
            no_session_has_an_unknown_hash,
            no_second_session_takes_a_hash,
            no_write_is_kept_when_dropped_without_a_commit,
            no_write_is_kept_when_the_commit_rolled_back,
            no_identity_is_taken_by_a_deactivated_user,
            no_unknown_key_is_touched,
            no_use_is_recorded_for_an_unused_master_key,
            edge_a_commit_of_unknown_outcome_may_have_kept_the_write,
            edge_a_commit_fault_counts_commits,
            edge_a_touch_to_the_same_or_an_earlier_time_changes_nothing,
            edge_a_second_revocation_keeps_the_first_time,
            edge_a_second_deactivation_keeps_the_first_time,
            edge_an_identity_is_free_again_after_deactivation,
            edge_a_tenant_with_nothing_optional_reads_back,
            edge_long_and_non_ascii_text_reads_back,
            edge_empty_text_reads_back,
            edge_a_count_of_nothing_makes_no_record,
            edge_counts_in_two_minutes_are_two_records,
            edge_a_record_with_no_target_and_no_answer_is_exported,
            edge_the_log_after_its_last_seq_is_empty,
            edge_a_session_ends_at_its_limit_exactly,
            edge_deleting_what_is_not_there_succeeds,
            edge_a_tenant_with_no_keys_lists_none,
            edge_deleting_a_tenant_keeps_an_earlier_revocation,
            edge_an_export_after_a_seq_leaves_out_the_older,
            edge_settings_of_two_tenants_are_apart,
            edge_an_empty_registry_lists_nothing,
        );
    };
    (@each $fresh:path; $($name:ident),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                let fresh = $fresh();
                $crate::backend::registry_conformance::$name(fresh.registry.as_ref());
            }
        )*
    };
}
pub(crate) use registry_conformance;

/// 2026-10-24T00:00:00Z, in seconds.
const T0: i64 = 1_792_756_800;

fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(T0 + seconds).expect("a time")
}

/// Runs `f` in a write and commits it.
fn write<T>(r: &dyn Registry, f: impl FnOnce(&mut dyn RegistryTx) -> T) -> T {
    let mut tx = r.immediate().expect("a write begins");
    let out = f(tx.as_mut());
    tx.commit().expect("the write commits");
    out
}

/// Runs `f` in a read.
fn read<T>(r: &dyn Registry, f: impl FnOnce(&mut dyn RegistryTx) -> T) -> T {
    let mut tx = r.snapshot().expect("a read begins");
    f(tx.as_mut())
}

fn tenant_id(n: u8) -> TenantId {
    TenantId([n; 16])
}

fn tenant(n: u8, name: &str) -> TenantRow {
    TenantRow {
        id: tenant_id(n),
        name: name.to_owned(),
        retention: Some("P30D".to_owned()),
        metadata: Some("parties".to_owned()),
        role_cap: Some("reader".to_owned()),
        created_at: T0 + i64::from(n),
        tenant_key_id: KeyId([n; 8]),
        wrapped: vec![n; 48],
        pending: Some(vec![n ^ 0xff; 48]),
        key_since: T0 + 5,
    }
}

fn key(n: u8, tenant: Option<TenantId>, created_at: i64) -> KeyRow {
    KeyRow {
        id: KeyId([n; 8]),
        tenant,
        token_hash: vec![n; 32],
        scopes: "vcons:read vcons:write".to_owned(),
        label: format!("key {n}"),
        created_at,
        revoked_at: None,
        expires_at: Some(created_at + 86_400),
        last_used_at: None,
    }
}

fn user(n: u8, identity: Option<(&str, &str)>, created_at: i64) -> UserRow {
    UserRow {
        id: UserId([n; 16]),
        label: format!("user {n}"),
        identity: identity.map(|(issuer, subject)| (issuer.to_owned(), subject.to_owned())),
        created_at,
        deactivated_at: None,
    }
}

fn membership(user: u8, tenant: Option<TenantId>, granted_at: i64) -> MembershipRow {
    MembershipRow {
        user: UserId([user; 16]),
        tenant,
        roles: "reader".to_owned(),
        granted_at,
    }
}

fn session(n: u8, user: u8, tenant: Option<TenantId>) -> SessionRow {
    SessionRow {
        id_hash: vec![n; 32],
        user: UserId([user; 16]),
        tenant,
        scopes: "vcons:read".to_owned(),
        created_at: T0,
        last_seen_at: T0 + 1,
        idle_expires_at: T0 + 900,
        expires_at: T0 + 3_600,
        idle_seconds: 900,
        amr: "pwd otp".to_owned(),
        csrf_hash: vec![n ^ 0xff; 32],
    }
}

fn entry(action: Action, target: Option<&str>, status: u16) -> Entry {
    Entry {
        actor: Actor::Cli,
        action,
        target: target.map(str::to_owned),
        version: None,
        status,
    }
}

/// The log after `after`, as the registry exports it.
fn exported(r: &dyn Registry, after: i64) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    r.export(after, &mut out).expect("the log exports");
    lines(&out)
}

fn lines(out: &[u8]) -> Vec<serde_json::Value> {
    std::str::from_utf8(out)
        .expect("UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("one JSON object on each line"))
        .collect()
}

/// A page of the log, as a transaction reads it.
fn page(r: &dyn Registry, after: i64, limit: Option<u32>) -> Vec<serde_json::Value> {
    read(r, |tx| {
        let mut out = Vec::new();
        tx.audit_lines(after, limit, &mut out)
            .expect("the page reads");
        lines(&out)
    })
}

fn seq(record: &serde_json::Value) -> i64 {
    record["seq"].as_i64().expect("a record has a seq")
}

// What must work.

pub(crate) fn ok_a_tenant_reads_back(r: &dyn Registry) {
    let row = tenant(1, "alpha");
    write(r, |tx| tx.insert_tenant(&row).expect("inserts"));
    let found = read(r, |tx| tx.tenant(row.id).expect("reads"));
    assert_eq!(found, Some(row), "every column reads back as inserted");
}

pub(crate) fn ok_tenants_are_listed_by_name(r: &dyn Registry) {
    write(r, |tx| {
        for (n, name) in [(1, "beta"), (2, "alpha"), (3, "Zulu")] {
            tx.insert_tenant(&tenant(n, name)).expect("inserts");
        }
    });
    let names: Vec<String> = read(r, |tx| tx.tenants().expect("lists"))
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(names, ["Zulu", "alpha", "beta"], "by name, in byte order");
}

pub(crate) fn ok_a_name_is_taken_once_inserted(r: &dyn Registry) {
    write(r, |tx| {
        tx.insert_tenant(&tenant(1, "alpha")).expect("inserts")
    });
    let taken = read(r, |tx| tx.name_taken("alpha").expect("reads"));
    assert!(taken, "the name of a tenant is taken");
}

pub(crate) fn ok_retention_metadata_and_role_cap_are_set_and_cleared(r: &dyn Registry) {
    let row = tenant(1, "alpha");
    write(r, |tx| tx.insert_tenant(&row).expect("inserts"));
    write(r, |tx| {
        assert!(
            tx.set_retention(row.id, Some("P7D")).expect("sets"),
            "retention: a row changed"
        );
        assert!(
            tx.set_metadata(row.id, None).expect("sets"),
            "metadata: a row changed"
        );
        assert!(
            tx.set_role_cap(row.id, Some("admin")).expect("sets"),
            "role cap: a row changed"
        );
    });
    let found = read(r, |tx| tx.tenant(row.id).expect("reads")).expect("the tenant");
    assert_eq!(found.retention.as_deref(), Some("P7D"), "the new retention");
    assert_eq!(found.metadata, None, "the metadata level was cleared");
    assert_eq!(found.role_cap.as_deref(), Some("admin"), "the new role cap");
    write(r, |tx| {
        tx.set_retention(row.id, None).expect("clears");
        tx.set_metadata(row.id, Some("all")).expect("sets");
        tx.set_role_cap(row.id, None).expect("clears");
    });
    let found = read(r, |tx| tx.tenant(row.id).expect("reads")).expect("the tenant");
    assert_eq!(
        (found.retention, found.metadata.as_deref(), found.role_cap),
        (None, Some("all"), None),
        "cleared, set and cleared"
    );
    assert_eq!(found.name, "alpha", "the other columns are as they were");
}

pub(crate) fn ok_a_pending_key_is_set_and_promoted(r: &dyn Registry) {
    let mut row = tenant(1, "alpha");
    row.pending = None;
    write(r, |tx| tx.insert_tenant(&row).expect("inserts"));
    write(r, |tx| tx.set_pending(row.id, &[7; 48]).expect("sets"));
    let found = read(r, |tx| tx.tenant(row.id).expect("reads")).expect("the tenant");
    assert_eq!(found.pending, Some(vec![7; 48]), "the pending key is held");
    assert_eq!(found.wrapped, row.wrapped, "the current key is unchanged");
    write(r, |tx| {
        tx.promote_pending(row.id, KeyId([9; 8]), T0 + 77)
            .expect("promotes")
    });
    let found = read(r, |tx| tx.tenant(row.id).expect("reads")).expect("the tenant");
    assert_eq!(
        found.wrapped,
        vec![7; 48],
        "the pending key is the current one"
    );
    assert_eq!(found.pending, None, "no key is pending");
    assert_eq!(found.tenant_key_id, KeyId([9; 8]), "the new key's id");
    assert_eq!(found.key_since, T0 + 77, "since the time given");
}

pub(crate) fn ok_master_wraps_are_counted_from_the_first(r: &dyn Registry) {
    let master = KeyId([4; 8]);
    write(r, |tx| tx.add_master_wrap(master, T0 + 1).expect("adds"));
    write(r, |tx| tx.add_master_wrap(master, T0 + 50).expect("adds"));
    write(r, |tx| {
        tx.add_master_wrap(KeyId([5; 8]), T0 + 60).expect("adds")
    });
    let (wraps, since) = read(r, |tx| {
        (
            tx.master_wraps(master).expect("reads"),
            tx.master_since(master).expect("reads"),
        )
    });
    assert_eq!(wraps, 2, "two wraps under this key, none of the other's");
    assert_eq!(since, Some(T0 + 1), "since the first wrap");
}

pub(crate) fn ok_a_key_reads_back(r: &dyn Registry) {
    let mut of_tenant = key(1, Some(tenant_id(1)), T0);
    of_tenant.revoked_at = Some(T0 + 9);
    of_tenant.last_used_at = Some(T0 + 60);
    let mut operator = key(2, None, T0 + 1);
    operator.expires_at = None;
    write(r, |tx| {
        tx.insert_key(&of_tenant).expect("inserts");
        tx.insert_key(&operator).expect("inserts");
    });
    let found = read(r, |tx| {
        (
            tx.key(of_tenant.id).expect("reads"),
            tx.key(operator.id).expect("reads"),
        )
    });
    assert_eq!(
        found,
        (Some(of_tenant), Some(operator)),
        "every column reads back"
    );
}

pub(crate) fn ok_keys_are_listed_by_creation_then_id(r: &dyn Registry) {
    write(r, |tx| {
        tx.insert_key(&key(3, None, T0 + 5)).expect("inserts");
        tx.insert_key(&key(2, Some(tenant_id(1)), T0 + 5))
            .expect("inserts");
        tx.insert_key(&key(9, None, T0)).expect("inserts");
    });
    let ids: Vec<KeyId> = read(r, |tx| tx.keys(None).expect("lists"))
        .into_iter()
        .map(|k| k.id)
        .collect();
    assert_eq!(
        ids,
        [KeyId([9; 8]), KeyId([2; 8]), KeyId([3; 8])],
        "oldest first, and by id at one time"
    );
}

pub(crate) fn ok_keys_of_one_tenant_are_listed(r: &dyn Registry) {
    write(r, |tx| {
        tx.insert_key(&key(1, Some(tenant_id(1)), T0))
            .expect("inserts");
        tx.insert_key(&key(2, Some(tenant_id(2)), T0 + 1))
            .expect("inserts");
        tx.insert_key(&key(3, None, T0 + 2)).expect("inserts");
        tx.insert_key(&key(4, Some(tenant_id(1)), T0 + 3))
            .expect("inserts");
    });
    let ids: Vec<KeyId> = read(r, |tx| tx.keys(Some(tenant_id(1))).expect("lists"))
        .into_iter()
        .map(|k| k.id)
        .collect();
    assert_eq!(
        ids,
        [KeyId([1; 8]), KeyId([4; 8])],
        "the tenant's keys and no other"
    );
}

pub(crate) fn ok_a_key_is_revoked_at_the_time_given(r: &dyn Registry) {
    let row = key(1, None, T0);
    write(r, |tx| tx.insert_key(&row).expect("inserts"));
    write(r, |tx| tx.revoke_key(row.id, T0 + 30).expect("revokes"));
    let found = read(r, |tx| tx.key(row.id).expect("reads")).expect("the key");
    assert_eq!(found.revoked_at, Some(T0 + 30), "revoked at the time given");
    assert_eq!(found.label, row.label, "the other columns are as they were");
}

pub(crate) fn ok_a_key_is_touched_forward(r: &dyn Registry) {
    let row = key(1, None, T0);
    write(r, |tx| tx.insert_key(&row).expect("inserts"));
    let first = write(r, |tx| tx.touch_key(row.id, T0 + 60).expect("touches"));
    let later = write(r, |tx| tx.touch_key(row.id, T0 + 120).expect("touches"));
    assert!(first, "a key no request used is touched");
    assert!(later, "a later time moves it");
    let found = read(r, |tx| tx.key(row.id).expect("reads")).expect("the key");
    assert_eq!(found.last_used_at, Some(T0 + 120), "the later time is held");
}

pub(crate) fn ok_a_user_reads_back(r: &dyn Registry) {
    let mut named = user(1, Some(("https://idp.example.com", "subject-1")), T0);
    named.deactivated_at = Some(T0 + 3);
    let local = user(2, None, T0 + 1);
    write(r, |tx| {
        tx.insert_user(&named).expect("inserts");
        tx.insert_user(&local).expect("inserts");
    });
    let found = read(r, |tx| {
        (
            tx.user(named.id).expect("reads"),
            tx.user(local.id).expect("reads"),
        )
    });
    assert_eq!(found, (Some(named), Some(local)), "every column reads back");
}

pub(crate) fn ok_users_are_listed_oldest_first(r: &dyn Registry) {
    write(r, |tx| {
        tx.insert_user(&user(3, None, T0 + 5)).expect("inserts");
        tx.insert_user(&user(2, None, T0 + 5)).expect("inserts");
        tx.insert_user(&user(9, None, T0)).expect("inserts");
    });
    let ids: Vec<UserId> = read(r, |tx| tx.users().expect("lists"))
        .into_iter()
        .map(|u| u.id)
        .collect();
    assert_eq!(
        ids,
        [UserId([9; 16]), UserId([2; 16]), UserId([3; 16])],
        "oldest first, and by id at one time"
    );
}

pub(crate) fn ok_an_active_users_identity_is_taken(r: &dyn Registry) {
    write(r, |tx| {
        tx.insert_user(&user(1, Some(("https://idp.example.com", "subject-1")), T0))
            .expect("inserts");
    });
    let (taken, other_subject, other_issuer) = read(r, |tx| {
        (
            tx.identity_taken("https://idp.example.com", "subject-1")
                .expect("reads"),
            tx.identity_taken("https://idp.example.com", "subject-2")
                .expect("reads"),
            tx.identity_taken("https://idp.example.net", "subject-1")
                .expect("reads"),
        )
    });
    assert!(taken, "the user's identity is taken");
    assert!(!other_subject, "another subject of the issuer is free");
    assert!(!other_issuer, "the subject at another issuer is free");
}

pub(crate) fn ok_a_user_is_deactivated_at_the_time_given(r: &dyn Registry) {
    let row = user(1, Some(("https://idp.example.com", "subject-1")), T0);
    write(r, |tx| tx.insert_user(&row).expect("inserts"));
    write(r, |tx| {
        tx.deactivate_user(row.id, T0 + 40).expect("deactivates")
    });
    let found = read(r, |tx| tx.user(row.id).expect("reads")).expect("the user");
    assert_eq!(
        found.deactivated_at,
        Some(T0 + 40),
        "deactivated at the time given"
    );
    assert_eq!(found.identity, row.identity, "the row keeps its identity");
}

pub(crate) fn ok_a_membership_reads_back(r: &dyn Registry) {
    let in_tenant = membership(1, Some(tenant_id(1)), T0);
    let mut operator = membership(1, None, T0 + 1);
    operator.roles = "operator".to_owned();
    write(r, |tx| {
        tx.insert_membership(&in_tenant).expect("inserts");
        tx.insert_membership(&operator).expect("inserts");
    });
    let found = read(r, |tx| {
        (
            tx.membership(in_tenant.user, in_tenant.tenant)
                .expect("reads"),
            tx.membership(operator.user, None).expect("reads"),
        )
    });
    assert_eq!(
        found,
        (Some(in_tenant), Some(operator)),
        "each reads back by its pair"
    );
}

pub(crate) fn ok_memberships_are_listed_by_user_then_time(r: &dyn Registry) {
    write(r, |tx| {
        tx.insert_membership(&membership(2, Some(tenant_id(1)), T0))
            .expect("inserts");
        tx.insert_membership(&membership(1, Some(tenant_id(2)), T0 + 9))
            .expect("inserts");
        tx.insert_membership(&membership(1, Some(tenant_id(3)), T0 + 4))
            .expect("inserts");
        tx.insert_membership(&membership(1, Some(tenant_id(1)), T0 + 4))
            .expect("inserts");
        tx.insert_membership(&membership(1, None, T0 + 4))
            .expect("inserts");
    });
    let listed: Vec<(UserId, Option<TenantId>)> = read(r, |tx| tx.memberships().expect("lists"))
        .into_iter()
        .map(|m| (m.user, m.tenant))
        .collect();
    assert_eq!(
        listed,
        [
            (UserId([1; 16]), None),
            (UserId([1; 16]), Some(tenant_id(1))),
            (UserId([1; 16]), Some(tenant_id(3))),
            (UserId([1; 16]), Some(tenant_id(2))),
            (UserId([2; 16]), Some(tenant_id(1))),
        ],
        "by user, then time, then tenant with the operator's first"
    );
}

pub(crate) fn ok_roles_are_set_on_one_membership(r: &dyn Registry) {
    write(r, |tx| {
        tx.insert_membership(&membership(1, Some(tenant_id(1)), T0))
            .expect("inserts");
        tx.insert_membership(&membership(1, Some(tenant_id(2)), T0))
            .expect("inserts");
        tx.insert_membership(&membership(1, None, T0))
            .expect("inserts");
    });
    let mut changed = membership(1, Some(tenant_id(1)), T0 + 99);
    changed.roles = "admin".to_owned();
    let mut operator = membership(1, None, T0 + 99);
    operator.roles = "operator".to_owned();
    write(r, |tx| {
        tx.set_roles(&changed).expect("sets");
        tx.set_roles(&operator).expect("sets");
    });
    let roles: Vec<(Option<TenantId>, String, i64)> =
        read(r, |tx| tx.memberships().expect("lists"))
            .into_iter()
            .map(|m| (m.tenant, m.roles, m.granted_at))
            .collect();
    assert_eq!(
        roles,
        [
            (None, "operator".to_owned(), T0),
            (Some(tenant_id(1)), "admin".to_owned(), T0),
            (Some(tenant_id(2)), "reader".to_owned(), T0),
        ],
        "the two named memberships changed, and only their roles"
    );
}

pub(crate) fn ok_memberships_are_deleted_one_by_user_and_by_tenant(r: &dyn Registry) {
    write(r, |tx| {
        for (u, t) in [
            (1, Some(1)),
            (1, Some(2)),
            (1, None),
            (2, Some(1)),
            (2, Some(3)),
            (3, Some(3)),
        ] {
            tx.insert_membership(&membership(u, t.map(tenant_id), T0))
                .expect("inserts");
        }
    });
    let left = |r: &dyn Registry| -> Vec<(UserId, Option<TenantId>)> {
        read(r, |tx| tx.memberships().expect("lists"))
            .into_iter()
            .map(|m| (m.user, m.tenant))
            .collect()
    };
    write(r, |tx| {
        tx.delete_membership(UserId([1; 16]), None)
            .expect("deletes")
    });
    assert_eq!(left(r).len(), 5, "one membership went");
    assert!(
        !left(r).contains(&(UserId([1; 16]), None)),
        "the operator's"
    );
    write(r, |tx| {
        tx.delete_membership(UserId([1; 16]), Some(tenant_id(2)))
            .expect("deletes")
    });
    assert_eq!(left(r).len(), 4, "one more went");
    write(r, |tx| {
        tx.delete_memberships_in(tenant_id(1)).expect("deletes")
    });
    assert_eq!(
        left(r),
        [
            (UserId([2; 16]), Some(tenant_id(3))),
            (UserId([3; 16]), Some(tenant_id(3)))
        ],
        "every membership in the tenant went"
    );
    write(r, |tx| {
        tx.delete_memberships_of_user(UserId([2; 16]))
            .expect("deletes")
    });
    assert_eq!(
        left(r),
        [(UserId([3; 16]), Some(tenant_id(3)))],
        "every one of the user went"
    );
}

pub(crate) fn ok_a_session_reads_back(r: &dyn Registry) {
    let in_tenant = session(1, 1, Some(tenant_id(1)));
    let operator = session(2, 1, None);
    write(r, |tx| {
        tx.insert_session(&in_tenant).expect("inserts");
        tx.insert_session(&operator).expect("inserts");
    });
    let found = read(r, |tx| {
        (
            tx.session(&in_tenant.id_hash).expect("reads"),
            tx.session(&operator.id_hash).expect("reads"),
        )
    });
    assert_eq!(
        found,
        (Some(in_tenant), Some(operator)),
        "every column reads back"
    );
}

pub(crate) fn ok_a_session_is_touched(r: &dyn Registry) {
    let row = session(1, 1, None);
    write(r, |tx| {
        tx.insert_session(&row).expect("inserts");
        tx.insert_session(&session(2, 1, None)).expect("inserts");
    });
    write(r, |tx| {
        tx.touch_session(&row.id_hash, T0 + 500, T0 + 1_400)
            .expect("touches")
    });
    let (found, other) = read(r, |tx| {
        (
            tx.session(&row.id_hash)
                .expect("reads")
                .expect("the session"),
            tx.session(&[2; 32]).expect("reads").expect("the other"),
        )
    });
    assert_eq!(
        (found.last_seen_at, found.idle_expires_at, found.expires_at),
        (T0 + 500, T0 + 1_400, row.expires_at),
        "its two times moved and its last limit did not"
    );
    assert_eq!(other.last_seen_at, T0 + 1, "another session is as it was");
}

pub(crate) fn ok_sessions_are_deleted_one_by_user_and_by_tenant(r: &dyn Registry) {
    write(r, |tx| {
        tx.insert_session(&session(1, 1, Some(tenant_id(1))))
            .expect("inserts");
        tx.insert_session(&session(2, 1, None)).expect("inserts");
        tx.insert_session(&session(3, 2, Some(tenant_id(1))))
            .expect("inserts");
        tx.insert_session(&session(4, 2, Some(tenant_id(2))))
            .expect("inserts");
        tx.insert_session(&session(5, 3, Some(tenant_id(2))))
            .expect("inserts");
        tx.insert_session(&session(6, 3, None)).expect("inserts");
    });
    let held = |r: &dyn Registry| -> Vec<u8> {
        read(r, |tx| {
            (1..=6)
                .filter(|n| tx.session(&[*n; 32]).expect("reads").is_some())
                .collect()
        })
    };
    write(r, |tx| tx.delete_session(&[6; 32]).expect("deletes"));
    assert_eq!(held(r), [1, 2, 3, 4, 5], "one session went");
    write(r, |tx| {
        tx.delete_sessions_in(tenant_id(1)).expect("deletes")
    });
    assert_eq!(
        held(r),
        [2, 4, 5],
        "the tenant's went, an operator's did not"
    );
    write(r, |tx| {
        tx.delete_sessions_of_user(UserId([2; 16]))
            .expect("deletes")
    });
    assert_eq!(held(r), [2, 5], "the user's went");
    write(r, |tx| {
        tx.delete_sessions_of_user(UserId([1; 16]))
            .expect("deletes")
    });
    assert_eq!(
        held(r),
        [5],
        "a user's operator session goes with its others"
    );
}

pub(crate) fn ok_ended_sessions_are_deleted(r: &dyn Registry) {
    let mut idle = session(1, 1, None);
    idle.idle_expires_at = T0 + 100;
    let mut whole = session(2, 1, None);
    whole.expires_at = T0 + 150;
    let live = session(3, 1, None);
    write(r, |tx| {
        for row in [&idle, &whole, &live] {
            tx.insert_session(row).expect("inserts");
        }
    });
    write(r, |tx| tx.delete_sessions_ended(T0 + 200).expect("deletes"));
    let held: Vec<bool> = read(r, |tx| {
        (1..=3)
            .map(|n: u8| tx.session(&[n; 32]).expect("reads").is_some())
            .collect()
    });
    assert_eq!(
        held,
        [false, false, true],
        "either limit passed ends a session"
    );
}

pub(crate) fn ok_settings_are_listed_by_name_and_replaced(r: &dyn Registry) {
    let id = tenant_id(1);
    write(r, |tx| {
        tx.put_setting(id, "purge.grace", "P7D").expect("puts");
        tx.put_setting(id, "audit.reads", "on").expect("puts");
        tx.put_setting(id, "Zeta", "z").expect("puts");
    });
    write(r, |tx| {
        tx.put_setting(id, "purge.grace", "P9D").expect("puts")
    });
    let found = read(r, |tx| tx.settings(id).expect("lists"));
    assert_eq!(
        found,
        [
            ("Zeta".to_owned(), "z".to_owned()),
            ("audit.reads".to_owned(), "on".to_owned()),
            ("purge.grace".to_owned(), "P9D".to_owned()),
        ],
        "by name in byte order, each with its last value"
    );
}

pub(crate) fn ok_settings_are_deleted_one_and_by_tenant(r: &dyn Registry) {
    let id = tenant_id(1);
    write(r, |tx| {
        for name in ["a", "b", "c"] {
            tx.put_setting(id, name, "1").expect("puts");
        }
    });
    write(r, |tx| tx.delete_setting(id, "b").expect("deletes"));
    let names: Vec<String> = read(r, |tx| tx.settings(id).expect("lists"))
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names, ["a", "c"], "one setting went");
    write(r, |tx| tx.delete_settings_in(id).expect("deletes"));
    assert!(
        read(r, |tx| tx.settings(id).expect("lists")).is_empty(),
        "every setting went"
    );
}

pub(crate) fn ok_a_record_is_exported(r: &dyn Registry) {
    let target = tenant_id(1).to_string();
    write(r, |tx| {
        tx.record(&entry(Action::Create, Some(&target), 201), at(3))
            .expect("records");
    });
    let records = exported(r, 0);
    assert_eq!(records.len(), 1, "one record");
    let record = &records[0];
    assert_eq!(record["actor"], "cli", "the actor");
    assert_eq!(record["action"], Action::Create.as_str(), "the action");
    assert_eq!(record["target"], target.as_str(), "the target, readable");
    assert_eq!(record["status"], 201, "the status");
    assert_eq!(record["count"], 1, "counted once");
    assert_eq!(
        record["at"],
        crate::audit::rfc3339(at(3)).as_str(),
        "at the time given"
    );
    assert!(record["version"].is_null(), "no version");
}

pub(crate) fn ok_counts_in_one_minute_are_one_record(r: &dyn Registry) {
    let counted = entry(Action::Read, Some("a"), 401);
    write(r, |tx| {
        tx.count(&counted, at(1)).expect("counts");
        tx.count(&counted, at(20)).expect("counts");
    });
    write(r, |tx| {
        tx.count(&counted, at(59)).expect("counts");
        tx.count(&entry(Action::Read, Some("b"), 401), at(30))
            .expect("counts");
        tx.count(&entry(Action::Read, Some("a"), 403), at(30))
            .expect("counts");
    });
    let counts: Vec<(String, i64, i64)> = exported(r, 0)
        .iter()
        .map(|rec| {
            (
                rec["target"].as_str().expect("a target").to_owned(),
                rec["status"].as_i64().expect("a status"),
                rec["count"].as_i64().expect("a count"),
            )
        })
        .collect();
    assert_eq!(
        counts,
        [
            ("a".to_owned(), 401, 3),
            ("b".to_owned(), 401, 1),
            ("a".to_owned(), 403, 1)
        ],
        "one record for each target and status in the minute"
    );
    let record = &exported(r, 0)[0];
    assert_eq!(
        record["at"],
        crate::audit::rfc3339(at(0)).as_str(),
        "at the minute's start"
    );
}

pub(crate) fn ok_count_many_adds_its_times(r: &dyn Registry) {
    let counted = entry(Action::Read, None, 429);
    write(r, |tx| tx.count_many(&counted, at(5), 40).expect("counts"));
    write(r, |tx| {
        tx.count_many(&counted, at(6), 2).expect("counts");
        tx.count(&counted, at(7)).expect("counts");
    });
    let records = exported(r, 0);
    assert_eq!(records.len(), 1, "one record for the minute");
    assert_eq!(records[0]["count"], 43, "40, then 2, then 1");
}

pub(crate) fn ok_record_times_are_read_by_action_and_status(r: &dyn Registry) {
    write(r, |tx| {
        tx.record(&entry(Action::Purge, None, 200), at(1))
            .expect("records");
        tx.record(&entry(Action::Purge, None, 500), at(2))
            .expect("records");
        tx.record(&entry(Action::Delete, None, 200), at(3))
            .expect("records");
        tx.record(&entry(Action::Purge, Some("x"), 200), at(4))
            .expect("records");
    });
    let mut times = read(r, |tx| tx.record_times(Action::Purge, 200).expect("reads"));
    times.sort();
    assert_eq!(
        times,
        [crate::audit::rfc3339(at(1)), crate::audit::rfc3339(at(4))],
        "the times of the records with the action and the status"
    );
}

pub(crate) fn ok_a_page_of_the_log_follows_a_seq_up_to_its_limit(r: &dyn Registry) {
    write(r, |tx| {
        for n in 0..5_u8 {
            tx.record(
                &entry(Action::Create, Some(&n.to_string()), 201),
                at(i64::from(n)),
            )
            .expect("records");
        }
    });
    let all = page(r, 0, None);
    assert_eq!(all.len(), 5, "with no limit, every record");
    let targets = |records: &[serde_json::Value]| -> Vec<String> {
        records
            .iter()
            .map(|rec| rec["target"].as_str().expect("a target").to_owned())
            .collect()
    };
    assert_eq!(targets(&all), ["0", "1", "2", "3", "4"], "oldest first");
    let some = page(r, seq(&all[1]), Some(2));
    assert_eq!(targets(&some), ["2", "3"], "the two after the seq given");
}

pub(crate) fn ok_deleting_a_tenant_revokes_its_keys(r: &dyn Registry) {
    write(r, |tx| {
        tx.insert_tenant(&tenant(1, "alpha")).expect("inserts");
        tx.insert_tenant(&tenant(2, "beta")).expect("inserts");
        tx.insert_key(&key(1, Some(tenant_id(1)), T0))
            .expect("inserts");
        tx.insert_key(&key(2, Some(tenant_id(2)), T0))
            .expect("inserts");
        tx.insert_key(&key(3, None, T0)).expect("inserts");
    });
    write(r, |tx| {
        tx.delete_tenant(tenant_id(1), T0 + 70).expect("deletes")
    });
    let (gone, kept, revoked) = read(r, |tx| {
        (
            tx.tenant(tenant_id(1)).expect("reads"),
            tx.tenant(tenant_id(2)).expect("reads").is_some(),
            tx.keys(None)
                .expect("lists")
                .into_iter()
                .map(|k| k.revoked_at)
                .collect::<Vec<_>>(),
        )
    });
    assert_eq!(gone, None, "the tenant's row is gone");
    assert!(kept, "another tenant's row is kept");
    assert_eq!(
        revoked,
        [Some(T0 + 70), None, None],
        "only the tenant's key is revoked"
    );
    assert!(
        !read(r, |tx| tx.name_taken("alpha").expect("reads")),
        "its name is free"
    );
}

pub(crate) fn ok_a_snapshot_reads_what_was_committed(r: &dyn Registry) {
    write(r, |tx| {
        tx.insert_tenant(&tenant(1, "alpha")).expect("inserts")
    });
    let mut snapshot = r.snapshot().expect("a read begins");
    let first = snapshot.tenants().expect("lists").len();
    let again = snapshot.tenants().expect("lists").len();
    snapshot.commit().expect("a read ends");
    assert_eq!((first, again), (1, 1), "the committed row, each time");
}

// What must be refused, or answer nothing.

pub(crate) fn no_tenant_has_an_unknown_id(r: &dyn Registry) {
    write(r, |tx| {
        tx.insert_tenant(&tenant(1, "alpha")).expect("inserts")
    });
    let found = read(r, |tx| tx.tenant(tenant_id(2)).expect("reads"));
    assert_eq!(found, None, "no row for an id no tenant has");
}

pub(crate) fn no_name_is_taken_before_it_is_inserted(r: &dyn Registry) {
    write(r, |tx| {
        tx.insert_tenant(&tenant(1, "alpha")).expect("inserts")
    });
    let (other, prefix, case) = read(r, |tx| {
        (
            tx.name_taken("beta").expect("reads"),
            tx.name_taken("alph").expect("reads"),
            tx.name_taken("Alpha").expect("reads"),
        )
    });
    assert!(!other, "a name no tenant has");
    assert!(!prefix, "the start of a name is another name");
    assert!(!case, "a name in another case is another name");
}

pub(crate) fn no_retention_is_set_on_an_unknown_tenant(r: &dyn Registry) {
    let set = write(r, |tx| {
        tx.set_retention(tenant_id(1), Some("P7D"))
            .expect("answers")
    });
    assert!(!set, "no row changed");
}

pub(crate) fn no_metadata_is_set_on_an_unknown_tenant(r: &dyn Registry) {
    let set = write(r, |tx| {
        tx.set_metadata(tenant_id(1), Some("all")).expect("answers")
    });
    assert!(!set, "no row changed");
}

pub(crate) fn no_role_cap_is_set_on_an_unknown_tenant(r: &dyn Registry) {
    let set = write(r, |tx| {
        tx.set_role_cap(tenant_id(1), Some("reader"))
            .expect("answers")
    });
    assert!(!set, "no row changed");
}

/// Runs a write that must fail in a transaction of its own, and drops it.
fn refused(r: &dyn Registry, f: impl FnOnce(&mut dyn RegistryTx) -> Result<(), TenantError>) {
    let mut tx = r.immediate().expect("a write begins");
    let outcome = f(tx.as_mut());
    assert_eq!(outcome, Err(TenantError::Io), "the write is refused");
}

pub(crate) fn no_second_tenant_takes_a_taken_name(r: &dyn Registry) {
    write(r, |tx| {
        tx.insert_tenant(&tenant(1, "alpha")).expect("inserts")
    });
    refused(r, |tx| tx.insert_tenant(&tenant(2, "alpha")));
    let ids: Vec<TenantId> = read(r, |tx| tx.tenants().expect("lists"))
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(ids, [tenant_id(1)], "only the first tenant holds the name");
}

pub(crate) fn no_second_tenant_takes_a_taken_id(r: &dyn Registry) {
    write(r, |tx| {
        tx.insert_tenant(&tenant(1, "alpha")).expect("inserts")
    });
    refused(r, |tx| tx.insert_tenant(&tenant(1, "beta")));
    let found = read(r, |tx| tx.tenant(tenant_id(1)).expect("reads")).expect("the tenant");
    assert_eq!(found.name, "alpha", "the first row is as it was");
}

pub(crate) fn no_second_key_takes_a_taken_id(r: &dyn Registry) {
    write(r, |tx| tx.insert_key(&key(1, None, T0)).expect("inserts"));
    refused(r, |tx| tx.insert_key(&key(1, Some(tenant_id(1)), T0 + 1)));
    let found = read(r, |tx| tx.keys(None).expect("lists"));
    assert_eq!(found, [key(1, None, T0)], "the first key is as it was");
}

pub(crate) fn no_key_has_an_unknown_id(r: &dyn Registry) {
    write(r, |tx| tx.insert_key(&key(1, None, T0)).expect("inserts"));
    let found = read(r, |tx| tx.key(KeyId([2; 8])).expect("reads"));
    assert_eq!(found, None, "no row for an id no key has");
}

pub(crate) fn no_user_has_an_unknown_id(r: &dyn Registry) {
    write(r, |tx| tx.insert_user(&user(1, None, T0)).expect("inserts"));
    let found = read(r, |tx| tx.user(UserId([2; 16])).expect("reads"));
    assert_eq!(found, None, "no row for an id no user has");
}

pub(crate) fn no_second_active_user_takes_an_identity(r: &dyn Registry) {
    let identity = Some(("https://idp.example.com", "subject-1"));
    write(r, |tx| {
        tx.insert_user(&user(1, identity, T0)).expect("inserts")
    });
    refused(r, |tx| tx.insert_user(&user(2, identity, T0 + 1)));
    let ids: Vec<UserId> = read(r, |tx| tx.users().expect("lists"))
        .into_iter()
        .map(|u| u.id)
        .collect();
    assert_eq!(
        ids,
        [UserId([1; 16])],
        "only the first user holds the identity"
    );
}

pub(crate) fn no_membership_is_held_by_an_unknown_pair(r: &dyn Registry) {
    write(r, |tx| {
        tx.insert_membership(&membership(1, Some(tenant_id(1)), T0))
            .expect("inserts");
    });
    let (other_tenant, operator, other_user) = read(r, |tx| {
        (
            tx.membership(UserId([1; 16]), Some(tenant_id(2)))
                .expect("reads"),
            tx.membership(UserId([1; 16]), None).expect("reads"),
            tx.membership(UserId([2; 16]), Some(tenant_id(1)))
                .expect("reads"),
        )
    });
    assert_eq!(other_tenant, None, "none in another tenant");
    assert_eq!(operator, None, "a tenant's membership is not an operator's");
    assert_eq!(other_user, None, "none of another user");
}

pub(crate) fn no_second_membership_of_a_user_in_a_tenant(r: &dyn Registry) {
    write(r, |tx| {
        tx.insert_membership(&membership(1, Some(tenant_id(1)), T0))
            .expect("inserts");
    });
    refused(r, |tx| {
        tx.insert_membership(&membership(1, Some(tenant_id(1)), T0 + 1))
    });
    let found = read(r, |tx| tx.memberships().expect("lists"));
    assert_eq!(
        found,
        [membership(1, Some(tenant_id(1)), T0)],
        "the first is as it was"
    );
}

pub(crate) fn no_second_operator_membership_of_a_user(r: &dyn Registry) {
    write(r, |tx| {
        tx.insert_membership(&membership(1, None, T0))
            .expect("inserts")
    });
    refused(r, |tx| tx.insert_membership(&membership(1, None, T0 + 1)));
    let found = read(r, |tx| tx.memberships().expect("lists"));
    assert_eq!(found, [membership(1, None, T0)], "the first is as it was");
}

pub(crate) fn no_session_has_an_unknown_hash(r: &dyn Registry) {
    write(r, |tx| {
        tx.insert_session(&session(1, 1, None)).expect("inserts")
    });
    let found = read(r, |tx| tx.session(&[2; 32]).expect("reads"));
    assert_eq!(found, None, "no row for a hash no session has");
}

pub(crate) fn no_second_session_takes_a_hash(r: &dyn Registry) {
    write(r, |tx| {
        tx.insert_session(&session(1, 1, None)).expect("inserts")
    });
    refused(r, |tx| {
        tx.insert_session(&session(1, 2, Some(tenant_id(1))))
    });
    let found = read(r, |tx| tx.session(&[1; 32]).expect("reads"));
    assert_eq!(
        found,
        Some(session(1, 1, None)),
        "the first session is as it was"
    );
}

pub(crate) fn no_write_is_kept_when_dropped_without_a_commit(r: &dyn Registry) {
    {
        let mut tx = r.immediate().expect("a write begins");
        tx.insert_tenant(&tenant(1, "alpha")).expect("inserts");
        tx.insert_key(&key(1, None, T0)).expect("inserts");
        tx.record(&entry(Action::Create, None, 201), at(0))
            .expect("records");
    }
    let (tenants, keys) = read(r, |tx| {
        (
            tx.tenants().expect("lists").len(),
            tx.keys(None).expect("lists").len(),
        )
    });
    assert_eq!(
        (tenants, keys),
        (0, 0),
        "nothing of the transaction is kept"
    );
    assert!(exported(r, 0).is_empty(), "nor its record");
    write(r, |tx| {
        tx.insert_tenant(&tenant(1, "alpha"))
            .expect("the next write inserts")
    });
}

pub(crate) fn no_write_is_kept_when_the_commit_rolled_back(r: &dyn Registry) {
    r.fail_commit(1, CommitError::RolledBack);
    let mut tx = r.immediate().expect("a write begins");
    tx.insert_tenant(&tenant(1, "alpha")).expect("inserts");
    assert_eq!(
        tx.commit(),
        Err(CommitError::RolledBack),
        "the commit says it rolled back"
    );
    let found = read(r, |tx| tx.tenant(tenant_id(1)).expect("reads"));
    assert_eq!(found, None, "the row is not kept");
}

pub(crate) fn no_identity_is_taken_by_a_deactivated_user(r: &dyn Registry) {
    let mut row = user(1, Some(("https://idp.example.com", "subject-1")), T0);
    row.deactivated_at = Some(T0 + 1);
    write(r, |tx| tx.insert_user(&row).expect("inserts"));
    let taken = read(r, |tx| {
        tx.identity_taken("https://idp.example.com", "subject-1")
            .expect("reads")
    });
    assert!(!taken, "a deactivated user no longer holds its identity");
}

pub(crate) fn no_unknown_key_is_touched(r: &dyn Registry) {
    let touched = write(r, |tx| tx.touch_key(KeyId([1; 8]), T0).expect("answers"));
    assert!(!touched, "no row changed");
}

pub(crate) fn no_use_is_recorded_for_an_unused_master_key(r: &dyn Registry) {
    write(r, |tx| tx.add_master_wrap(KeyId([1; 8]), T0).expect("adds"));
    let (wraps, since) = read(r, |tx| {
        (
            tx.master_wraps(KeyId([2; 8])).expect("reads"),
            tx.master_since(KeyId([2; 8])).expect("reads"),
        )
    });
    assert_eq!(wraps, 0, "no wraps under a key not used");
    assert_eq!(since, None, "and no start of use");
}

// Limits and odd inputs.

pub(crate) fn edge_a_commit_of_unknown_outcome_may_have_kept_the_write(r: &dyn Registry) {
    r.fail_commit(1, CommitError::Unknown);
    let mut tx = r.immediate().expect("a write begins");
    tx.insert_tenant(&tenant(1, "alpha")).expect("inserts");
    assert_eq!(
        tx.commit(),
        Err(CommitError::Unknown),
        "the commit does not know"
    );
    let found = read(r, |tx| tx.tenant(tenant_id(1)).expect("reads"));
    assert_eq!(
        found,
        Some(tenant(1, "alpha")),
        "the injected fault kept the row"
    );
}

pub(crate) fn edge_a_commit_fault_counts_commits(r: &dyn Registry) {
    r.fail_commit(2, CommitError::RolledBack);
    write(r, |tx| {
        tx.insert_tenant(&tenant(1, "alpha")).expect("inserts")
    });
    let mut tx = r.immediate().expect("a write begins");
    tx.insert_tenant(&tenant(2, "beta")).expect("inserts");
    assert_eq!(
        tx.commit(),
        Err(CommitError::RolledBack),
        "the second commit fails"
    );
    write(r, |tx| {
        tx.insert_tenant(&tenant(3, "gamma")).expect("inserts")
    });
    let ids: Vec<TenantId> = read(r, |tx| tx.tenants().expect("lists"))
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(
        ids,
        [tenant_id(1), tenant_id(3)],
        "the first and the third were kept"
    );
}

pub(crate) fn edge_a_touch_to_the_same_or_an_earlier_time_changes_nothing(r: &dyn Registry) {
    let mut row = key(1, None, T0);
    row.last_used_at = Some(T0 + 120);
    write(r, |tx| tx.insert_key(&row).expect("inserts"));
    let same = write(r, |tx| tx.touch_key(row.id, T0 + 120).expect("answers"));
    let earlier = write(r, |tx| tx.touch_key(row.id, T0 + 60).expect("answers"));
    assert!(!same, "the same time changes no row");
    assert!(!earlier, "an earlier time changes no row");
    let found = read(r, |tx| tx.key(row.id).expect("reads")).expect("the key");
    assert_eq!(
        found.last_used_at,
        Some(T0 + 120),
        "the time held is the latest"
    );
}

pub(crate) fn edge_a_second_revocation_keeps_the_first_time(r: &dyn Registry) {
    let row = key(1, None, T0);
    write(r, |tx| tx.insert_key(&row).expect("inserts"));
    write(r, |tx| tx.revoke_key(row.id, T0 + 30).expect("revokes"));
    write(r, |tx| tx.revoke_key(row.id, T0 + 90).expect("answers"));
    write(r, |tx| tx.revoke_key(row.id, T0 + 10).expect("answers"));
    let found = read(r, |tx| tx.key(row.id).expect("reads")).expect("the key");
    assert_eq!(
        found.revoked_at,
        Some(T0 + 30),
        "the first revocation's time"
    );
}

pub(crate) fn edge_a_second_deactivation_keeps_the_first_time(r: &dyn Registry) {
    let row = user(1, None, T0);
    write(r, |tx| tx.insert_user(&row).expect("inserts"));
    write(r, |tx| {
        tx.deactivate_user(row.id, T0 + 30).expect("deactivates")
    });
    write(r, |tx| {
        tx.deactivate_user(row.id, T0 + 90).expect("answers")
    });
    let found = read(r, |tx| tx.user(row.id).expect("reads")).expect("the user");
    assert_eq!(
        found.deactivated_at,
        Some(T0 + 30),
        "the first deactivation's time"
    );
}

pub(crate) fn edge_an_identity_is_free_again_after_deactivation(r: &dyn Registry) {
    let identity = Some(("https://idp.example.com", "subject-1"));
    write(r, |tx| {
        tx.insert_user(&user(1, identity, T0)).expect("inserts")
    });
    write(r, |tx| {
        tx.deactivate_user(UserId([1; 16]), T0 + 5)
            .expect("deactivates")
    });
    write(r, |tx| {
        tx.insert_user(&user(2, identity, T0 + 6))
            .expect("a new user takes it")
    });
    let (taken, users) = read(r, |tx| {
        (
            tx.identity_taken("https://idp.example.com", "subject-1")
                .expect("reads"),
            tx.users().expect("lists").len(),
        )
    });
    assert!(taken, "the new user holds the identity");
    assert_eq!(users, 2, "the deactivated user keeps its row");
}

pub(crate) fn edge_a_tenant_with_nothing_optional_reads_back(r: &dyn Registry) {
    let mut row = tenant(1, "alpha");
    row.retention = None;
    row.metadata = None;
    row.role_cap = None;
    row.pending = None;
    write(r, |tx| tx.insert_tenant(&row).expect("inserts"));
    let found = read(r, |tx| tx.tenant(row.id).expect("reads"));
    assert_eq!(found, Some(row), "each absent column reads back absent");
}

pub(crate) fn edge_long_and_non_ascii_text_reads_back(r: &dyn Registry) {
    let name = "\u{e9}".repeat(128);
    let row = tenant(1, &name);
    let mut labeled = key(1, None, T0);
    labeled.label = format!("{} \u{4e2d}\u{6587} \u{1f511}", "l".repeat(300));
    write(r, |tx| {
        tx.insert_tenant(&row).expect("inserts");
        tx.insert_key(&labeled).expect("inserts");
        tx.put_setting(row.id, "note", &"v".repeat(4_000))
            .expect("puts");
    });
    let (taken, found, key_found, settings) = read(r, |tx| {
        (
            tx.name_taken(&name).expect("reads"),
            tx.tenant(row.id).expect("reads"),
            tx.key(labeled.id).expect("reads"),
            tx.settings(row.id).expect("lists"),
        )
    });
    assert!(taken, "a name of 128 two-byte characters is found");
    assert_eq!(found, Some(row), "and reads back");
    assert_eq!(
        key_found,
        Some(labeled),
        "a long label in three scripts reads back"
    );
    assert_eq!(
        settings[0].1.len(),
        4_000,
        "a value of 4,000 bytes reads back whole"
    );
}

pub(crate) fn edge_empty_text_reads_back(r: &dyn Registry) {
    let mut bare = key(1, None, T0);
    bare.label = String::new();
    bare.scopes = String::new();
    let mut unnamed = user(1, Some(("", "")), T0);
    unnamed.label = String::new();
    let mut roleless = membership(1, None, T0);
    roleless.roles = String::new();
    write(r, |tx| {
        tx.insert_key(&bare).expect("inserts");
        tx.insert_user(&unnamed).expect("inserts");
        tx.insert_membership(&roleless).expect("inserts");
        tx.put_setting(tenant_id(1), "empty", "").expect("puts");
    });
    let found = read(r, |tx| {
        (
            tx.key(bare.id).expect("reads"),
            tx.user(unnamed.id).expect("reads"),
            tx.membership(roleless.user, None).expect("reads"),
            tx.settings(tenant_id(1)).expect("lists"),
        )
    });
    assert_eq!(
        found,
        (
            Some(bare),
            Some(unnamed),
            Some(roleless),
            vec![("empty".to_owned(), String::new())]
        ),
        "empty text is text, not absence"
    );
}

pub(crate) fn edge_a_count_of_nothing_makes_no_record(r: &dyn Registry) {
    write(r, |tx| {
        tx.count_many(&entry(Action::Read, None, 429), at(0), 0)
            .expect("answers");
    });
    assert!(exported(r, 0).is_empty(), "no record for a count of 0");
}

pub(crate) fn edge_counts_in_two_minutes_are_two_records(r: &dyn Registry) {
    let counted = entry(Action::Read, Some("a"), 401);
    write(r, |tx| {
        tx.count(&counted, at(59)).expect("counts");
        tx.count(&counted, at(60)).expect("counts");
        tx.count(&counted, at(119)).expect("counts");
    });
    let counts: Vec<(String, i64)> = exported(r, 0)
        .iter()
        .map(|rec| {
            (
                rec["at"].as_str().expect("a time").to_owned(),
                rec["count"].as_i64().expect("a count"),
            )
        })
        .collect();
    assert_eq!(
        counts,
        [
            (crate::audit::rfc3339(at(0)), 1),
            (crate::audit::rfc3339(at(60)), 2)
        ],
        "the second 59 ends one minute and the second 60 starts the next"
    );
}

pub(crate) fn edge_a_record_with_no_target_and_no_answer_is_exported(r: &dyn Registry) {
    write(r, |tx| {
        tx.record(&entry(Action::Create, None, NO_ANSWER), at(0))
            .expect("records");
    });
    let records = exported(r, 0);
    assert_eq!(records.len(), 1, "one record");
    assert!(records[0]["target"].is_null(), "no target");
    assert!(
        records[0]["status"].is_null(),
        "no status for what answers no request"
    );
}

pub(crate) fn edge_the_log_after_its_last_seq_is_empty(r: &dyn Registry) {
    write(r, |tx| {
        tx.record(&entry(Action::Create, None, 201), at(0))
            .expect("records");
        tx.record(&entry(Action::Create, None, 201), at(1))
            .expect("records");
    });
    let all = page(r, 0, None);
    let last = seq(&all[1]);
    assert!(
        page(r, last, Some(10)).is_empty(),
        "a page after the last record"
    );
    assert!(
        page(r, last, None).is_empty(),
        "the export after the last record"
    );
    assert!(page(r, 0, Some(0)).is_empty(), "a page of no records");
    assert_eq!(
        page(r, last - 1, Some(10)).len(),
        1,
        "the last record follows the one before"
    );
}

pub(crate) fn edge_a_session_ends_at_its_limit_exactly(r: &dyn Registry) {
    let mut idle = session(1, 1, None);
    idle.idle_expires_at = T0 + 100;
    let mut whole = session(2, 1, None);
    whole.expires_at = T0 + 100;
    let mut later = session(3, 1, None);
    later.idle_expires_at = T0 + 101;
    write(r, |tx| {
        for row in [&idle, &whole, &later] {
            tx.insert_session(row).expect("inserts");
        }
    });
    write(r, |tx| tx.delete_sessions_ended(T0 + 100).expect("deletes"));
    let held: Vec<bool> = read(r, |tx| {
        (1..=3)
            .map(|n: u8| tx.session(&[n; 32]).expect("reads").is_some())
            .collect()
    });
    assert_eq!(
        held,
        [false, false, true],
        "a limit at the time given has passed"
    );
}

pub(crate) fn edge_deleting_what_is_not_there_succeeds(r: &dyn Registry) {
    write(r, |tx| {
        tx.delete_session(&[1; 32]).expect("no session");
        tx.delete_sessions_of_user(UserId([1; 16]))
            .expect("no sessions");
        tx.delete_sessions_in(tenant_id(1)).expect("no sessions");
        tx.delete_sessions_ended(T0).expect("no sessions");
        tx.delete_setting(tenant_id(1), "none").expect("no setting");
        tx.delete_settings_in(tenant_id(1)).expect("no settings");
        tx.delete_membership(UserId([1; 16]), None)
            .expect("no membership");
        tx.delete_memberships_of_user(UserId([1; 16]))
            .expect("no memberships");
        tx.delete_memberships_in(tenant_id(1))
            .expect("no memberships");
        tx.delete_tenant(tenant_id(1), T0).expect("no tenant");
        tx.revoke_key(KeyId([1; 8]), T0).expect("no key");
        tx.deactivate_user(UserId([1; 16]), T0).expect("no user");
        tx.set_roles(&membership(1, None, T0))
            .expect("no membership");
        tx.touch_session(&[1; 32], T0, T0).expect("no session");
    });
    let empty = read(r, |tx| {
        tx.tenants().expect("lists").is_empty()
            && tx.keys(None).expect("lists").is_empty()
            && tx.users().expect("lists").is_empty()
            && tx.memberships().expect("lists").is_empty()
    });
    assert!(
        empty,
        "nothing was made by removing or changing what is not there"
    );
}

pub(crate) fn edge_a_tenant_with_no_keys_lists_none(r: &dyn Registry) {
    write(r, |tx| {
        tx.insert_key(&key(1, None, T0)).expect("inserts");
        tx.insert_key(&key(2, Some(tenant_id(2)), T0))
            .expect("inserts");
    });
    let found = read(r, |tx| tx.keys(Some(tenant_id(1))).expect("lists"));
    assert!(
        found.is_empty(),
        "an operator's key and another tenant's are not this tenant's"
    );
}

pub(crate) fn edge_deleting_a_tenant_keeps_an_earlier_revocation(r: &dyn Registry) {
    let mut revoked = key(1, Some(tenant_id(1)), T0);
    revoked.revoked_at = Some(T0 + 10);
    write(r, |tx| {
        tx.insert_tenant(&tenant(1, "alpha")).expect("inserts");
        tx.insert_key(&revoked).expect("inserts");
        tx.insert_key(&key(2, Some(tenant_id(1)), T0 + 1))
            .expect("inserts");
    });
    write(r, |tx| {
        tx.delete_tenant(tenant_id(1), T0 + 70).expect("deletes")
    });
    let times: Vec<Option<i64>> = read(r, |tx| tx.keys(Some(tenant_id(1))).expect("lists"))
        .into_iter()
        .map(|k| k.revoked_at)
        .collect();
    assert_eq!(
        times,
        [Some(T0 + 10), Some(T0 + 70)],
        "a key revoked before keeps its time, and the keys stay listed"
    );
}

pub(crate) fn edge_an_export_after_a_seq_leaves_out_the_older(r: &dyn Registry) {
    write(r, |tx| {
        for n in 0..3_u8 {
            tx.record(
                &entry(Action::Create, Some(&n.to_string()), 201),
                at(i64::from(n)),
            )
            .expect("records");
        }
    });
    let all = exported(r, 0);
    assert_eq!(all.len(), 3, "after 0, every record");
    assert!(
        seq(&all[0]) < seq(&all[1]) && seq(&all[1]) < seq(&all[2]),
        "seqs rise"
    );
    let rest = exported(r, seq(&all[0]));
    assert_eq!(rest.len(), 2, "after the first seq, the two newer");
    assert_eq!(rest[0]["target"], "1", "starting at the second record");
    assert!(
        exported(r, i64::MAX).is_empty(),
        "after the largest seq, none"
    );
}

pub(crate) fn edge_settings_of_two_tenants_are_apart(r: &dyn Registry) {
    write(r, |tx| {
        tx.put_setting(tenant_id(1), "purge.grace", "P7D")
            .expect("puts");
        tx.put_setting(tenant_id(2), "purge.grace", "P1D")
            .expect("puts");
    });
    write(r, |tx| {
        tx.delete_settings_in(tenant_id(2)).expect("deletes")
    });
    let (first, second) = read(r, |tx| {
        (
            tx.settings(tenant_id(1)).expect("lists"),
            tx.settings(tenant_id(2)).expect("lists"),
        )
    });
    assert_eq!(
        first,
        [("purge.grace".to_owned(), "P7D".to_owned())],
        "one tenant's is kept"
    );
    assert!(second.is_empty(), "the other's went");
}

pub(crate) fn edge_an_empty_registry_lists_nothing(r: &dyn Registry) {
    let empty = read(r, |tx| {
        tx.tenants().expect("lists").is_empty()
            && tx.keys(None).expect("lists").is_empty()
            && tx.users().expect("lists").is_empty()
            && tx.memberships().expect("lists").is_empty()
            && tx.settings(tenant_id(1)).expect("lists").is_empty()
            && tx
                .record_times(Action::Create, 201)
                .expect("reads")
                .is_empty()
    });
    assert!(empty, "no rows of any kind");
    assert!(exported(r, 0).is_empty(), "and no records");
    let tx = r.immediate().expect("a write begins");
    assert_eq!(tx.commit(), Ok(()), "a write of nothing commits");
}

/// The traits' own shape, read from the source.
mod shape {
    /// The text of `backend.rs` from the trait `name` to the next item at
    /// the start of a line.
    fn trait_text(name: &str) -> &'static str {
        let source = include_str!("../backend.rs");
        let head = format!("pub trait {name}");
        let start = source.find(&head).expect("the trait is declared");
        let end = source[start..].find("\n}\n").expect("the trait ends");
        &source[start..start + end]
    }

    #[test]
    fn the_registry_traits_name_no_type_of_one_backend() {
        for name in ["Registry:", "RegistryTx "] {
            let text = trait_text(name);
            assert!(text.contains("fn "), "{name} the text read is the trait's");
            for word in ["rusqlite", "tokio_postgres", "local_conn"] {
                assert!(
                    !text.contains(word),
                    "{name} every backend implements the trait, so it names no `{word}`"
                );
            }
        }
    }
}

/// The local registry, held to the suite.
mod local {
    use super::Fresh;
    use crate::backend::local::LocalRegistry;
    use crate::envelope::Key;

    fn fresh() -> Fresh {
        let dir = tempfile::tempdir().expect("a directory");
        let key = Key::generate().expect("a key");
        let registry = LocalRegistry::open(dir.path(), key).expect("a new registry opens");
        Fresh {
            registry: Box::new(registry),
            _keep: Box::new(dir),
        }
    }

    registry_conformance!(fresh);
}
