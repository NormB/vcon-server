//! The catalog's schema: the migrations that make it, the command that
//! applies them (`vcon-server migrate`), and the checks a new connection
//! makes against what is installed.
//!
//! - A server never migrates: with several instances, one starting up would
//!   change the schema under the others. It refuses a database at any
//!   other version than the binary's, on every new connection.
//! - `migrate` runs alone, holding the two advisory locks every server and
//!   command will share, with the role `vcon_owner` it reaches through
//!   `SET ROLE`, one transaction per schema version. That role is never
//!   given to a server: a serving connection is refused when its role owns
//!   the catalog or is a member of the role that does.

use tokio_postgres::error::SqlState;
use tokio_postgres::{Client, SimpleQueryMessage, SimpleQueryRow};

use super::connect::with_causes;

/// The migrations, in order; the version of each is its position, from 1.
pub(crate) const MIGRATIONS: &[&str] = &[
    include_str!("schema_v1.sql"),
    include_str!("schema_v2.sql"),
    include_str!("schema_v3.sql"),
    include_str!("schema_v4.sql"),
    include_str!("schema_v5.sql"),
    include_str!("schema_v6.sql"),
];

/// The schema version this binary makes and reads:
/// [`MIGRATIONS`]`.len()`, a test holds them equal.
pub(crate) const SCHEMA_VERSION: i32 = 6;

/// The advisory lock every server takes, a constant all instances share.
pub(crate) const SERVERS_LOCK: i64 = 0x7663_6f6e_5352_0001;

/// The advisory lock every command takes.
pub(crate) const COMMANDS_LOCK: i64 = 0x7663_6f6e_434d_0001;

/// What `migrate` did: the schema version found, and the one it left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Migrated {
    /// The installed version before; 0 for none.
    pub(crate) from: i32,
    /// The installed version after.
    pub(crate) to: i32,
}

impl Migrated {
    /// The line `migrate` prints.
    #[must_use]
    pub(crate) fn message(self) -> String {
        if self.from == self.to {
            format!("already at schema version {}", self.to)
        } else {
            format!("schema version {} to schema version {}", self.from, self.to)
        }
    }

    /// The JSON line `migrate` prints: both versions, whether the schema
    /// changed, and [`Migrated::message`].
    #[must_use]
    pub(crate) fn line(self) -> serde_json::Value {
        serde_json::json!({
            "migrated": self.from != self.to,
            "from": self.from,
            "to": self.to,
            "message": self.message(),
        })
    }
}

/// Why a connection is refused for the schema version `installed` (`None`
/// for no schema, or none recorded), or `None` when it is the binary's.
#[must_use]
pub(crate) fn version_refusal(installed: Option<i32>) -> Option<String> {
    match installed {
        None => Some("the database has no vCon schema: run vcon-server migrate".to_owned()),
        Some(n) if n == SCHEMA_VERSION => None,
        Some(n) if n < SCHEMA_VERSION => Some(format!(
            "the database schema is version {n}, older than this binary's {SCHEMA_VERSION}: run \
             vcon-server migrate"
        )),
        Some(n) => Some(format!(
            "the database schema is newer than this binary (version {n}, this binary \
             {SCHEMA_VERSION}): upgrade this server"
        )),
    }
}

/// Why `migrate` may not go on: `owner` and `app` say that the two roles
/// exist, and `can_set` that the login role `user` may `SET ROLE
/// vcon_owner`. `None` when it may.
#[must_use]
pub(crate) fn role_refusal(user: &str, owner: bool, app: bool, can_set: bool) -> Option<String> {
    if !owner {
        return Some(
            "the role vcon_owner does not exist; create it as the how-to shows, with no login"
                .to_owned(),
        );
    }
    if !app {
        return Some(
            "the role vcon_app does not exist; create it as the how-to shows, for the server to \
             log in as"
                .to_owned(),
        );
    }
    if !can_set {
        return Some(format!(
            "the role {user:?} cannot act as vcon_owner, which migrate needs; run GRANT \
             vcon_owner TO {user:?} as a superuser, for a role the server does not use"
        ));
    }
    None
}

/// A row, its columns as text (`None` for NULL).
type Row = Vec<Option<String>>;

fn is_true(row: &Row, i: usize) -> bool {
    row.get(i).and_then(Option::as_deref) == Some("t")
}

/// The first row of the answer to `sql`, or why the server cannot be
/// checked.
async fn checked_row(client: &Client, sql: &str) -> Result<Row, String> {
    let messages = client
        .simple_query(sql)
        .await
        .map_err(|e| format!("the server cannot be checked: {}", with_causes(&e)))?;
    let row = first_row(&messages)
        .ok_or_else(|| "the server cannot be checked: no row came back".to_owned())?;
    Ok((0..row.len())
        .map(|i| row.get(i).map(str::to_owned))
        .collect())
}

/// The checks of a serving connection: its role does not own the catalog
/// and cannot act as its owner, and the schema is this binary's.
pub(crate) async fn serving_checks(client: &Client, user: &str) -> Result<(), String> {
    const OWNS: &str = "SELECT \
        EXISTS (SELECT 1 FROM pg_tables t WHERE t.schemaname = 'vcon' \
                AND pg_has_role(current_user, t.tableowner, 'MEMBER')) \
        OR EXISTS (SELECT 1 FROM pg_namespace n WHERE n.nspname = 'vcon' \
                AND pg_has_role(current_user, n.nspowner, 'MEMBER'))";
    let owns = checked_row(client, OWNS).await?;
    if is_true(&owns, 0) {
        return Err(format!(
            "the role {user:?} owns the catalog's tables or is a member of the role that does; \
             a server logs in as vcon_app"
        ));
    }
    let installed = installed_version(client, user).await?;
    match version_refusal(installed) {
        Some(why) => Err(why),
        None => Ok(()),
    }
}

/// The schema version `vcon.installed_version()` reports: `None` when no
/// version is recorded or the schema is absent.
async fn installed_version(client: &Client, user: &str) -> Result<Option<i32>, String> {
    let row = match client.simple_query("SELECT vcon.installed_version()").await {
        Ok(messages) => messages,
        Err(e) => {
            return match e.code() {
                Some(&SqlState::INVALID_SCHEMA_NAME) | Some(&SqlState::UNDEFINED_FUNCTION) => {
                    Ok(None)
                }
                Some(&SqlState::INSUFFICIENT_PRIVILEGE) => Err(format!(
                    "the role {user:?} may not use the vCon schema; make it a member of \
                     vcon_app, or log in as vcon_app"
                )),
                _ => Err(format!("the server cannot be checked: {}", with_causes(&e))),
            };
        }
    };
    let text = first_row(&row).and_then(|r| r.get(0).map(str::to_owned));
    text.map_or(Ok(None), |v| {
        v.parse().map(Some).map_err(|_| {
            "the server cannot be checked: the schema version is not a number".to_owned()
        })
    })
}

/// The checks of `migrate`'s connection: both roles exist and the login
/// role may act as `vcon_owner`.
pub(crate) async fn migrating_checks(client: &Client, user: &str) -> Result<(), String> {
    const ROLES: &str = "SELECT to_regrole('vcon_owner') IS NOT NULL, \
        to_regrole('vcon_app') IS NOT NULL, \
        CASE WHEN to_regrole('vcon_owner') IS NULL THEN false \
             ELSE pg_has_role(current_user, 'vcon_owner', 'SET') END";
    let row = checked_row(client, ROLES).await?;
    match role_refusal(user, is_true(&row, 0), is_true(&row, 1), is_true(&row, 2)) {
        Some(why) => Err(why),
        None => Ok(()),
    }
}

/// Applies `migrations` the database has not yet, alone and as
/// `vcon_owner`, each in one transaction. It leaves the connection's
/// session changed (`SET ROLE` and the locks), so the caller closes it.
pub(crate) async fn run(client: &mut Client, migrations: &[&str]) -> Result<Migrated, String> {
    take_locks(client).await?;
    client
        .batch_execute("SET ROLE vcon_owner")
        .await
        .map_err(|e| format!("the owner role cannot be used: {}", statement_error(&e)))?;
    let from = installed_for_migrate(client).await?;
    let latest = i32::try_from(migrations.len()).expect("the migrations number under 2^31");
    if from > latest {
        return Err(format!(
            "the database schema is newer than this binary (version {from}, this binary {latest}): \
             upgrade this server"
        ));
    }
    for (index, sql) in migrations
        .iter()
        .enumerate()
        .skip(from.try_into().unwrap_or(usize::MAX))
    {
        let version = i32::try_from(index + 1).expect("the migrations number under 2^31");
        apply(client, version, sql).await?;
    }
    Ok(Migrated {
        from,
        to: latest.max(from),
    })
}

/// The two advisory locks, tried without waiting: a server or command that
/// holds either, exclusive or shared, refuses the migration.
async fn take_locks(client: &Client) -> Result<(), String> {
    let row = checked_row(
        client,
        &format!(
            "SELECT pg_try_advisory_lock({SERVERS_LOCK}), pg_try_advisory_lock({COMMANDS_LOCK})"
        ),
    )
    .await?;
    if is_true(&row, 0) && is_true(&row, 1) {
        Ok(())
    } else {
        Err(
            "another server or command holds the database's locks; stop every server and \
             command and run migrate again"
                .to_owned(),
        )
    }
}

/// The version recorded, or 0 for a database no migration has touched.
async fn installed_for_migrate(client: &Client) -> Result<i32, String> {
    let exists = checked_row(
        client,
        "SELECT to_regclass('vcon.schema_version') IS NOT NULL",
    )
    .await?;
    if !is_true(&exists, 0) {
        return Ok(0);
    }
    let row = checked_row(
        client,
        "SELECT coalesce(max(version), 0) FROM vcon.schema_version",
    )
    .await?;
    row.first()
        .and_then(Option::as_deref)
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| "the schema version recorded is not a number".to_owned())
}

/// One migration in one transaction: a failure leaves nothing of it.
async fn apply(client: &mut Client, version: i32, sql: &str) -> Result<(), String> {
    let tx = client
        .transaction()
        .await
        .map_err(|e| format!("version {version}: {}", statement_error(&e)))?;
    tx.batch_execute(sql).await.map_err(|e| {
        if e.code() == Some(&SqlState::DUPLICATE_SCHEMA) {
            "a schema named vcon exists that vcon-server did not make; remove it, or use \
             another database"
                .to_owned()
        } else {
            format!("version {version}: {}", statement_error(&e))
        }
    })?;
    tx.execute(
        "INSERT INTO vcon.schema_version (version) VALUES ($1)",
        &[&version],
    )
    .await
    .map_err(|e| format!("version {version}: {}", statement_error(&e)))?;
    tx.commit()
        .await
        .map_err(|e| format!("version {version}: {}", statement_error(&e)))
}

/// What the server said of a statement, or the error itself. The statement
/// text is not repeated: it is the binary's own and holds no secret, but a
/// failure need not print a page of SQL.
fn statement_error(e: &tokio_postgres::Error) -> String {
    match e.as_db_error() {
        Some(db) => format!("{} (SQLSTATE {})", db.message(), db.code().code()),
        None => with_causes(e),
    }
}

fn first_row(messages: &[SimpleQueryMessage]) -> Option<&SimpleQueryRow> {
    messages.iter().find_map(|m| match m {
        SimpleQueryMessage::Row(row) => Some(row),
        _ => None,
    })
}
