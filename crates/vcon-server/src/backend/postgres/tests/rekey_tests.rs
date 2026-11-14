//! A rotation of the master key over the registry in PostgreSQL: what
//! must work, what must be refused, and the edges. The tests in
//! `on_a_server` need a PostgreSQL server.

use super::super::catalog::audit_key_statements;
use super::super::registry::{
    Column, KeyColumn, KeyValue, REMADE, membership_key, row_key, setting_key,
};
use crate::envelope::REGISTRY;
use crate::ids::{TenantId, UserId};

/// The names of the columns a rotation writes for `column`.
fn written(column: &Column) -> Vec<&'static str> {
    match *column {
        Column::Sealed(name) | Column::TenantKey(name) => vec![name],
        Column::SealedWithMac { sealed, mac } => vec![mac, sealed],
    }
}

/// The name of a column of a primary key.
fn key_name(column: &KeyColumn) -> &'static str {
    match *column {
        KeyColumn::Bytes(name) | KeyColumn::BytesOrNone(name) | KeyColumn::Text(name) => name,
    }
}

// What must work.

#[test]
fn the_registry_s_log_keys_are_read_and_sealed_again_with_no_tenant_named() {
    let (select, update) = audit_key_statements(REGISTRY);
    assert_eq!(
        select, "SELECT key_id, wrapped FROM vcon.admin_audit_key ORDER BY ordinal",
        "the oldest first, from the registry's table"
    );
    assert_eq!(
        update, "UPDATE vcon.admin_audit_key SET wrapped = $2 WHERE key_id = $1",
        "the sealed copy alone is written"
    );
}

#[test]
fn a_tenant_s_log_keys_are_read_and_sealed_again_for_that_tenant_alone() {
    let (select, update) = audit_key_statements(TenantId([4; 16]));
    assert_eq!(
        select, "SELECT key_id, wrapped FROM vcon.audit_key WHERE tenant = $1 ORDER BY ordinal",
        "the tenant's keys, oldest first"
    );
    assert_eq!(
        update, "UPDATE vcon.audit_key SET wrapped = $2 WHERE key_id = $1 AND tenant = $3",
        "the tenant is the last parameter, after the two the registry's statement takes"
    );
}

#[test]
fn a_membership_s_row_key_is_the_one_the_registry_seals_it_with() {
    let (user, tenant) = (UserId([7; 16]), TenantId([9; 16]));
    assert_eq!(
        row_key(&[
            KeyValue::Bytes(Some(user.0.to_vec())),
            KeyValue::Bytes(Some(tenant.0.to_vec())),
        ]),
        membership_key(user, Some(tenant)),
        "the user, then the tenant"
    );
}

#[test]
fn a_setting_s_row_key_is_the_one_the_registry_seals_it_with() {
    let tenant = TenantId([4; 16]);
    assert_eq!(
        row_key(&[
            KeyValue::Bytes(Some(tenant.0.to_vec())),
            KeyValue::Text("retention.days".to_owned()),
        ]),
        setting_key(tenant, "retention.days"),
        "the tenant, then the setting's name"
    );
}

#[test]
fn the_rotation_makes_six_tables_again_each_once() {
    let tables: Vec<&str> = REMADE.iter().map(|t| t.table).collect();
    assert_eq!(
        tables,
        [
            "tenant",
            "api_key",
            "user_account",
            "membership",
            "tenant_setting",
            "session"
        ],
        "the registry's tables with a sealed column"
    );
}

#[test]
fn a_table_is_read_by_its_key_then_the_columns_made_again() {
    let tenant = REMADE
        .iter()
        .find(|t| t.table == "tenant")
        .expect("the tenants");
    assert_eq!(
        tenant.fetch_text(),
        "SELECT id, name_mac, name_sealed, retention_sealed, metadata_sealed, role_cap_sealed, \
         wrapped_tenant_key, pending_wrapped_tenant_key FROM vcon.tenant",
        "the key first, then each column in the order it is written"
    );
}

#[test]
fn a_batch_is_written_in_one_statement_that_finds_each_row_by_its_key() {
    let session = REMADE
        .iter()
        .find(|t| t.table == "session")
        .expect("the sessions");
    assert_eq!(
        session.update_text(),
        "UPDATE vcon.session t SET scopes_sealed = n.scopes_sealed, amr_sealed = n.amr_sealed \
         FROM unnest($1::bytea[], $2::bytea[], $3::bytea[]) AS n(id_hash, scopes_sealed, amr_sealed) \
         WHERE t.id_hash = n.id_hash",
        "one array for each column, the key's first"
    );
}

// What must be refused.

#[test]
fn no_statement_of_the_rotation_s_tables_names_the_log() {
    for table in &REMADE {
        for text in [table.fetch_text(), table.update_text()] {
            assert!(
                !text.contains("audit"),
                "a record of the log is never written again: {text}"
            );
        }
    }
}

#[test]
fn no_batch_writes_a_column_of_a_row_s_key() {
    for table in &REMADE {
        let text = table.update_text();
        for key in table.key {
            let name = key_name(key);
            assert!(
                !text.contains(&format!(" {name} = n.{name},"))
                    && !text.contains(&format!(" {name} = n.{name} FROM")),
                "{}: {name} names the row and stays as it is: {text}",
                table.table
            );
        }
    }
}

// The edges.

#[test]
fn an_operator_s_row_key_is_the_user_alone() {
    let user = UserId([7; 16]);
    assert_eq!(
        row_key(&[
            KeyValue::Bytes(Some(user.0.to_vec())),
            KeyValue::Bytes(None)
        ]),
        membership_key(user, None),
        "a tenant that is NULL adds no byte"
    );
}

#[test]
fn a_row_key_of_no_value_is_empty_and_a_name_gives_its_utf8_bytes() {
    assert_eq!(row_key(&[]), Vec::<u8>::new(), "no value, no byte");
    assert_eq!(
        row_key(&[KeyValue::Text("\u{fc}".to_owned())]),
        [0xc3, 0xbc],
        "a name's bytes are its UTF-8"
    );
}

#[test]
fn a_key_column_that_may_be_null_is_matched_as_not_distinct() {
    let membership = REMADE
        .iter()
        .find(|t| t.table == "membership")
        .expect("the memberships");
    assert_eq!(
        membership.update_text(),
        "UPDATE vcon.membership t SET roles_sealed = n.roles_sealed \
         FROM unnest($1::bytea[], $2::bytea[], $3::bytea[]) AS n(user_id, tenant, roles_sealed) \
         WHERE t.user_id = n.user_id AND t.tenant IS NOT DISTINCT FROM n.tenant",
        "an operator's membership has no tenant, and = finds no NULL"
    );
}

#[test]
fn a_key_column_of_text_goes_as_an_array_of_text() {
    let setting = REMADE
        .iter()
        .find(|t| t.table == "tenant_setting")
        .expect("the settings");
    assert_eq!(
        setting.update_text(),
        "UPDATE vcon.tenant_setting t SET value_sealed = n.value_sealed \
         FROM unnest($1::bytea[], $2::text[], $3::bytea[]) AS n(tenant, name, value_sealed) \
         WHERE t.tenant = n.tenant AND t.name = n.name",
        "a setting's name is text"
    );
}

#[test]
fn only_the_tenants_hold_keys_wrapped_under_the_master_key() {
    for table in &REMADE {
        assert!(!table.key.is_empty(), "{}: a row has a key", table.table);
        assert!(
            !table.columns.is_empty(),
            "{}: a table with no column is not in the list",
            table.table
        );
        let wraps = table
            .columns
            .iter()
            .filter(|c| matches!(c, Column::TenantKey(_)))
            .count();
        if table.table == "tenant" {
            assert_eq!(wraps, 2, "the tenant key and the pending one");
            assert_eq!(
                table.key,
                [KeyColumn::Bytes("id")],
                "the tenant's id is the wrapped key's context"
            );
        } else {
            assert_eq!(wraps, 0, "{} holds no tenant key", table.table);
        }
    }
}

#[test]
fn no_column_is_written_twice_in_a_table() {
    for table in &REMADE {
        let mut names: Vec<&str> = table.columns.iter().flat_map(written).collect();
        let all = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all, "{}: each column once", table.table);
    }
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    use time::OffsetDateTime;

    use super::super::super::catalog::Database;
    use super::super::super::registry::{PgRegistry, REMADE, Remade};
    use super::super::catalog_tests::on_a_server::{database, tenant_catalog};
    use super::super::registry_tests::on_a_server::{
        as_owner, exported, hex, log_keys, named, wait,
    };
    use super::super::schema_tests::live::{Db, in_scope};
    use super::{key_name, written};
    use crate::audit::{Action, Actor, Entry};
    use crate::backend::{
        CommitError, KeyRow, MembershipRow, Registry, RegistryTx, SessionRow, TenantRow, UserRow,
    };
    use crate::clock::nanos;
    use crate::envelope::{self, Context, Key};
    use crate::ids::{KeyId, TenantId, UserId};
    use crate::tenants::TenantError;

    const T0: i64 = 1_792_756_800;

    /// The rows a rotation reads and writes in one statement, in the
    /// tests that do not set it.
    const BATCH: i64 = 1_000;

    /// Every table that holds a row of the registry.
    const TABLES: [&str; 10] = [
        "tenant",
        "api_key",
        "user_account",
        "membership",
        "tenant_setting",
        "session",
        "master_key_use",
        "registry_key",
        "admin_audit_key",
        "admin_audit",
    ];

    const ISSUER: &str = "https://issuer.example.com";

    /// A migrated database of the test's own and two master keys: the
    /// one the registry is under, and the one a rotation moves it to.
    struct World {
        db: Db,
        database: Arc<Database>,
        old: Key,
        new: Key,
    }

    fn world() -> World {
        let (db, database) = database();
        World {
            db,
            database,
            old: Key::generate().expect("a key"),
            new: Key::generate().expect("a key"),
        }
    }

    fn catalog_key(master: &Key) -> Key {
        envelope::catalog_key(master).expect("the catalog key")
    }

    /// The registry, opened under `master`'s catalog key.
    fn open(w: &World, master: &Key) -> Result<PgRegistry, TenantError> {
        PgRegistry::open(w.database.clone(), &catalog_key(master))
    }

    /// The time the rotations of these tests are made at.
    fn now() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(T0 + 3_600).expect("a time")
    }

    fn rotate(w: &World) -> Result<(), TenantError> {
        PgRegistry::rekey_master(&w.database, &w.old, &w.new, now(), BATCH)
    }

    fn tenant_id(n: u8) -> TenantId {
        TenantId([n; 16])
    }

    fn user_id(n: u8) -> UserId {
        UserId([n; 16])
    }

    /// A bytea literal of `bytes`.
    fn lit(bytes: &[u8]) -> String {
        format!("'\\x{}'", hex(bytes))
    }

    /// Every row of one table of the registry, as one hash.
    fn table(db: &Db, name: &str) -> String {
        as_owner(
            db,
            &format!(
                "SELECT coalesce(md5(string_agg(t::text, ';' ORDER BY t::text)), 'none') \
                 FROM vcon.{name} t"
            ),
        )
        .concat()
    }

    /// Every row of the registry, by table.
    fn state(db: &Db) -> Vec<(&'static str, String)> {
        TABLES.iter().map(|name| (*name, table(db, name))).collect()
    }

    /// The names of the tables whose rows differ between two states.
    fn changed(
        before: &[(&'static str, String)],
        after: &[(&'static str, String)],
    ) -> Vec<&'static str> {
        before
            .iter()
            .zip(after)
            .filter(|(b, a)| b.1 != a.1)
            .map(|(b, _)| b.0)
            .collect()
    }

    fn write<T>(r: &PgRegistry, f: impl FnOnce(&mut dyn RegistryTx) -> T) -> T {
        let mut tx = r.immediate().expect("a write begins");
        let out = f(tx.as_mut());
        tx.commit().expect("the write commits");
        out
    }

    fn about(target: &str) -> Entry {
        Entry {
            actor: Actor::Cli,
            action: Action::Read,
            target: Some(target.to_owned()),
            version: None,
            status: 200,
        }
    }

    /// A tenant whose key, and with `pending` a second one, is wrapped
    /// under `master`; the ids of the two keys.
    fn tenant(
        master: &Key,
        n: u8,
        name: &str,
        full: bool,
        pending: bool,
    ) -> (TenantRow, KeyId, Option<KeyId>) {
        let id = tenant_id(n);
        let ctx = Context::Tenant { tenant: id };
        let key = Key::generate().expect("a key");
        let next = pending.then(|| Key::generate().expect("a key"));
        let text = |value: &str| full.then(|| value.to_owned());
        let row = TenantRow {
            id,
            name: name.to_owned(),
            retention: text("P30D-retention"),
            metadata: text("metadata-level"),
            role_cap: text("role-cap-text"),
            created_at: T0,
            tenant_key_id: key.id(),
            wrapped: envelope::wrap(master, &ctx, &key),
            pending: next.as_ref().map(|k| envelope::wrap(master, &ctx, k)),
            key_since: T0,
        };
        (row, key.id(), next.map(|k| k.id()))
    }

    fn user(n: u8, label: &str, subject: Option<&str>, deactivated: bool) -> UserRow {
        UserRow {
            id: user_id(n),
            label: label.to_owned(),
            identity: subject.map(|s| (ISSUER.to_owned(), s.to_owned())),
            created_at: T0,
            deactivated_at: deactivated.then_some(T0 + 5),
        }
    }

    fn session(n: u8, user: u8, tenant: Option<u8>) -> SessionRow {
        SessionRow {
            id_hash: vec![n; 32],
            user: user_id(user),
            tenant: tenant.map(tenant_id),
            scopes: "scopes-of-the-session".to_owned(),
            created_at: T0,
            last_seen_at: T0 + 1,
            idle_expires_at: T0 + 900,
            expires_at: T0 + 86_400,
            idle_seconds: 900,
            amr: "authentication-methods".to_owned(),
            csrf_hash: vec![n ^ 0xff; 32],
        }
    }

    /// The tenants' keys a filled registry holds: the tenant, its key's
    /// id, and the pending key's.
    type TenantKeys = Vec<(TenantId, KeyId, Option<KeyId>)>;

    /// Fills the registry under `master` with rows of every table: three
    /// tenants (the first with a pending key, the second with no optional
    /// text), two API keys, three users (one with no identity, one
    /// deactivated that shares the first's), three memberships (one an
    /// operator's), three settings, two sessions and two records.
    fn fill_under(w: &World, master: &Key) -> TenantKeys {
        let registry = open(w, master).expect("the registry opens");
        let tenants = [
            tenant(master, 1, "alpha-tenant-name", true, true),
            tenant(master, 2, "beta-tenant-name", false, false),
            tenant(master, 3, "gamma-tenant-name", true, false),
        ];
        write(&registry, |tx| {
            for (row, _, _) in &tenants {
                tx.insert_tenant(row).expect("a tenant");
            }
            for (n, tenant) in [(1, None), (2, Some(tenant_id(1)))] {
                tx.insert_key(&KeyRow {
                    id: KeyId([n; 8]),
                    tenant,
                    token_hash: vec![n; 32],
                    scopes: "scopes-of-the-key".to_owned(),
                    label: format!("label-of-key-{n}"),
                    created_at: T0,
                    revoked_at: None,
                    expires_at: Some(T0 + 99),
                    last_used_at: None,
                })
                .expect("a key");
            }
            tx.insert_user(&user(3, "user-three-label", Some("subject-one"), true))
                .expect("a user");
            tx.insert_user(&user(1, "user-one-label", Some("subject-one"), false))
                .expect("a user");
            tx.insert_user(&user(2, "user-two-label", None, false))
                .expect("a user");
            for (user, tenant) in [(1, Some(1)), (1, None), (2, Some(2))] {
                tx.insert_membership(&MembershipRow {
                    user: user_id(user),
                    tenant: tenant.map(tenant_id),
                    roles: format!("roles-of-{user}"),
                    granted_at: T0,
                })
                .expect("a membership");
            }
            tx.put_setting(tenant_id(1), "retention.days", "thirty-days-value")
                .expect("a setting");
            tx.put_setting(tenant_id(1), "n\u{e4}me", "second-value")
                .expect("a setting");
            tx.put_setting(tenant_id(2), "retention.days", "other-value")
                .expect("a setting");
            tx.insert_session(&session(0xa1, 1, Some(1)))
                .expect("a session");
            tx.insert_session(&session(0xa2, 1, None))
                .expect("a session");
            let at = OffsetDateTime::from_unix_timestamp(T0).expect("a time");
            tx.record(&about("target-one"), at).expect("a record");
            tx.record(&about("target-two"), at).expect("a record");
        });
        tenants.iter().map(|(row, k, p)| (row.id, *k, *p)).collect()
    }

    fn fill(w: &World) -> TenantKeys {
        fill_under(w, &w.old)
    }

    /// What the registry reads of every table, the tenants' wrapped keys
    /// left out: a rotation wraps them again.
    #[derive(Debug, PartialEq)]
    struct Rows {
        tenants: Vec<TenantRow>,
        keys: Vec<Option<KeyRow>>,
        users: Vec<UserRow>,
        memberships: Vec<MembershipRow>,
        settings: Vec<Vec<(String, String)>>,
        sessions: Vec<Option<SessionRow>>,
    }

    fn read(registry: &PgRegistry) -> Rows {
        let mut tx = registry.snapshot().expect("a read begins");
        let mut tenants = tx.tenants().expect("the tenants");
        tenants.sort_by_key(|t| t.id.0);
        for t in &mut tenants {
            t.wrapped.clear();
            t.pending = t.pending.as_ref().map(|_| Vec::new());
        }
        let keys = [1, 2]
            .iter()
            .map(|n| tx.key(KeyId([*n; 8])).expect("a key"))
            .collect();
        let mut users = tx.users().expect("the users");
        users.sort_by_key(|u| u.id.0);
        let mut memberships = tx.memberships().expect("the memberships");
        memberships.sort_by_key(|m| (m.user.0, m.tenant.map(|t| t.0)));
        let settings = [1, 2, 3]
            .iter()
            .map(|n| {
                let mut settings = tx.settings(tenant_id(*n)).expect("the settings");
                settings.sort();
                settings
            })
            .collect();
        let sessions = [0xa1_u8, 0xa2]
            .iter()
            .map(|n| tx.session(&[*n; 32]).expect("a session"))
            .collect();
        tx.commit().expect("the read ends");
        Rows {
            tenants,
            keys,
            users,
            memberships,
            settings,
            sessions,
        }
    }

    /// What a filled registry reads, with the registry still under the
    /// old master key.
    fn filled(w: &World) -> Rows {
        read(&open(w, &w.old).expect("the registry opens under the old key"))
    }

    /// The wrapped keys of each tenant as the database holds them, in
    /// the order of the tenants' ids.
    fn wraps(w: &World, master: &Key) -> Vec<(TenantId, Vec<u8>, Option<Vec<u8>>)> {
        let registry = open(w, master).expect("the registry opens");
        let mut tx = registry.snapshot().expect("a read begins");
        let mut tenants = tx.tenants().expect("the tenants");
        tx.commit().expect("the read ends");
        tenants.sort_by_key(|t| t.id.0);
        tenants
            .into_iter()
            .map(|t| (t.id, t.wrapped, t.pending))
            .collect()
    }

    /// Each value of `column` in `spec`'s table, in hex, in the order of
    /// the rows' keys; a NULL is an empty text.
    fn values(db: &Db, spec: &Remade, column: &str) -> Vec<String> {
        let order: Vec<&str> = spec.key.iter().map(key_name).collect();
        as_owner(
            db,
            &format!(
                "SELECT coalesce(encode({column}, 'hex'), '') FROM vcon.{} ORDER BY {}",
                spec.table,
                order.join(", ")
            ),
        )
    }

    /// Changes the last byte of one sealed value.
    fn flip(db: &Db, table: &str, column: &str, row: &str) {
        let last = format!("octet_length({column}) - 1");
        let rows = as_owner(
            db,
            &format!(
                "UPDATE vcon.{table} SET {column} = \
                 set_byte({column}, {last}, get_byte({column}, {last}) # 1) \
                 WHERE {row} RETURNING 1"
            ),
        );
        assert_eq!(rows.len(), 1, "{table}.{column}: one row is changed");
    }

    /// Each record of the registry's log, as the database holds it, in
    /// the order they were written.
    fn records(db: &Db) -> Vec<String> {
        as_owner(db, "SELECT a::text FROM vcon.admin_audit a ORDER BY seq")
    }

    /// The rows of the master keys' use: the key's id in hex, its wraps
    /// and the time it has been in use since.
    fn uses(db: &Db) -> Vec<String> {
        as_owner(
            db,
            "SELECT encode(master_key_id, 'hex'), wraps, since FROM vcon.master_key_use \
             ORDER BY master_key_id",
        )
    }

    // What must work.

    #[test]
    fn the_registry_opens_under_the_new_master_key_and_not_under_the_old() {
        let w = world();
        fill(&w);
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        assert!(
            open(&w, &w.new).is_ok(),
            "the registry opens under the new master key"
        );
        assert_eq!(
            open(&w, &w.old).err(),
            Some(TenantError::WrongKey),
            "and is refused under the old one"
        );
    }

    #[test]
    fn every_row_reads_the_same_under_the_new_master_key() {
        let w = world();
        fill(&w);
        let before = filled(&w);
        assert_eq!(before.tenants.len(), 3, "the tenants were written");
        assert_eq!(before.users.len(), 3, "the users were written");
        assert_eq!(before.memberships.len(), 3, "the memberships were written");
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        let after = read(&open(&w, &w.new).expect("opens under the new key"));
        assert_eq!(after, before, "what a person typed is what it was");
    }

    #[test]
    fn a_tenant_is_found_by_its_name_under_the_new_master_key() {
        let w = world();
        fill(&w);
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        let registry = open(&w, &w.new).expect("opens under the new key");
        let mut tx = registry.snapshot().expect("a read begins");
        assert_eq!(
            tx.name_taken("alpha-tenant-name"),
            Ok(true),
            "the name's HMAC was made again under the new index key"
        );
        assert_eq!(
            tx.name_taken("no-such-tenant"),
            Ok(false),
            "a name no tenant has is not found"
        );
    }

    #[test]
    fn a_user_is_found_by_its_identity_under_the_new_master_key() {
        let w = world();
        fill(&w);
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        let registry = open(&w, &w.new).expect("opens under the new key");
        let mut tx = registry.snapshot().expect("a read begins");
        assert_eq!(
            tx.identity_taken(ISSUER, "subject-one"),
            Ok(true),
            "the identity's HMAC was made again under the new index key"
        );
        assert_eq!(
            tx.identity_taken(ISSUER, "subject-nine"),
            Ok(false),
            "an identity no user has is not found"
        );
    }

    #[test]
    fn each_tenant_key_opens_under_the_new_master_key() {
        let w = world();
        let keys = fill(&w);
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        let wraps = wraps(&w, &w.new);
        assert_eq!(wraps.len(), 3, "three tenants");
        for ((tenant, wrapped, _), (id, key, _)) in wraps.iter().zip(&keys) {
            assert_eq!(tenant, id, "the tenants in the order of their ids");
            let opened = envelope::unwrap(&w.new, &Context::Tenant { tenant: *tenant }, wrapped)
                .expect("the tenant key opens under the new master key");
            assert_eq!(opened.id(), *key, "and is the key the tenant had");
        }
    }

    #[test]
    fn a_pending_tenant_key_opens_under_the_new_master_key() {
        let w = world();
        let keys = fill(&w);
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        let wraps = wraps(&w, &w.new);
        let (tenant, _, pending) = &wraps[0];
        let pending = pending
            .as_ref()
            .expect("the first tenant has a pending key");
        let opened = envelope::unwrap(&w.new, &Context::Tenant { tenant: *tenant }, pending)
            .expect("the pending key opens under the new master key");
        assert_eq!(
            Some(opened.id()),
            keys[0].2,
            "and is the key that was pending"
        );
    }

    #[test]
    fn no_tenant_key_opens_under_the_old_master_key_after() {
        let w = world();
        fill(&w);
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        for (tenant, wrapped, pending) in wraps(&w, &w.new) {
            let ctx = Context::Tenant { tenant };
            assert!(
                envelope::unwrap(&w.old, &ctx, &wrapped).is_err(),
                "the old master key opens no tenant key"
            );
            if let Some(pending) = pending {
                assert!(
                    envelope::unwrap(&w.old, &ctx, &pending).is_err(),
                    "and no pending one"
                );
            }
        }
    }

    #[test]
    fn the_new_master_key_s_use_counts_the_keys_it_wrapped() {
        let w = world();
        fill(&w);
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        assert_eq!(
            uses(&w.db),
            [format!("{} 4 {}", hex(&w.new.id().0), nanos(now()))],
            "three tenant keys and one pending, since the rotation"
        );
        let registry = open(&w, &w.new).expect("opens under the new key");
        let mut tx = registry.snapshot().expect("a read begins");
        assert_eq!(tx.master_wraps(w.new.id()), Ok(4), "as the registry reads");
        assert_eq!(
            tx.master_since(w.new.id()),
            Ok(Some(nanos(now()))),
            "the new key is in use since the rotation"
        );
    }

    #[test]
    fn the_database_names_the_new_catalog_key() {
        let w = world();
        fill(&w);
        assert_eq!(
            named(&w.db),
            [hex(&catalog_key(&w.old).id().0)],
            "the old key's before"
        );
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        assert_eq!(
            named(&w.db),
            [hex(&catalog_key(&w.new).id().0)],
            "the one row names the new master key's catalog key"
        );
    }

    #[test]
    fn the_log_keeps_its_key_and_the_key_is_sealed_again() {
        let w = world();
        fill(&w);
        let before = log_keys(&w.db);
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        let after = log_keys(&w.db);
        assert_eq!(before.len(), 1, "one key before");
        assert_eq!(after.len(), 1, "one key after");
        let (b, a): (Vec<&str>, Vec<&str>) = (
            before[0].split(' ').collect(),
            after[0].split(' ').collect(),
        );
        assert_eq!(a[..2], b[..2], "the ordinal and the id stay");
        assert_ne!(a[2], b[2], "the sealed copy is made again");
    }

    #[test]
    fn the_records_written_before_read_after() {
        let w = world();
        fill(&w);
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        let registry = open(&w, &w.new).expect("opens under the new key");
        let text = exported(&registry).expect("the log is read");
        assert!(text.contains("target-one"), "the first target: {text}");
        assert!(text.contains("target-two"), "the second target: {text}");
    }

    #[test]
    fn no_record_of_the_log_is_changed_and_one_is_added() {
        let w = world();
        fill(&w);
        let before = records(&w.db);
        assert_eq!(before.len(), 2, "two records before");
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        let after = records(&w.db);
        assert_eq!(after.len(), 3, "one record more");
        assert_eq!(
            after[..2],
            before[..],
            "the records that were there are byte for byte what they were"
        );
    }

    #[test]
    fn the_rotation_is_recorded_under_the_log_s_key() {
        let w = world();
        fill(&w);
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        let key = as_owner(
            &w.db,
            "SELECT encode(key_id, 'hex') FROM vcon.admin_audit_key",
        )
        .concat();
        assert_eq!(
            as_owner(
                &w.db,
                "SELECT actor, action, status, count, target_sealed IS NULL, \
                 encode(key_id, 'hex') FROM vcon.admin_audit ORDER BY seq DESC LIMIT 1",
            ),
            [format!("cli rekey_master 200 1 t {key}")],
            "the command line rotated the master key"
        );
        let registry = open(&w, &w.new).expect("opens under the new key");
        let mut tx = registry.snapshot().expect("a read begins");
        assert_eq!(
            tx.record_times(Action::RekeyMaster, 200).map(|t| t.len()),
            Ok(1),
            "one record of the rotation"
        );
    }

    #[test]
    fn a_second_run_changes_nothing() {
        let w = world();
        fill(&w);
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        let before = state(&w.db);
        assert_eq!(
            rotate(&w),
            Ok(()),
            "a rotation that was made is asked for again"
        );
        assert_eq!(
            changed(&before, &state(&w.db)),
            Vec::<&str>::new(),
            "and no row is written, nor a second record"
        );
    }

    #[test]
    fn a_rotation_after_a_rotation_moves_the_registry_again() {
        let w = world();
        fill(&w);
        let before = filled(&w);
        assert_eq!(rotate(&w), Ok(()), "the first rotation is made");
        let third = Key::generate().expect("a key");
        assert_eq!(
            PgRegistry::rekey_master(&w.database, &w.new, &third, now(), BATCH),
            Ok(()),
            "and the second"
        );
        let registry = open(&w, &third).expect("opens under the third key");
        assert_eq!(read(&registry), before, "the rows read as they did");
        let mut tx = registry.snapshot().expect("a read begins");
        assert_eq!(
            tx.record_times(Action::RekeyMaster, 200).map(|t| t.len()),
            Ok(2),
            "each rotation has its record"
        );
        assert_eq!(
            open(&w, &w.new).err(),
            Some(TenantError::WrongKey),
            "the second key is the old one now"
        );
    }

    #[test]
    fn one_row_in_a_statement_gives_what_a_thousand_give() {
        let w = world();
        fill(&w);
        let before = filled(&w);
        assert_eq!(
            PgRegistry::rekey_master(&w.database, &w.old, &w.new, now(), 1),
            Ok(()),
            "a rotation that reads one row at a time"
        );
        let registry = open(&w, &w.new).expect("opens under the new key");
        assert_eq!(read(&registry), before, "every row was moved");
        assert_eq!(
            uses(&w.db),
            [format!("{} 4 {}", hex(&w.new.id().0), nanos(now()))],
            "and every tenant key counted"
        );
    }

    #[test]
    fn every_sealed_value_hmac_and_wrapped_key_is_written_again() {
        let w = world();
        fill(&w);
        let read_all = |db: &Db| -> Vec<(String, Vec<String>)> {
            REMADE
                .iter()
                .flat_map(|spec| {
                    spec.columns.iter().flat_map(written).map(move |column| {
                        (format!("{}.{column}", spec.table), values(db, spec, column))
                    })
                })
                .collect()
        };
        let before = read_all(&w.db);
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        let after = read_all(&w.db);
        assert_eq!(before.len(), 16, "sixteen columns in six tables");
        for ((name, was), (_, is)) in before.iter().zip(&after) {
            assert_eq!(was.len(), is.len(), "{name}: the same rows");
            assert!(!was.is_empty(), "{name}: the table has rows");
            for (was, is) in was.iter().zip(is) {
                assert_eq!(
                    was.is_empty(),
                    is.is_empty(),
                    "{name}: a NULL stays one, a value stays one"
                );
                assert!(
                    was.is_empty() || was != is,
                    "{name}: a value under the old key is not kept"
                );
            }
        }
    }

    #[test]
    fn what_is_not_sealed_is_as_it_was() {
        let w = world();
        fill(&w);
        let rest = |db: &Db| -> Vec<String> {
            REMADE
                .iter()
                .map(|spec| {
                    let made: Vec<&str> = spec.columns.iter().flat_map(written).collect();
                    let kept: Vec<String> = as_owner(
                        db,
                        &format!(
                            "SELECT column_name::text FROM information_schema.columns \
                             WHERE table_schema = 'vcon' AND table_name = '{}' \
                             ORDER BY ordinal_position",
                            spec.table
                        ),
                    )
                    .into_iter()
                    .filter(|c| !made.contains(&c.as_str()))
                    .collect();
                    assert!(!kept.is_empty(), "{}: a table has its key", spec.table);
                    let row = format!("ROW({})::text", kept.join(", "));
                    as_owner(
                        db,
                        &format!(
                            "SELECT md5(string_agg({row}, ';' ORDER BY {row})) FROM vcon.{}",
                            spec.table
                        ),
                    )
                    .concat()
                })
                .collect()
        };
        let before = rest(&w.db);
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        assert_eq!(
            rest(&w.db),
            before,
            "ids, times, hashes and names are not written"
        );
    }

    #[test]
    fn the_registry_takes_writes_after() {
        let w = world();
        fill(&w);
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        let registry = open(&w, &w.new).expect("opens under the new key");
        let (row, _, _) = tenant(&w.new, 9, "delta-tenant-name", true, false);
        write(&registry, |tx| {
            tx.insert_tenant(&row).expect("a tenant is added");
            tx.add_master_wrap(w.new.id(), nanos(now()))
                .expect("the wrap is counted");
            tx.record(&about("target-after"), now()).expect("a record");
        });
        let mut tx = registry.snapshot().expect("a read begins");
        assert_eq!(tx.tenants().map(|t| t.len()), Ok(4), "four tenants now");
        assert_eq!(
            tx.master_wraps(w.new.id()),
            Ok(5),
            "the count goes on from the rotation's"
        );
        drop(tx);
        let text = exported(&registry).expect("the log is read");
        assert!(text.contains("target-after"), "the new record: {text}");
        let mut again = registry.immediate().expect("a write begins");
        let (twin, _, _) = tenant(&w.new, 10, "alpha-tenant-name", true, false);
        assert!(
            again.insert_tenant(&twin).is_err(),
            "a name a tenant has is still one name, under the new HMAC"
        );
    }

    #[test]
    fn the_database_holds_no_registry_text_in_the_clear_after() {
        let w = world();
        fill(&w);
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        for spec in &REMADE {
            let rows = as_owner(&w.db, &format!("SELECT t::text FROM vcon.{} t", spec.table));
            assert!(!rows.is_empty(), "{}: the table has rows", spec.table);
            let all = rows.join("\n");
            for text in [
                "alpha-tenant-name",
                "P30D-retention",
                "metadata-level",
                "role-cap-text",
                "scopes-of-the-key",
                "label-of-key-1",
                "user-one-label",
                "subject-one",
                "roles-of-1",
                "thirty-days-value",
                "scopes-of-the-session",
                "authentication-methods",
            ] {
                assert!(
                    !all.contains(text) && !all.contains(&hex(text.as_bytes())),
                    "{}: {text} is not in the clear",
                    spec.table
                );
            }
        }
    }

    #[test]
    fn the_time_of_the_rotation_is_the_one_given() {
        let w = world();
        fill(&w);
        let at = OffsetDateTime::from_unix_timestamp(T0 + 86_400).expect("a time");
        assert_eq!(
            PgRegistry::rekey_master(&w.database, &w.old, &w.new, at, BATCH),
            Ok(()),
            "a rotation a day later"
        );
        assert_eq!(
            uses(&w.db),
            [format!("{} 4 {}", hex(&w.new.id().0), nanos(at))],
            "the new key is in use since the time given, not the clock's"
        );
    }

    // What must be refused.

    #[test]
    fn a_registry_under_neither_key_is_refused_and_left_as_it_is() {
        let w = world();
        fill(&w);
        let before = state(&w.db);
        let stranger = Key::generate().expect("a key");
        assert_eq!(
            PgRegistry::rekey_master(&w.database, &stranger, &w.new, now(), BATCH),
            Err(TenantError::WrongKey),
            "the registry is under neither key"
        );
        assert_eq!(
            changed(&before, &state(&w.db)),
            Vec::<&str>::new(),
            "no row is written"
        );
        assert!(open(&w, &w.old).is_ok(), "it opens under its key as before");
    }

    #[test]
    fn a_batch_of_no_row_is_refused() {
        let w = world();
        fill(&w);
        let before = state(&w.db);
        assert_eq!(
            PgRegistry::rekey_master(&w.database, &w.old, &w.new, now(), 0),
            Err(TenantError::Io),
            "a statement must read a row"
        );
        assert_eq!(
            changed(&before, &state(&w.db)),
            Vec::<&str>::new(),
            "no row is written"
        );
        assert_eq!(rotate(&w), Ok(()), "with a batch of rows it is made");
    }

    #[test]
    fn a_batch_below_no_row_is_refused() {
        let w = world();
        fill(&w);
        let before = state(&w.db);
        for batch in [-1, i64::MIN] {
            assert_eq!(
                PgRegistry::rekey_master(&w.database, &w.old, &w.new, now(), batch),
                Err(TenantError::Io),
                "a batch of {batch} rows"
            );
        }
        assert_eq!(
            changed(&before, &state(&w.db)),
            Vec::<&str>::new(),
            "no row is written"
        );
        assert_eq!(rotate(&w), Ok(()), "with a batch of rows it is made");
    }

    #[test]
    fn a_batch_of_no_row_is_refused_before_a_database_is_opened_for_the_first_time() {
        let w = world();
        assert_eq!(
            PgRegistry::rekey_master(&w.database, &w.old, &w.new, now(), 0),
            Err(TenantError::Io),
            "a statement must read a row"
        );
        assert_eq!(
            named(&w.db),
            Vec::<String>::new(),
            "the refusal comes before the first open, which would name the old key"
        );
        assert_eq!(
            log_keys(&w.db),
            Vec::<String>::new(),
            "and would make the log's key"
        );
    }

    #[test]
    fn an_open_registry_is_not_rotated_with_a_batch_of_no_row() {
        let w = world();
        fill(&w);
        let registry = open(&w, &w.old).expect("the registry opens");
        let before = state(&w.db);
        assert_eq!(
            registry.rotate(&w.old, &w.new, now(), 0),
            Err(TenantError::Io),
            "a cursor that reads no row would move the key's id and no value"
        );
        assert_eq!(
            changed(&before, &state(&w.db)),
            Vec::<&str>::new(),
            "no row is written"
        );
        assert!(open(&w, &w.old).is_ok(), "it opens under its key as before");
    }

    /// A rotation over a registry in which one value of `table.column`,
    /// in the row `row`, does not open: it stops, nothing of it is kept,
    /// and once the value is what it was the rotation is made.
    fn a_value_that_does_not_open_stops_the_rotation(table: &str, column: &str, row: &str) {
        let w = world();
        fill(&w);
        let rows = filled(&w);
        flip(&w.db, table, column, row);
        let before = state(&w.db);
        assert_eq!(
            rotate(&w),
            Err(TenantError::Io),
            "{table}.{column} does not open under the old key"
        );
        assert_eq!(
            changed(&before, &state(&w.db)),
            Vec::<&str>::new(),
            "nothing is kept of a rotation that stopped at {table}.{column}"
        );
        assert_eq!(
            named(&w.db),
            [hex(&catalog_key(&w.old).id().0)],
            "the database still names the old key"
        );
        flip(&w.db, table, column, row);
        assert_eq!(
            rotate(&w),
            Ok(()),
            "with the value as it was, the rotation is made"
        );
        assert_eq!(
            read(&open(&w, &w.new).expect("opens under the new key")),
            rows,
            "and every row reads as it did"
        );
    }

    fn tenant_is(n: u8) -> String {
        format!("id = {}", lit(&[n; 16]))
    }

    #[test]
    fn a_tenant_name_that_does_not_open_stops_the_rotation() {
        a_value_that_does_not_open_stops_the_rotation("tenant", "name_sealed", &tenant_is(3));
    }

    #[test]
    fn a_retention_that_does_not_open_stops_the_rotation() {
        a_value_that_does_not_open_stops_the_rotation("tenant", "retention_sealed", &tenant_is(1));
    }

    #[test]
    fn a_metadata_level_that_does_not_open_stops_the_rotation() {
        a_value_that_does_not_open_stops_the_rotation("tenant", "metadata_sealed", &tenant_is(3));
    }

    #[test]
    fn a_role_cap_that_does_not_open_stops_the_rotation() {
        a_value_that_does_not_open_stops_the_rotation("tenant", "role_cap_sealed", &tenant_is(1));
    }

    #[test]
    fn a_tenant_key_that_does_not_open_stops_the_rotation() {
        a_value_that_does_not_open_stops_the_rotation(
            "tenant",
            "wrapped_tenant_key",
            &tenant_is(2),
        );
    }

    #[test]
    fn a_pending_tenant_key_that_does_not_open_stops_the_rotation() {
        a_value_that_does_not_open_stops_the_rotation(
            "tenant",
            "pending_wrapped_tenant_key",
            &tenant_is(1),
        );
    }

    #[test]
    fn an_api_key_s_scopes_that_do_not_open_stop_the_rotation() {
        let row = format!("id = {}", lit(&[2; 8]));
        a_value_that_does_not_open_stops_the_rotation("api_key", "scopes_sealed", &row);
    }

    #[test]
    fn an_api_key_s_label_that_does_not_open_stops_the_rotation() {
        let row = format!("id = {}", lit(&[1; 8]));
        a_value_that_does_not_open_stops_the_rotation("api_key", "label_sealed", &row);
    }

    #[test]
    fn a_user_s_label_that_does_not_open_stops_the_rotation() {
        a_value_that_does_not_open_stops_the_rotation(
            "user_account",
            "label_sealed",
            &tenant_is(2),
        );
    }

    #[test]
    fn a_user_s_identity_that_does_not_open_stops_the_rotation() {
        a_value_that_does_not_open_stops_the_rotation(
            "user_account",
            "identity_sealed",
            &tenant_is(3),
        );
    }

    #[test]
    fn an_operator_s_roles_that_do_not_open_stop_the_rotation() {
        let row = format!("user_id = {} AND tenant IS NULL", lit(&[1; 16]));
        a_value_that_does_not_open_stops_the_rotation("membership", "roles_sealed", &row);
    }

    #[test]
    fn a_setting_that_does_not_open_stops_the_rotation() {
        let row = format!("tenant = {} AND name = 'retention.days'", lit(&[2; 16]));
        a_value_that_does_not_open_stops_the_rotation("tenant_setting", "value_sealed", &row);
    }

    #[test]
    fn a_session_s_scopes_that_do_not_open_stop_the_rotation() {
        let row = format!("id_hash = {}", lit(&[0xa1; 32]));
        a_value_that_does_not_open_stops_the_rotation("session", "scopes_sealed", &row);
    }

    #[test]
    fn a_session_s_methods_that_do_not_open_stop_the_rotation() {
        let row = format!("id_hash = {}", lit(&[0xa2; 32]));
        a_value_that_does_not_open_stops_the_rotation("session", "amr_sealed", &row);
    }

    #[test]
    fn a_newest_log_key_that_does_not_open_stops_the_rotation() {
        let w = world();
        fill(&w);
        flip(&w.db, "admin_audit_key", "wrapped", "ordinal = 1");
        let before = state(&w.db);
        assert_eq!(
            rotate(&w),
            Err(TenantError::Io),
            "the key that seals the rotation's record does not open"
        );
        assert_eq!(
            changed(&before, &state(&w.db)),
            Vec::<&str>::new(),
            "no row is written"
        );
        flip(&w.db, "admin_audit_key", "wrapped", "ordinal = 1");
        assert_eq!(rotate(&w), Ok(()), "with the key as it was, it is made");
    }

    #[test]
    fn a_registry_whose_log_has_no_key_is_not_rotated() {
        let w = world();
        drop(open(&w, &w.old).expect("the first open"));
        as_owner(&w.db, "DELETE FROM vcon.admin_audit_key");
        let before = state(&w.db);
        assert_eq!(
            rotate(&w),
            Err(TenantError::Io),
            "a registry that would not open is not moved"
        );
        assert_eq!(
            changed(&before, &state(&w.db)),
            Vec::<&str>::new(),
            "no row is written"
        );
    }

    /// One tenant under `master` in a database that names no key and
    /// whose log has none, as a registry of before schema version 12
    /// left it.
    fn before_version_12(w: &World, master: &Key) {
        let registry = open(w, master).expect("the registry opens");
        let (row, _, _) = tenant(master, 1, "alpha-tenant-name", true, true);
        write(&registry, |tx| tx.insert_tenant(&row).expect("a tenant"));
        as_owner(&w.db, "DELETE FROM vcon.registry_key");
        as_owner(&w.db, "DELETE FROM vcon.admin_audit_key");
    }

    #[test]
    fn a_database_that_names_no_key_with_rows_under_a_third_is_refused() {
        let w = world();
        let stranger = Key::generate().expect("a key");
        before_version_12(&w, &stranger);
        let before = state(&w.db);
        assert_eq!(
            rotate(&w),
            Err(TenantError::WrongKey),
            "the tenant opens under neither key"
        );
        assert_eq!(
            changed(&before, &state(&w.db)),
            Vec::<&str>::new(),
            "no id is written and no key of the log made"
        );
    }

    #[test]
    fn a_commit_that_is_rolled_back_keeps_nothing() {
        let w = world();
        fill(&w);
        let rows = filled(&w);
        let registry = open(&w, &w.old).expect("the registry opens");
        let before = state(&w.db);
        registry.fail_commit(1, CommitError::RolledBack);
        assert_eq!(
            registry.rotate(&w.old, &w.new, now(), BATCH),
            Err(TenantError::Io),
            "the commit failed"
        );
        assert_eq!(
            changed(&before, &state(&w.db)),
            Vec::<&str>::new(),
            "the one transaction kept nothing"
        );
        assert_eq!(
            registry.rotate(&w.old, &w.new, now(), BATCH),
            Ok(()),
            "the next commit is kept"
        );
        assert_eq!(
            read(&open(&w, &w.new).expect("opens under the new key")),
            rows,
            "and every row reads as it did"
        );
    }

    #[test]
    fn a_sealed_value_moved_to_another_row_stops_the_rotation() {
        let w = world();
        fill(&w);
        as_owner(
            &w.db,
            &format!(
                "UPDATE vcon.tenant SET name_sealed = \
                 (SELECT name_sealed FROM vcon.tenant WHERE {}) WHERE {}",
                tenant_is(1),
                tenant_is(3)
            ),
        );
        let before = state(&w.db);
        assert_eq!(
            rotate(&w),
            Err(TenantError::Io),
            "a value is bound to its row"
        );
        assert_eq!(
            changed(&before, &state(&w.db)),
            Vec::<&str>::new(),
            "no row is written"
        );
    }

    #[test]
    fn a_tenant_key_wrapped_for_another_tenant_stops_the_rotation() {
        let w = world();
        fill(&w);
        as_owner(
            &w.db,
            &format!(
                "UPDATE vcon.tenant SET wrapped_tenant_key = \
                 (SELECT wrapped_tenant_key FROM vcon.tenant WHERE {}) WHERE {}",
                tenant_is(2),
                tenant_is(3)
            ),
        );
        let before = state(&w.db);
        assert_eq!(
            rotate(&w),
            Err(TenantError::Io),
            "a wrapped key is bound to its tenant"
        );
        assert_eq!(
            changed(&before, &state(&w.db)),
            Vec::<&str>::new(),
            "no row is written"
        );
    }

    #[test]
    fn a_registry_moved_to_a_third_key_while_the_rotation_waited_is_refused() {
        let w = world();
        fill(&w);
        let registry = open(&w, &w.old).expect("the registry opens");
        as_owner(
            &w.db,
            "UPDATE vcon.registry_key SET catalog_key_id = '\\x0102030405060708'",
        );
        let before = state(&w.db);
        assert_eq!(
            registry.rotate(&w.old, &w.new, now(), BATCH),
            Err(TenantError::WrongKey),
            "the id is compared again with the lock held"
        );
        assert_eq!(
            changed(&before, &state(&w.db)),
            Vec::<&str>::new(),
            "no row is written"
        );
    }

    // The edges.

    #[test]
    fn an_empty_registry_is_rotated() {
        let w = world();
        drop(open(&w, &w.old).expect("the first open"));
        assert_eq!(rotate(&w), Ok(()), "a registry with no row");
        assert_eq!(
            named(&w.db),
            [hex(&catalog_key(&w.new).id().0)],
            "the database names the new key"
        );
        assert_eq!(
            uses(&w.db),
            [format!("{} 0 {}", hex(&w.new.id().0), nanos(now()))],
            "the new master key has wrapped no key yet"
        );
        assert_eq!(records(&w.db).len(), 1, "the rotation is recorded");
        assert!(open(&w, &w.new).is_ok(), "it opens under the new key");
    }

    #[test]
    fn a_database_no_registry_has_opened_is_rotated() {
        let w = world();
        assert_eq!(named(&w.db), Vec::<String>::new(), "no key is named");
        assert_eq!(rotate(&w), Ok(()), "the first open is the rotation's");
        assert_eq!(
            named(&w.db),
            [hex(&catalog_key(&w.new).id().0)],
            "the database names the new key"
        );
        assert_eq!(log_keys(&w.db).len(), 1, "the log has its key");
        assert!(open(&w, &w.new).is_ok(), "it opens under the new key");
        assert_eq!(
            open(&w, &w.old).err(),
            Some(TenantError::WrongKey),
            "and not under the old"
        );
    }

    #[test]
    fn rows_of_before_version_12_under_the_old_key_are_rotated() {
        let w = world();
        before_version_12(&w, &w.old);
        assert_eq!(rotate(&w), Ok(()), "the tenant opens under the old key");
        assert_eq!(
            named(&w.db),
            [hex(&catalog_key(&w.new).id().0)],
            "the database names the new key"
        );
        let registry = open(&w, &w.new).expect("opens under the new key");
        let mut tx = registry.snapshot().expect("a read begins");
        assert_eq!(
            tx.tenant(tenant_id(1)).map(|t| t.map(|t| t.name)),
            Ok(Some("alpha-tenant-name".to_owned())),
            "the tenant reads under the new key"
        );
        assert_eq!(uses(&w.db).len(), 1, "one master key is in use");
    }

    #[test]
    fn rows_of_before_version_12_under_the_new_key_are_left_and_named() {
        let w = world();
        before_version_12(&w, &w.new);
        let tenants = table(&w.db, "tenant");
        assert_eq!(rotate(&w), Ok(()), "the registry is under the new key");
        assert_eq!(
            table(&w.db, "tenant"),
            tenants,
            "the tenant is not written again"
        );
        assert_eq!(
            named(&w.db),
            [hex(&catalog_key(&w.new).id().0)],
            "the open that found it names the key"
        );
        assert_eq!(
            records(&w.db),
            Vec::<String>::new(),
            "no rotation was made, so none is recorded"
        );
    }

    #[test]
    fn a_value_that_is_null_stays_null() {
        let w = world();
        fill(&w);
        let nulls = |db: &Db| {
            as_owner(
                db,
                &format!(
                    "SELECT retention_sealed IS NULL, metadata_sealed IS NULL, \
                     role_cap_sealed IS NULL, pending_wrapped_tenant_key IS NULL \
                     FROM vcon.tenant WHERE {}",
                    tenant_is(2)
                ),
            )
        };
        assert_eq!(nulls(&w.db), ["t t t t"], "the second tenant has none");
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        assert_eq!(nulls(&w.db), ["t t t t"], "and has none after");
        assert_eq!(
            uses(&w.db),
            [format!("{} 4 {}", hex(&w.new.id().0), nanos(now()))],
            "a pending key that is not there is not counted"
        );
    }

    #[test]
    fn a_user_with_no_identity_keeps_none() {
        let w = world();
        fill(&w);
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        assert_eq!(
            as_owner(
                &w.db,
                &format!(
                    "SELECT identity_mac IS NULL, identity_sealed IS NULL \
                     FROM vcon.user_account WHERE {}",
                    tenant_is(2)
                ),
            ),
            ["t t"],
            "no HMAC and no sealed copy are made of nothing"
        );
        let registry = open(&w, &w.new).expect("opens under the new key");
        let mut tx = registry.snapshot().expect("a read begins");
        assert_eq!(
            tx.user(user_id(2)).map(|u| u.map(|u| u.identity)),
            Ok(Some(None)),
            "the user reads with no identity"
        );
    }

    #[test]
    fn an_operator_s_membership_reads_after() {
        let w = world();
        fill(&w);
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        let registry = open(&w, &w.new).expect("opens under the new key");
        let mut tx = registry.snapshot().expect("a read begins");
        assert_eq!(
            tx.membership(user_id(1), None).map(|m| m.map(|m| m.roles)),
            Ok(Some("roles-of-1".to_owned())),
            "the row with no tenant was found by its user and sealed again"
        );
    }

    #[test]
    fn an_older_log_key_that_does_not_open_is_left_as_it_is() {
        let w = world();
        fill(&w);
        as_owner(&w.db, "UPDATE vcon.admin_audit_key SET ordinal = 2");
        as_owner(
            &w.db,
            "INSERT INTO vcon.admin_audit_key (key_id, ordinal, wrapped) \
             VALUES ('\\x0a0b0c0d0e0f0a0b', 1, decode(repeat('5a', 61), 'hex'))",
        );
        assert_eq!(rotate(&w), Ok(()), "the newest key opens");
        let keys = log_keys(&w.db);
        assert_eq!(keys.len(), 2, "two keys still");
        assert_eq!(
            keys[0],
            format!("1 0a0b0c0d0e0f0a0b {}", "5a".repeat(61)),
            "the key that did not open is not written"
        );
        let registry = open(&w, &w.new).expect("opens under the new key");
        let text = exported(&registry).expect("the log is read");
        assert!(text.contains("target-one"), "the newest key reads: {text}");
    }

    #[test]
    fn two_users_that_share_an_identity_share_its_new_hmac() {
        let w = world();
        fill(&w);
        let macs = |db: &Db| {
            as_owner(
                db,
                "SELECT encode(identity_mac, 'hex') FROM vcon.user_account \
                 WHERE identity_mac IS NOT NULL ORDER BY id",
            )
        };
        let before = macs(&w.db);
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        let after = macs(&w.db);
        assert_eq!(after.len(), 2, "the user and the deactivated one");
        assert_eq!(after[0], after[1], "one identity, one HMAC");
        assert_ne!(after[0], before[0], "under the new index key");
    }

    #[test]
    fn a_batch_of_as_many_rows_as_a_table_has_moves_them_all() {
        let w = world();
        fill(&w);
        let before = filled(&w);
        assert_eq!(
            PgRegistry::rekey_master(&w.database, &w.old, &w.new, now(), 3),
            Ok(()),
            "three tenants, three users, three memberships, three settings"
        );
        assert_eq!(
            read(&open(&w, &w.new).expect("opens under the new key")),
            before,
            "every row was moved"
        );
    }

    #[test]
    fn a_batch_of_one_row_fewer_than_a_table_has_moves_them_all() {
        let w = world();
        fill(&w);
        let before = filled(&w);
        assert_eq!(
            PgRegistry::rekey_master(&w.database, &w.old, &w.new, now(), 2),
            Ok(()),
            "the third row is in a second batch, and a table of two rows in one"
        );
        assert_eq!(
            read(&open(&w, &w.new).expect("opens under the new key")),
            before,
            "every row was moved"
        );
        assert_eq!(
            uses(&w.db),
            [format!("{} 4 {}", hex(&w.new.id().0), nanos(now()))],
            "the keys of both batches are counted"
        );
    }

    #[test]
    fn the_use_rows_of_other_master_keys_are_removed() {
        let w = world();
        fill(&w);
        let registry = open(&w, &w.old).expect("the registry opens");
        write(&registry, |tx| {
            tx.add_master_wrap(w.old.id(), T0).expect("the old key's");
            tx.add_master_wrap(KeyId([0xee; 8]), T0)
                .expect("a key of an older rotation");
        });
        assert_eq!(uses(&w.db).len(), 2, "two keys in use before");
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        assert_eq!(
            uses(&w.db),
            [format!("{} 4 {}", hex(&w.new.id().0), nanos(now()))],
            "the new master key alone is in use"
        );
    }

    #[test]
    fn two_rotations_at_once_make_one() {
        let w = world();
        fill(&w);
        let before = filled(&w);
        let results: Vec<Result<(), TenantError>> = std::thread::scope(|scope| {
            let runs: Vec<_> = (0..2).map(|_| scope.spawn(|| rotate(&w))).collect();
            runs.into_iter()
                .map(|run| run.join().expect("a rotation's thread"))
                .collect()
        });
        assert_eq!(results, [Ok(()), Ok(())], "both end well");
        let registry = open(&w, &w.new).expect("opens under the new key");
        assert_eq!(read(&registry), before, "the rows read as they did");
        let mut tx = registry.snapshot().expect("a read begins");
        assert_eq!(
            tx.record_times(Action::RekeyMaster, 200).map(|t| t.len()),
            Ok(1),
            "one rotation was made: the other found it done"
        );
    }

    #[test]
    fn a_rotation_to_the_key_the_registry_is_under_writes_nothing() {
        let w = world();
        fill(&w);
        let before = state(&w.db);
        assert_eq!(
            PgRegistry::rekey_master(&w.database, &w.old, &w.old, now(), BATCH),
            Ok(()),
            "the registry is under the new key already"
        );
        assert_eq!(
            PgRegistry::rekey_master(&w.database, &w.new, &w.old, now(), BATCH),
            Ok(()),
            "whatever the old key is said to be"
        );
        assert_eq!(
            changed(&before, &state(&w.db)),
            Vec::<&str>::new(),
            "no row is written and nothing recorded"
        );
    }

    #[test]
    fn every_sealed_column_of_the_registry_is_in_the_rotation_s_list() {
        let w = world();
        let mut there = as_owner(
            &w.db,
            "SELECT c.table_name || '.' || c.column_name FROM information_schema.columns c \
             WHERE c.table_schema = 'vcon' \
               AND c.table_name IN (SELECT tablename::text FROM pg_policies \
                   WHERE schemaname = 'vcon' AND policyname = 'registry_rows') \
               AND (c.column_name LIKE '%\\_sealed' OR c.column_name LIKE '%\\_mac' \
                    OR c.column_name LIKE '%wrapped%') \
             ORDER BY 1",
        );
        // The log's records are never written again, and the rotation
        // seals the log's keys with the function a tenant's log has.
        let apart = [
            "admin_audit.target_mac",
            "admin_audit.target_sealed",
            "admin_audit_key.wrapped",
        ];
        for name in apart {
            assert!(
                there.iter().any(|c| c == name),
                "{name} is a column of the schema"
            );
        }
        there.retain(|c| !apart.contains(&c.as_str()));
        let mut listed: Vec<String> = REMADE
            .iter()
            .flat_map(|spec| {
                spec.columns
                    .iter()
                    .flat_map(written)
                    .map(move |column| format!("{}.{column}", spec.table))
            })
            .collect();
        listed.sort();
        assert_eq!(
            listed, there,
            "a column the schema seals and the rotation does not make again \
             could not be read after one"
        );
    }

    #[test]
    fn a_tenant_s_own_rows_are_not_written() {
        let w = world();
        fill(&w);
        let (tenant, _key, _catalog) = tenant_catalog(&w.database);
        let own = |db: &Db| -> Vec<String> {
            ["tenant_catalog", "audit_key", "key_use", "audit"]
                .iter()
                .map(|name| {
                    let sql = format!(
                        "SELECT coalesce(md5(string_agg(t::text, ';' ORDER BY t::text)), 'none') \
                         FROM vcon.{name} t"
                    );
                    wait(async {
                        let admin = db.admin().await;
                        admin
                            .batch_execute("SET ROLE vcon_owner")
                            .await
                            .expect("the owner role");
                        in_scope(&admin, &tenant.to_string(), &sql)
                            .await
                            .unwrap_or_else(|e| panic!("{sql}: {e}"))
                            .iter()
                            .find_map(|m| match m {
                                tokio_postgres::SimpleQueryMessage::Row(row) => {
                                    row.get(0).map(str::to_owned)
                                }
                                _ => None,
                            })
                            .unwrap_or_else(|| panic!("{sql}: no row"))
                    })
                })
                .collect()
        };
        let before = own(&w.db);
        assert_ne!(before[0], "none", "the tenant has its catalog row");
        assert_ne!(before[1], "none", "and its log's key");
        assert_eq!(rotate(&w), Ok(()), "the rotation is made");
        assert_eq!(
            own(&w.db),
            before,
            "a tenant's catalog is under its tenant key, which the rotation only wraps again"
        );
    }

    #[test]
    fn a_commit_with_no_answer_that_was_kept_is_found_done_by_the_next_run() {
        let w = world();
        fill(&w);
        let rows = filled(&w);
        let registry = open(&w, &w.old).expect("the registry opens");
        registry.fail_commit(1, CommitError::Unknown);
        assert_eq!(
            registry.rotate(&w.old, &w.new, now(), BATCH),
            Err(TenantError::Io),
            "the commit got no answer"
        );
        let before = state(&w.db);
        assert_eq!(rotate(&w), Ok(()), "the next run finds the rotation made");
        assert_eq!(
            changed(&before, &state(&w.db)),
            Vec::<&str>::new(),
            "and writes nothing"
        );
        assert_eq!(
            read(&open(&w, &w.new).expect("opens under the new key")),
            rows,
            "every row reads as it did"
        );
    }

    #[test]
    fn a_rotation_waits_for_a_write_of_the_registry_to_end() {
        let w = world();
        fill(&w);
        let registry = open(&w, &w.old).expect("the registry opens");
        let mut first = registry.immediate().expect("a write begins");
        let (row, key, _) = tenant(&w.old, 7, "eta-tenant-name", true, false);
        first.insert_tenant(&row).expect("inserts");
        let (done, rotated) = mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(|| done.send(rotate(&w)).expect("the test waits"));
            assert_eq!(
                rotated.recv_timeout(Duration::from_millis(400)),
                Err(mpsc::RecvTimeoutError::Timeout),
                "the rotation has not ended while the write holds the lock"
            );
            first.commit().expect("the write commits");
            assert_eq!(
                rotated.recv_timeout(Duration::from_secs(30)),
                Ok(Ok(())),
                "the rotation is made once the write has ended"
            );
        });
        let wraps = wraps(&w, &w.new);
        let (tenant, wrapped, _) = wraps.last().expect("the tenant the write added");
        assert_eq!(*tenant, tenant_id(7), "the write's tenant is there");
        let opened = envelope::unwrap(&w.new, &Context::Tenant { tenant: *tenant }, wrapped)
            .expect("its key was wrapped again with the others");
        assert_eq!(opened.id(), key, "and is the key it had");
    }

    #[test]
    fn a_rotation_that_stopped_gives_up_the_lock() {
        let w = world();
        fill(&w);
        let row = format!("id_hash = {}", lit(&[0xa2; 32]));
        flip(&w.db, "session", "amr_sealed", &row);
        assert_eq!(
            rotate(&w),
            Err(TenantError::Io),
            "a value of the last table does not open"
        );
        let registry = open(&w, &w.old).expect("the registry opens under the old key");
        let (extra, _, _) = tenant(&w.old, 8, "theta-tenant-name", true, false);
        write(&registry, |tx| {
            tx.insert_tenant(&extra).expect("a write is taken");
        });
        assert_eq!(wraps(&w, &w.old).len(), 4, "the write was kept");
        flip(&w.db, "session", "amr_sealed", &row);
        assert_eq!(
            rotate(&w),
            Ok(()),
            "and with the value as it was, the rotation is made"
        );
    }

    #[test]
    fn a_rotation_that_found_the_registry_moved_while_it_waited_writes_nothing() {
        let w = world();
        fill(&w);
        let registry = open(&w, &w.old).expect("the registry opens");
        assert_eq!(rotate(&w), Ok(()), "another run makes the rotation");
        let before = state(&w.db);
        assert_eq!(
            registry.rotate(&w.old, &w.new, now(), BATCH),
            Ok(()),
            "the id, compared with the lock held, is the new key's"
        );
        assert_eq!(
            changed(&before, &state(&w.db)),
            Vec::<&str>::new(),
            "no row is written a second time"
        );
    }
}
