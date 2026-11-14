//! The advisory locks a process holds on the catalog database for as long
//! as it runs, in place of the data directory's lock files: a server, a
//! command that runs beside a server, and a command that runs alone. The
//! live tests need a PostgreSQL server.

use super::super::schema::{COMMANDS_LOCK, Holds, SERVERS_LOCK, lock_refusal, lock_text};
use crate::datadir::Sharing;

// What must work.

#[test]
fn a_server_tries_the_servers_lock_alone_and_not_shared() {
    assert_eq!(
        lock_text(Holds::Serve),
        format!("SELECT pg_try_advisory_lock({SERVERS_LOCK}), true"),
        "one instance serves a database in this release"
    );
}

#[test]
fn a_command_beside_a_server_shares_the_commands_lock() {
    assert_eq!(
        lock_text(Holds::Beside),
        format!("SELECT true, pg_try_advisory_lock_shared({COMMANDS_LOCK})"),
        "with other such commands, and takes nothing a server holds"
    );
}

#[test]
fn a_command_alone_tries_both_locks_and_shares_neither() {
    assert_eq!(
        lock_text(Holds::Alone),
        format!(
            "SELECT pg_try_advisory_lock({SERVERS_LOCK}), pg_try_advisory_lock({COMMANDS_LOCK})"
        ),
        "no server and no other command"
    );
}

#[test]
fn locks_that_were_given_are_no_refusal() {
    for holds in [Holds::Serve, Holds::Beside, Holds::Alone] {
        assert_eq!(lock_refusal(holds, true, true), None, "{holds:?}");
    }
}

#[test]
fn the_kind_of_work_on_the_data_directory_is_the_kind_on_the_database() {
    assert_eq!(Holds::from(Sharing::Serve), Holds::Serve, "a server");
    assert_eq!(Holds::from(Sharing::Beside), Holds::Beside, "beside one");
    assert_eq!(Holds::from(Sharing::Alone), Holds::Alone, "alone");
}

// What must be refused.

#[test]
fn a_server_is_told_what_may_hold_the_database() {
    let why = lock_refusal(Holds::Serve, false, true).expect("refused");
    assert_eq!(
        why,
        "another server, a command that runs alone, or a migration holds this database's \
         lock; one server serves a database, and those commands run while none does",
        "who it may be, and the rule"
    );
}

#[test]
fn a_command_beside_a_server_is_told_to_wait_for_the_one_that_runs_alone() {
    let why = lock_refusal(Holds::Beside, true, false).expect("refused");
    assert_eq!(
        why,
        "a command that runs alone, or a migration, holds this database's lock; run this \
         command again when it has ended",
        "who it may be, and what to do"
    );
}

#[test]
fn a_command_alone_is_told_to_stop_the_servers_and_wait_for_the_commands() {
    for (servers, commands) in [(false, true), (true, false), (false, false)] {
        let why = lock_refusal(Holds::Alone, servers, commands).expect("refused");
        assert_eq!(
            why,
            "a server or another command holds this database's locks; this command runs \
             alone: stop every server, wait for the other commands to end, and run it again",
            "servers {servers}, commands {commands}"
        );
    }
}

// The edges.

#[test]
fn no_lock_is_waited_for() {
    for holds in [Holds::Serve, Holds::Beside, Holds::Alone] {
        let text = lock_text(holds);
        assert_eq!(
            text.matches("pg_try_advisory_lock").count(),
            text.matches("advisory").count(),
            "{holds:?} tries, and is refused at once: {text}"
        );
    }
}

#[test]
fn a_lock_that_was_not_asked_for_is_not_a_refusal() {
    // The statement answers `true` in the column of a lock it does not
    // take; a `false` there cannot come from the server.
    assert_eq!(
        lock_refusal(Holds::Serve, true, false),
        None,
        "a server takes nothing on the commands' lock"
    );
    assert_eq!(
        lock_refusal(Holds::Beside, false, true),
        None,
        "a command beside a server takes nothing on the servers' lock"
    );
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use std::sync::Arc;

    use super::super::super::catalog::Database;
    use super::super::super::schema::{
        COMMANDS_LOCK, Holds, MIGRATIONS, SERVERS_LOCK, take_locks_for,
    };
    use super::super::live::value;
    use super::super::schema_tests::live::Db;
    use crate::config::StorageError;

    /// Runs `call` on a runtime of its own, for a test's own connections.
    fn wait<T>(call: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(call)
    }

    /// A migrated database of the test's own.
    fn migrated() -> Db {
        wait(Db::migrated())
    }

    /// Connects to `db` as the application role for the work `holds`.
    fn connect(db: &Db, holds: Holds) -> Result<Arc<Database>, String> {
        let (_dir, settings) = db.settings("vcon_app");
        Database::connect_as(&settings, holds).map_err(|StorageError(why)| why)
    }

    /// The advisory locks held in the database, as `key:mode` in order.
    fn held(db: &Db) -> String {
        wait(async {
            value(
                &db.admin().await,
                "SELECT coalesce(string_agg(((l.classid::bigint << 32) | l.objid::bigint) \
                 || ':' || l.mode, ',' ORDER BY l.objid DESC, l.mode), '') \
                 FROM pg_locks l WHERE l.locktype = 'advisory' AND l.granted AND l.database = \
                 (SELECT oid FROM pg_database WHERE datname = current_database())",
            )
            .await
        })
    }

    const SERVER_REFUSED: &str = "catalog.postgres: another server, a command that runs alone, \
                                  or a migration holds this database's lock";
    const BESIDE_REFUSED: &str =
        "catalog.postgres: a command that runs alone, or a migration, holds this database's lock";
    const ALONE_REFUSED: &str =
        "catalog.postgres: a server or another command holds this database's locks";

    // What must work.

    #[test]
    fn a_command_runs_beside_a_server() {
        let db = migrated();
        let _server = connect(&db, Holds::Serve).expect("the server");
        let _command = connect(&db, Holds::Beside).expect("a command beside it");
        assert_eq!(
            held(&db),
            format!("{SERVERS_LOCK}:ExclusiveLock,{COMMANDS_LOCK}:ShareLock"),
            "each holds its own lock"
        );
    }

    #[test]
    fn a_server_starts_while_a_command_runs_beside() {
        let db = migrated();
        let _command = connect(&db, Holds::Beside).expect("a command");
        connect(&db, Holds::Serve).expect("the server starts");
    }

    #[test]
    fn two_commands_run_beside_each_other() {
        let db = migrated();
        let _one = connect(&db, Holds::Beside).expect("one");
        let _two = connect(&db, Holds::Beside).expect("another");
    }

    #[test]
    fn a_command_runs_alone_when_nothing_else_does_and_holds_both_locks() {
        let db = migrated();
        let _command = connect(&db, Holds::Alone).expect("alone");
        assert_eq!(
            held(&db),
            format!("{SERVERS_LOCK}:ExclusiveLock,{COMMANDS_LOCK}:ExclusiveLock"),
            "the servers' lock and the commands' lock"
        );
    }

    #[test]
    fn a_command_alone_runs_once_the_server_has_stopped() {
        let db = migrated();
        let server = connect(&db, Holds::Serve).expect("the server");
        server.give_up_lock();
        connect(&db, Holds::Alone).expect("alone, after the server");
    }

    #[test]
    fn a_server_starts_once_the_command_alone_has_ended() {
        let db = migrated();
        let command = connect(&db, Holds::Alone).expect("alone");
        command.give_up_lock();
        assert_eq!(held(&db), "", "both locks are given up");
        connect(&db, Holds::Serve).expect("the server, after the command");
    }

    // What must be refused.

    #[test]
    fn a_command_alone_is_refused_while_a_server_runs() {
        let db = migrated();
        let _server = connect(&db, Holds::Serve).expect("the server");
        let why = connect(&db, Holds::Alone).expect_err("refused");
        assert!(why.starts_with(ALONE_REFUSED), "says who holds it: {why}");
    }

    #[test]
    fn a_command_alone_is_refused_while_a_command_runs_beside() {
        let db = migrated();
        let _command = connect(&db, Holds::Beside).expect("a command");
        let why = connect(&db, Holds::Alone).expect_err("refused");
        assert!(why.starts_with(ALONE_REFUSED), "says who holds it: {why}");
    }

    #[test]
    fn a_second_command_alone_is_refused() {
        let db = migrated();
        let _command = connect(&db, Holds::Alone).expect("alone");
        let why = connect(&db, Holds::Alone).expect_err("refused");
        assert!(why.starts_with(ALONE_REFUSED), "says who holds it: {why}");
    }

    #[test]
    fn a_server_is_refused_while_a_command_runs_alone() {
        let db = migrated();
        let _command = connect(&db, Holds::Alone).expect("alone");
        let why = connect(&db, Holds::Serve).expect_err("refused");
        assert!(why.starts_with(SERVER_REFUSED), "says who holds it: {why}");
    }

    #[test]
    fn a_command_beside_is_refused_while_a_command_runs_alone() {
        let db = migrated();
        let _command = connect(&db, Holds::Alone).expect("alone");
        let why = connect(&db, Holds::Beside).expect_err("refused");
        assert!(why.starts_with(BESIDE_REFUSED), "says who holds it: {why}");
    }

    #[test]
    fn migrate_is_refused_while_a_command_runs_beside() {
        let db = migrated();
        let _command = connect(&db, Holds::Beside).expect("a command");
        let why = wait(db.migrate_with(MIGRATIONS)).expect_err("refused");
        assert!(
            why.contains("another server or command holds the database's locks"),
            "says so: {why}"
        );
    }

    #[test]
    fn a_command_beside_is_refused_while_migrate_holds_the_locks() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let db = runtime.block_on(Db::migrated());
        let _holder = runtime.block_on(async {
            let holder = db.admin().await;
            holder
                .batch_execute(&format!(
                    "SELECT pg_advisory_lock({SERVERS_LOCK}), pg_advisory_lock({COMMANDS_LOCK})"
                ))
                .await
                .expect("holds the locks migrate takes");
            holder
        });
        let why = connect(&db, Holds::Beside).expect_err("refused");
        assert!(why.starts_with(BESIDE_REFUSED), "says who holds it: {why}");
    }

    // The edges.

    #[tokio::test]
    async fn a_refused_work_gives_up_the_lock_it_got_while_its_session_is_open() {
        // A session that ends frees its locks, and that is seen some time
        // after: the refused work gives up what it got before it ends.
        let db = Db::migrated().await;
        let holder = db.admin().await;
        holder
            .batch_execute(&format!("SELECT pg_advisory_lock_shared({COMMANDS_LOCK})"))
            .await
            .expect("holds the commands' lock, as a command beside does");
        let client = db.app().await;
        let why = take_locks_for(&client, Holds::Alone)
            .await
            .expect_err("refused the commands' lock");
        assert!(why.starts_with("a server or another command"), "{why}");
        assert_eq!(
            value(
                &client,
                "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND granted \
                 AND pid = pg_backend_pid()"
            )
            .await,
            "0",
            "the session that was refused holds no lock, and is still open"
        );
    }

    #[test]
    fn a_command_alone_that_is_refused_one_lock_keeps_neither() {
        let db = migrated();
        let _command = connect(&db, Holds::Beside).expect("a command beside");
        connect(&db, Holds::Alone).expect_err("refused the commands' lock");
        assert_eq!(
            held(&db),
            format!("{COMMANDS_LOCK}:ShareLock"),
            "the servers' lock it was given is given up"
        );
        connect(&db, Holds::Serve).expect("so a server starts");
    }

    #[test]
    fn a_server_takes_nothing_on_the_commands_lock() {
        let db = migrated();
        let _server = connect(&db, Holds::Serve).expect("the server");
        assert_eq!(
            held(&db),
            format!("{SERVERS_LOCK}:ExclusiveLock"),
            "the servers' lock alone"
        );
    }

    #[test]
    fn a_command_beside_takes_nothing_on_the_servers_lock() {
        let db = migrated();
        let _command = connect(&db, Holds::Beside).expect("a command");
        assert_eq!(
            held(&db),
            format!("{COMMANDS_LOCK}:ShareLock"),
            "the commands' lock alone, shared"
        );
    }

    #[test]
    fn a_command_beside_that_ends_gives_its_lock_up() {
        let db = migrated();
        let command = connect(&db, Holds::Beside).expect("a command");
        command.give_up_lock();
        assert_eq!(held(&db), "", "nothing is held");
        connect(&db, Holds::Alone).expect("so a command runs alone");
    }

    #[test]
    fn a_process_that_ends_without_a_word_frees_its_locks() {
        let db = migrated();
        // Dropped with its lock held, as a process that is killed leaves
        // it: PostgreSQL frees the locks when it sees the session closed.
        drop(connect(&db, Holds::Alone).expect("alone"));
        let mut freed = false;
        for _ in 0..100 {
            if held(&db).is_empty() {
                freed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(freed, "within five seconds of the session's end");
        connect(&db, Holds::Alone).expect("the next command runs");
    }

    #[test]
    fn the_connection_that_serves_is_the_one_that_connect_makes() {
        let db = migrated();
        let (_dir, settings) = db.settings("vcon_app");
        let _server = Database::connect(&settings).expect("a server");
        assert_eq!(
            held(&db),
            format!("{SERVERS_LOCK}:ExclusiveLock"),
            "connect is a server's connect"
        );
    }
}
