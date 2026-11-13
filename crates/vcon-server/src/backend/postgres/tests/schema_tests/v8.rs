//! Schema version 8: the server numbers every row that has a `seq`,
//! whatever the statement says, and the two audit logs take new records
//! and one more in the count of a counted record, and nothing else, from
//! the application role and from the owner. Each live test of the
//! migration starts from a database at version 7. Version 9 (v9.rs)
//! replaced "one more" with "a larger count"; the live tests here that
//! start from a migrated database hold the rule as it is now.

use super::super::super::schema::{MIGRATIONS, SCHEMA_VERSION, version_refusal};

/// The position of this version's migration.
const THIS: usize = 7;

/// The tables whose rows the server numbers.
const NUMBERED: [&str; 4] = ["version", "manifest", "audit", "admin_audit"];

/// The two audit logs.
const LOGS: [&str; 2] = ["audit", "admin_audit"];

/// This version's migration, or a panic that says there is none.
fn text() -> &'static str {
    assert!(
        MIGRATIONS.len() > THIS,
        "there are {} migrations",
        MIGRATIONS.len()
    );
    MIGRATIONS[THIS]
}

// What must work.

#[test]
fn version_8_is_the_eighth_migration_and_numbers_the_rows() {
    assert!(
        text().contains("CREATE FUNCTION vcon.number_row() RETURNS trigger"),
        "the eighth makes the function that numbers a row"
    );
}

#[test]
fn the_binary_s_version_is_at_least_8() {
    assert!(
        usize::try_from(SCHEMA_VERSION).expect("positive") > THIS,
        "this binary's version: {SCHEMA_VERSION}"
    );
}

#[test]
fn every_numbered_table_is_named_once_for_the_numbering_trigger() {
    let named = format!(
        "ARRAY[{}]",
        NUMBERED.map(|table| format!("'{table}'")).join(", ")
    );
    assert_eq!(
        text().matches(&named).count(),
        1,
        "the four tables with a seq, in one list: {named}"
    );
}

#[test]
fn both_logs_are_named_once_for_the_append_only_triggers() {
    let named = format!(
        "ARRAY[{}]",
        LOGS.map(|table| format!("'{table}'")).join(", ")
    );
    assert_eq!(
        text().matches(&named).count(),
        1,
        "the tenant's log and the admin log, in one list: {named}"
    );
}

// What must be refused.

#[test]
fn a_version_7_database_is_refused_with_the_command_that_upgrades_it() {
    let why = version_refusal(Some(7)).expect("refused");
    assert!(
        why.contains(&format!(
            "version 7, older than this binary's {SCHEMA_VERSION}"
        )) && why.contains("vcon-server migrate"),
        "the versions and the command: {why}"
    );
}

#[test]
fn no_role_is_granted_anything_by_version_8() {
    assert!(
        !text()
            .lines()
            .any(|line| line.trim_start().to_uppercase().starts_with("GRANT ")),
        "the triggers need no grant, and the application role gains none"
    );
}

// The limits and odd inputs.

#[test]
fn version_8_changes_no_row_by_a_statement_and_makes_no_table() {
    // The owner role is held to row-level security too, so an UPDATE or
    // DELETE in a migration would raise. A statement begins its line.
    for statement in [
        "UPDATE ",
        "DELETE ",
        "DROP ",
        "TRUNCATE ",
        "CREATE TABLE ",
        "ALTER TABLE ",
    ] {
        assert!(
            !text()
                .lines()
                .any(|line| line.trim_start().to_uppercase().starts_with(statement)),
            "no {statement}in version 8"
        );
    }
}

#[test]
fn each_function_of_version_8_has_a_fixed_search_path() {
    let functions = text().matches("CREATE FUNCTION ").count();
    assert_eq!(functions, 2, "the numbering and the append-only rule");
    assert_eq!(
        text()
            .matches("SET search_path = pg_catalog, pg_temp")
            .count(),
        functions,
        "a function reads no name from the caller's search path"
    );
}

#[test]
fn only_the_numbering_runs_as_its_owner() {
    assert_eq!(
        text().matches("SECURITY DEFINER").count(),
        1,
        "the sequence is the owner's; the append-only rule needs no privilege"
    );
}

#[test]
fn the_functions_are_revoked_from_public() {
    assert!(
        text().contains(
            "REVOKE ALL ON FUNCTION vcon.number_row(), vcon.audit_append_only() FROM PUBLIC;"
        ),
        "no role is given the functions"
    );
}

#[cfg(feature = "backend-tests")]
pub(super) mod on_a_server {
    use tokio_postgres::Client;

    use super::super::super::super::schema::{MIGRATIONS, Migrated, SCHEMA_VERSION};
    use super::super::live::{Db, TENANT_A, TENANT_B, in_scope};
    use super::{LOGS, NUMBERED, THIS};

    /// The server's words when a record would be changed.
    /// Version 9 changed them with the rule.
    pub(in super::super) const NOT_CHANGED: &str =
        "an audit record is not changed, but for a larger count of a counted record";
    /// The server's words when a record would be deleted.
    pub(in super::super) const NOT_DELETED: &str = "an audit record is not deleted";
    /// The server's words when a log would be emptied.
    pub(in super::super) const NOT_EMPTIED: &str = "an audit log is not emptied";

    /// A database at version 7.
    async fn at_version_7() -> Db {
        let db = Db::fresh().await;
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..THIS])
                .await
                .expect("version 7"),
            Migrated { from: 0, to: 7 },
            "the first seven migrations alone"
        );
        db
    }

    /// The tester's connection as the role that owns the schema.
    pub(in super::super) async fn owner(db: &Db) -> Client {
        let client = db.admin().await;
        client
            .batch_execute("SET ROLE vcon_owner")
            .await
            .expect("the owner role");
        client
    }

    /// The scope a transaction on `table` runs in: the registry's for the
    /// admin log, the first tenant's for the others.
    pub(in super::super) fn scope(table: &str) -> &'static str {
        if table == "admin_audit" {
            "registry"
        } else {
            TENANT_A
        }
    }

    /// The first value of the first row `sql` gives `client` in `scope`.
    pub(in super::super) async fn one(client: &Client, scope: &str, sql: &str) -> String {
        in_scope(client, scope, sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
            .iter()
            .find_map(|m| match m {
                tokio_postgres::SimpleQueryMessage::Row(r) => {
                    Some(r.get(0).unwrap_or("NULL").to_owned())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{sql}: no row"))
    }

    /// The server's message refusing `sql`.
    pub(in super::super) async fn refused(client: &Client, scope: &str, sql: &str) -> String {
        match in_scope(client, scope, sql).await {
            Ok(_) => panic!("not refused: {sql}"),
            Err(e) => e,
        }
    }

    /// An INSERT of one row into `table` for `tenant`, with ` RETURNING
    /// seq`; `seq` is the number the statement asks for, written with
    /// OVERRIDING SYSTEM VALUE, and `counted` marks an audit record.
    fn insert(table: &str, tenant: &str, seq: Option<&str>, counted: bool) -> String {
        let (columns, values) = match table {
            "version" => (
                "tenant, version, uuid_mac, form_sealed, gzip_sealed, size_sealed, stored_at, \
                 tenant_key_id, wrapped_key"
                    .to_owned(),
                format!(
                    "'\\x{tenant}', gen_random_uuid()::text::bytea, '\\x02', '\\x03', '\\x04', \
                     '\\x05', 1, '\\x06', '\\x07'"
                ),
            ),
            "manifest" => (
                "tenant, published_at".to_owned(),
                format!("'\\x{tenant}', 1"),
            ),
            "audit" => (
                "tenant, at, actor, action, status, count, counted".to_owned(),
                format!("'\\x{tenant}', 't', 'a', 'b', 1, 1, {counted}"),
            ),
            "admin_audit" => (
                "at, actor, action, status, count, counted".to_owned(),
                format!("'t', 'a', 'b', 1, 1, {counted}"),
            ),
            other => panic!("no insert for {other}"),
        };
        match seq {
            Some(seq) => format!(
                "INSERT INTO vcon.{table} ({columns}, seq) OVERRIDING SYSTEM VALUE \
                 VALUES ({values}, {seq}) RETURNING seq"
            ),
            None => format!("INSERT INTO vcon.{table} ({columns}) VALUES ({values}) RETURNING seq"),
        }
    }

    /// Stores one row in `table` as the application role and gives its
    /// number.
    async fn store(db: &Db, table: &str, counted: bool) -> i64 {
        one(
            &db.app().await,
            scope(table),
            &insert(table, TENANT_A, None, counted),
        )
        .await
        .parse()
        .expect("a number")
    }

    /// The count of the one record of `table`, read by the owner.
    pub(in super::super) async fn count_of(db: &Db, table: &str) -> String {
        one(
            &owner(db).await,
            scope(table),
            &format!("SELECT count FROM vcon.{table}"),
        )
        .await
    }

    /// How many rows `table` holds, read by the owner in the table's scope.
    pub(in super::super) async fn rows(db: &Db, table: &str) -> String {
        one(
            &owner(db).await,
            scope(table),
            &format!("SELECT count(*) FROM vcon.{table}"),
        )
        .await
    }

    /// How many triggers of this version the database has.
    pub(in super::super) async fn triggers(db: &Db) -> String {
        one(
            &db.admin().await,
            TENANT_A,
            "SELECT count(*) FROM pg_trigger WHERE NOT tgisinternal AND tgname IN \
             ('number_rows', 'append_only', 'not_emptied')",
        )
        .await
    }

    // What must work.

    #[tokio::test]
    async fn a_version_7_database_is_brought_to_version_8_with_its_rows_as_they_were() {
        let db = at_version_7().await;
        // A server is refused at version 7, so the tester writes the rows.
        let admin = db.admin().await;
        let mut before = Vec::new();
        for table in NUMBERED {
            let seq = one(&admin, scope(table), &insert(table, TENANT_A, None, false)).await;
            before.push(seq);
        }
        assert_eq!(
            triggers(&db).await,
            "0",
            "version 7 has none of the triggers"
        );
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: 7,
                to: SCHEMA_VERSION
            },
            "migrate brings it to this binary's version"
        );
        let mut after = Vec::new();
        for table in NUMBERED {
            let sql = format!("SELECT seq FROM vcon.{table}");
            after.push(one(&admin, scope(table), &sql).await);
        }
        assert_eq!(after, before, "each row kept its number");
        assert_eq!(
            triggers(&db).await,
            "8",
            "four number the rows, and each log has one against changes and one against emptying"
        );
    }

    #[tokio::test]
    async fn rows_stored_after_the_migration_take_numbers_after_the_ones_before() {
        let db = at_version_7().await;
        let admin = db.admin().await;
        let mut before = Vec::new();
        for table in NUMBERED {
            let seq = one(&admin, scope(table), &insert(table, TENANT_A, None, false)).await;
            before.push(seq.parse::<i64>().expect("a number"));
        }
        db.migrate().await;
        for (table, before) in NUMBERED.iter().zip(before) {
            let after = store(&db, table, false).await;
            assert!(
                after > before,
                "{table}: {after} is after {before}, the sequence went on"
            );
        }
    }

    #[tokio::test]
    async fn the_server_numbers_each_row_in_the_order_stored() {
        let db = Db::migrated().await;
        for table in NUMBERED {
            let first = store(&db, table, false).await;
            let second = store(&db, table, false).await;
            let third = store(&db, table, false).await;
            assert!(
                first < second && second < third,
                "{table}: {first}, {second}, {third} rise"
            );
        }
    }

    #[tokio::test]
    async fn the_application_role_adds_one_to_a_counted_record() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            let app = db.app().await;
            let update = format!("UPDATE vcon.{table} SET count = count + 1");
            in_scope(&app, scope(table), &update)
                .await
                .expect("adds one");
            in_scope(&app, scope(table), &update)
                .await
                .expect("and one more");
            assert_eq!(
                count_of(&db, table).await,
                "3",
                "{table}: one, and one twice"
            );
        }
    }

    #[tokio::test]
    async fn the_owner_adds_one_to_a_counted_record_too() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            in_scope(
                &owner(&db).await,
                scope(table),
                &format!("UPDATE vcon.{table} SET count = count + 1"),
            )
            .await
            .expect("the one change a record takes");
            assert_eq!(count_of(&db, table).await, "2", "{table}");
        }
    }

    #[tokio::test]
    async fn an_update_that_writes_the_other_columns_as_they_are_is_one_more_in_the_count() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            in_scope(
                &owner(&db).await,
                scope(table),
                &format!(
                    "UPDATE vcon.{table} SET count = count + 1, at = at, version = NULL, \
                     target_mac = NULL, counted = true"
                ),
            )
            .await
            .expect("nothing but the count is different");
            assert_eq!(count_of(&db, table).await, "2", "{table}");
        }
    }

    #[tokio::test]
    async fn the_application_role_reads_the_records_it_wrote_in_their_order() {
        let db = Db::migrated().await;
        for table in LOGS {
            let first = store(&db, table, false).await;
            let second = store(&db, table, true).await;
            let read = one(
                &db.app().await,
                scope(table),
                &format!("SELECT string_agg(seq::text, ',' ORDER BY seq) FROM vcon.{table}"),
            )
            .await;
            assert_eq!(read, format!("{first},{second}"), "{table}");
        }
    }

    #[tokio::test]
    async fn a_new_database_has_the_triggers() {
        let db = Db::migrated().await;
        assert_eq!(
            triggers(&db).await,
            "8",
            "migrate from nothing makes them too"
        );
    }

    #[tokio::test]
    async fn every_column_the_server_numbers_has_the_numbering_trigger() {
        let db = Db::migrated().await;
        let numbered = "FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
            JOIN pg_namespace n ON n.oid = c.relnamespace \
            WHERE n.nspname = 'vcon' AND c.relkind = 'r' AND a.attnum > 0 AND NOT a.attisdropped \
              AND (a.attidentity <> '' OR EXISTS (SELECT 1 FROM pg_attrdef d \
                   WHERE d.adrelid = a.attrelid AND d.adnum = a.attnum \
                     AND pg_get_expr(d.adbin, d.adrelid) LIKE '%nextval(%'))";
        let admin = db.admin().await;
        assert_eq!(
            one(
                &admin,
                TENANT_A,
                &format!("SELECT string_agg(c.relname || '.' || a.attname, ',' ORDER BY c.relname) {numbered}"),
            )
            .await,
            "admin_audit.seq,audit.seq,manifest.seq,version.seq",
            "the columns a sequence fills: a new one needs the trigger and a place in this test"
        );
        assert_eq!(
            one(
                &admin,
                TENANT_A,
                &format!(
                    "SELECT count(*) {numbered} AND NOT EXISTS (SELECT 1 FROM pg_trigger t \
                     WHERE t.tgrelid = c.oid AND t.tgname = 'number_rows' AND NOT t.tgisinternal \
                       AND t.tgtype = 23)"
                ),
            )
            .await,
            "0",
            "each has the trigger, for each row, before INSERT and UPDATE"
        );
    }

    #[tokio::test]
    async fn the_owner_stores_a_record_as_the_application_role_does() {
        let db = Db::migrated().await;
        for table in LOGS {
            one(
                &owner(&db).await,
                scope(table),
                &insert(table, TENANT_A, None, false),
            )
            .await;
            assert_eq!(rows(&db, table).await, "1", "{table}: appended");
        }
    }

    #[tokio::test]
    async fn an_update_of_a_version_or_a_manifest_keeps_its_number() {
        let db = Db::migrated().await;
        for (table, change) in [
            ("version", "stored_at = 2"),
            ("manifest", "published_at = 2"),
        ] {
            let seq = store(&db, table, false).await;
            let after = one(
                &db.app().await,
                TENANT_A,
                &format!("UPDATE vcon.{table} SET {change} RETURNING seq"),
            )
            .await;
            assert_eq!(
                after,
                seq.to_string(),
                "{table}: the row is changed, not its number"
            );
        }
    }

    #[tokio::test]
    async fn migrating_twice_from_version_7_does_the_work_once() {
        let db = at_version_7().await;
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: 7,
                to: SCHEMA_VERSION
            },
            "the first run"
        );
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: SCHEMA_VERSION,
                to: SCHEMA_VERSION
            },
            "the second finds nothing to do"
        );
        assert_eq!(triggers(&db).await, "8", "and makes no trigger twice");
    }

    // What must be refused.

    #[tokio::test]
    async fn a_number_the_statement_chooses_is_not_kept() {
        let db = Db::migrated().await;
        for table in NUMBERED {
            let before = store(&db, table, false).await;
            for chosen in ["999999", "0", "-5"] {
                let got = one(
                    &db.app().await,
                    scope(table),
                    &insert(table, TENANT_A, Some(chosen), false),
                )
                .await;
                assert!(
                    got != chosen && got.parse::<i64>().expect("a number") > before,
                    "{table}: asked for {chosen}, numbered {got} after {before}"
                );
            }
        }
    }

    #[tokio::test]
    async fn a_number_an_earlier_row_holds_cannot_be_chosen_again() {
        let db = Db::migrated().await;
        for table in NUMBERED {
            let first = store(&db, table, false).await;
            let got = one(
                &db.app().await,
                scope(table),
                &insert(table, TENANT_A, Some(&first.to_string()), false),
            )
            .await;
            assert!(
                got.parse::<i64>().expect("a number") > first,
                "{table}: a second row is after {first}, not at it: {got}"
            );
            assert_eq!(rows(&db, table).await, "2", "{table}: both rows are there");
        }
    }

    #[tokio::test]
    async fn a_version_or_a_manifest_is_not_given_a_new_number() {
        let db = Db::migrated().await;
        for table in ["version", "manifest"] {
            let seq = store(&db, table, false).await;
            store(&db, table, false).await;
            let after = one(
                &db.app().await,
                TENANT_A,
                &format!("UPDATE vcon.{table} SET seq = DEFAULT WHERE seq = {seq} RETURNING seq"),
            )
            .await;
            assert_eq!(
                after,
                seq.to_string(),
                "{table}: the first row stays first, whatever DEFAULT would give"
            );
        }
    }

    #[tokio::test]
    async fn the_application_role_deletes_no_record() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, false).await;
            let e = refused(
                &db.app().await,
                scope(table),
                &format!("DELETE FROM vcon.{table}"),
            )
            .await;
            assert!(
                e.contains(&format!("permission denied for table {table}")),
                "{table}: {e}"
            );
            assert_eq!(rows(&db, table).await, "1", "{table}: the record is there");
        }
    }

    #[tokio::test]
    async fn the_owner_deletes_no_record() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, false).await;
            let e = refused(
                &owner(&db).await,
                scope(table),
                &format!("DELETE FROM vcon.{table}"),
            )
            .await;
            assert_eq!(e, NOT_DELETED, "{table}: the trigger holds the owner");
            assert_eq!(rows(&db, table).await, "1", "{table}: the record is there");
        }
    }

    #[tokio::test]
    async fn the_application_role_empties_no_log() {
        let db = Db::migrated().await;
        for table in LOGS {
            let e = refused(
                &db.app().await,
                scope(table),
                &format!("TRUNCATE vcon.{table}"),
            )
            .await;
            assert!(
                e.contains(&format!("permission denied for table {table}")),
                "{table}: {e}"
            );
        }
    }

    #[tokio::test]
    async fn the_owner_empties_no_log() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, false).await;
            let e = refused(
                &owner(&db).await,
                scope(table),
                &format!("TRUNCATE vcon.{table}"),
            )
            .await;
            assert_eq!(e, NOT_EMPTIED, "{table}: the trigger holds the owner");
            assert_eq!(rows(&db, table).await, "1", "{table}: the record is there");
        }
    }

    #[tokio::test]
    async fn the_owner_changes_no_column_of_a_record() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            let client = owner(&db).await;
            for change in [
                "at = 'later'",
                "actor = 'other'",
                "action = 'other'",
                "target_mac = '\\x01'",
                "target_sealed = '\\x01'",
                "version = 'v'",
                "status = 2",
                "status = NULL",
                "counted = false",
                "seq = DEFAULT",
            ] {
                for with_count in ["", ", count = count + 1"] {
                    let sql = format!("UPDATE vcon.{table} SET {change}{with_count}");
                    let e = refused(&client, scope(table), &sql).await;
                    assert_eq!(e, NOT_CHANGED, "{sql}");
                }
            }
            assert_eq!(
                count_of(&db, table).await,
                "1",
                "{table}: the count is as stored"
            );
        }
    }

    #[tokio::test]
    async fn the_owner_moves_no_record_to_another_tenant() {
        let db = Db::migrated().await;
        store(&db, "audit", true).await;
        let e = refused(
            &owner(&db).await,
            TENANT_A,
            &format!("UPDATE vcon.audit SET tenant = '\\x{TENANT_B}', count = count + 1"),
        )
        .await;
        assert_eq!(e, NOT_CHANGED, "the tenant is a column like the others");
    }

    #[tokio::test]
    async fn the_application_role_changes_no_column_but_the_count() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            let e = refused(
                &db.app().await,
                scope(table),
                &format!("UPDATE vcon.{table} SET actor = 'other'"),
            )
            .await;
            assert!(
                e.contains(&format!("permission denied for table {table}")),
                "{table}: {e}"
            );
        }
    }

    #[tokio::test]
    async fn the_count_of_a_record_that_is_not_counted_is_not_changed() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, false).await;
            let sql = format!("UPDATE vcon.{table} SET count = count + 1");
            for client in [db.app().await, owner(&db).await] {
                let e = refused(&client, scope(table), &sql).await;
                assert_eq!(e, NOT_CHANGED, "{table}");
            }
            assert_eq!(count_of(&db, table).await, "1", "{table}");
        }
    }

    #[tokio::test]
    async fn a_count_is_not_changed_to_itself_or_to_less() {
        // Version 8 took one more and nothing else; since version 9 a
        // count takes any larger number (v9.rs).
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            for to in ["count - 1", "count", "0", "1", "-1000", "count * 2 - 2"] {
                let sql = format!("UPDATE vcon.{table} SET count = {to}");
                for client in [db.app().await, owner(&db).await] {
                    let e = refused(&client, scope(table), &sql).await;
                    assert_eq!(e, NOT_CHANGED, "{sql}");
                }
            }
            assert_eq!(count_of(&db, table).await, "1", "{table}");
        }
    }

    #[tokio::test]
    async fn the_application_role_cannot_use_or_set_a_sequence() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for table in NUMBERED {
            for call in [
                format!("SELECT nextval('vcon.{table}_seq_seq')"),
                format!("SELECT setval('vcon.{table}_seq_seq', 1)"),
                format!("ALTER SEQUENCE vcon.{table}_seq_seq RESTART"),
            ] {
                let e = refused(&app, scope(table), &call).await;
                assert!(
                    e.contains("permission denied for sequence")
                        || e.contains("must be owner of sequence"),
                    "{call}: {e}"
                );
            }
        }
    }

    #[tokio::test]
    async fn the_application_role_cannot_remove_or_stop_a_trigger() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for table in LOGS {
            for statement in [
                format!("DROP TRIGGER append_only ON vcon.{table}"),
                format!("DROP TRIGGER number_rows ON vcon.{table}"),
                format!("ALTER TABLE vcon.{table} DISABLE TRIGGER ALL"),
                format!("ALTER TABLE vcon.{table} DISABLE TRIGGER not_emptied"),
            ] {
                let e = refused(&app, scope(table), &statement).await;
                assert!(
                    e.contains(&format!("must be owner of relation {table}"))
                        || e.contains(&format!("must be owner of table {table}")),
                    "{statement}: {e}"
                );
            }
        }
    }

    #[tokio::test]
    async fn the_application_role_cannot_replace_or_call_a_function() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for function in ["number_row", "audit_append_only"] {
            let e = refused(
                &app,
                TENANT_A,
                &format!(
                    "CREATE OR REPLACE FUNCTION vcon.{function}() RETURNS trigger \
                     LANGUAGE plpgsql AS 'BEGIN RETURN NEW; END'"
                ),
            )
            .await;
            assert!(
                e.contains("permission denied for schema vcon"),
                "{function}: {e}"
            );
            let e = refused(&app, TENANT_A, &format!("SELECT vcon.{function}()")).await;
            assert!(
                e.contains(&format!("permission denied for function {function}")),
                "{function}: {e}"
            );
        }
    }

    #[tokio::test]
    async fn the_application_role_cannot_turn_the_triggers_off_for_its_session() {
        let db = Db::migrated().await;
        let e = refused(
            &db.app().await,
            TENANT_A,
            "SET session_replication_role = replica",
        )
        .await;
        assert!(
            e.contains("permission denied to set parameter \"session_replication_role\""),
            "{e}"
        );
    }

    #[tokio::test]
    async fn a_version_8_that_fails_leaves_version_7_whole() {
        let db = at_version_7().await;
        let broken = format!("{}\nSELECT boom;", MIGRATIONS[THIS]);
        let mut migrations: Vec<&str> = MIGRATIONS[..THIS].to_vec();
        migrations.push(&broken);
        let e = db.migrate_with(&migrations).await.expect_err("fails");
        assert!(e.starts_with("version 8: "), "names the version: {e}");
        assert_eq!(
            triggers(&db).await,
            "0",
            "the triggers were rolled back with it"
        );
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: 7,
                to: SCHEMA_VERSION
            },
            "and the real migration then runs from 7"
        );
    }

    #[tokio::test]
    async fn a_tenant_adds_nothing_to_another_tenant_s_count() {
        let db = Db::migrated().await;
        store(&db, "audit", true).await;
        in_scope(
            &db.app().await,
            TENANT_B,
            "UPDATE vcon.audit SET count = count + 1",
        )
        .await
        .expect("finds no row of its own");
        assert_eq!(
            count_of(&db, "audit").await,
            "1",
            "row-level security still stands before the trigger"
        );
    }

    // The limits and odd inputs.

    #[tokio::test]
    async fn a_refused_change_among_several_rows_changes_none() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            store(&db, table, false).await;
            store(&db, table, true).await;
            let e = refused(
                &db.app().await,
                scope(table),
                &format!("UPDATE vcon.{table} SET count = count + 1"),
            )
            .await;
            assert_eq!(e, NOT_CHANGED, "{table}: one of the three is not counted");
            assert_eq!(
                one(
                    &owner(&db).await,
                    scope(table),
                    &format!("SELECT sum(count) FROM vcon.{table}"),
                )
                .await,
                "3",
                "{table}: the statement changed no count"
            );
        }
    }

    #[tokio::test]
    async fn only_the_counted_records_among_several_are_added_to() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            store(&db, table, false).await;
            in_scope(
                &db.app().await,
                scope(table),
                &format!("UPDATE vcon.{table} SET count = count + 1 WHERE counted"),
            )
            .await
            .expect("the statement the server sends");
            assert_eq!(
                one(
                    &owner(&db).await,
                    scope(table),
                    &format!("SELECT string_agg(count::text, ',' ORDER BY seq) FROM vcon.{table}"),
                )
                .await,
                "2,1",
                "{table}"
            );
        }
    }

    #[tokio::test]
    async fn a_delete_or_an_update_that_finds_no_record_is_no_error() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, false).await;
            let client = owner(&db).await;
            for statement in [
                format!("DELETE FROM vcon.{table} WHERE seq < 0"),
                format!("UPDATE vcon.{table} SET actor = 'other' WHERE seq < 0"),
            ] {
                in_scope(&client, scope(table), &statement)
                    .await
                    .expect("a row trigger runs for a row, and there is none");
            }
            assert_eq!(rows(&db, table).await, "1", "{table}: nothing changed");
        }
    }

    #[tokio::test]
    async fn an_empty_log_is_not_emptied_either() {
        let db = Db::migrated().await;
        for table in LOGS {
            let e = refused(
                &owner(&db).await,
                scope(table),
                &format!("TRUNCATE vcon.{table}"),
            )
            .await;
            assert_eq!(
                e, NOT_EMPTIED,
                "{table}: the statement is refused, not a row"
            );
        }
    }

    #[tokio::test]
    async fn emptying_a_log_with_another_table_is_refused() {
        let db = Db::migrated().await;
        let e = refused(
            &owner(&db).await,
            TENANT_A,
            "TRUNCATE vcon.key_use, vcon.audit, vcon.admin_audit RESTART IDENTITY CASCADE",
        )
        .await;
        assert_eq!(e, NOT_EMPTIED, "whatever else the statement names");
    }

    #[tokio::test]
    async fn a_record_stored_with_a_null_number_is_numbered() {
        let db = Db::migrated().await;
        for table in LOGS {
            let got = one(
                &db.app().await,
                scope(table),
                &insert(table, TENANT_A, Some("NULL"), false),
            )
            .await;
            assert!(
                got.parse::<i64>().is_ok_and(|seq| seq > 0),
                "{table}: {got}"
            );
        }
    }

    #[tokio::test]
    async fn a_write_rolled_back_leaves_a_number_unused_and_none_used_twice() {
        let db = Db::migrated().await;
        for table in LOGS {
            let first = store(&db, table, false).await;
            let app = db.app().await;
            let rolled = format!(
                "{}; SELECT 1 / 0",
                insert(table, TENANT_A, None, false).replace(" RETURNING seq", "")
            );
            let e = refused(&app, scope(table), &rolled).await;
            assert!(e.contains("division by zero"), "{e}");
            let next = store(&db, table, false).await;
            assert!(
                next > first + 1,
                "{table}: {next} skips the number the rolled-back record took after {first}"
            );
            assert_eq!(rows(&db, table).await, "2", "{table}");
        }
    }

    #[tokio::test]
    async fn the_records_of_two_tenants_take_numbers_from_one_sequence() {
        let db = Db::migrated().await;
        let app = db.app().await;
        let mut numbers = Vec::new();
        for tenant in [TENANT_A, TENANT_B, TENANT_A] {
            let seq = one(&app, tenant, &insert("audit", tenant, None, false)).await;
            numbers.push(seq.parse::<i64>().expect("a number"));
        }
        assert!(
            numbers[0] < numbers[1] && numbers[1] < numbers[2],
            "the order is the database's: {numbers:?}"
        );
    }

    #[tokio::test]
    async fn an_insert_that_would_change_a_record_on_conflict_finds_no_conflict() {
        let db = Db::migrated().await;
        let first = store(&db, "audit", false).await;
        let statement = format!(
            "INSERT INTO vcon.audit (tenant, at, actor, action, status, count, seq) \
             OVERRIDING SYSTEM VALUE VALUES ('\\x{TENANT_A}', 't', 'a', 'b', 1, 1, {first}) \
             ON CONFLICT (tenant, seq) DO UPDATE SET count = vcon.audit.count + 1 RETURNING seq"
        );
        let got = one(&db.app().await, TENANT_A, &statement).await;
        assert!(
            got.parse::<i64>().expect("a number") > first,
            "the new record took its own number: {got}"
        );
        assert_eq!(
            one(
                &owner(&db).await,
                TENANT_A,
                "SELECT string_agg(count::text, ',' ORDER BY seq) FROM vcon.audit",
            )
            .await,
            "1,1",
            "and the first record's count is as stored"
        );
    }

    #[tokio::test]
    async fn a_count_at_the_largest_number_is_not_added_to() {
        let db = Db::migrated().await;
        let statement = format!(
            "INSERT INTO vcon.audit (tenant, at, actor, action, status, count, counted) \
             VALUES ('\\x{TENANT_A}', 't', 'a', 'b', 1, 9223372036854775807, true)"
        );
        in_scope(&db.app().await, TENANT_A, &statement)
            .await
            .expect("stores");
        let e = refused(
            &db.app().await,
            TENANT_A,
            "UPDATE vcon.audit SET count = count + 1",
        )
        .await;
        assert!(e.contains("bigint out of range"), "{e}");
    }
}
