//! The leases of the work one server does for every server of a database
//! (lease.rs): the holder's name and the statements, which need no
//! server, and then, live, what two holders on one database see.

use std::time::Duration;

use super::super::lease::{GIVE_UP, HOST_LEN, TAKE, holder, host_text, name, new_holder, seconds};
use crate::backend::Job;

/// The holder's random part, the 32 hex digits before the `@`.
fn random_part(holder: &str) -> &str {
    holder.split('@').next().expect("a part before the @")
}

// What must work.

#[test]
fn a_holder_is_its_random_bits_in_hex_then_the_host_and_the_process() {
    let random = [
        0x00, 0x01, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ];
    assert_eq!(
        holder(random, "db-1.example.com", 4242),
        "00012233445566778899aabbccddeeff@db-1.example.com:4242",
        "the random bits, the host and the process id"
    );
}

#[test]
fn each_new_holder_has_random_bits_of_its_own() {
    let (first, second) = (new_holder(), new_holder());
    assert_ne!(
        random_part(&first),
        random_part(&second),
        "two connects of one process are two holders"
    );
    assert_eq!(random_part(&first).len(), 32, "128 bits: {first}");
    assert!(
        random_part(&first)
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
        "lowercase hex: {first}"
    );
}

#[test]
fn a_new_holder_names_this_process() {
    let made = new_holder();
    assert!(
        made.ends_with(&format!(":{}", std::process::id())),
        "the process id last: {made}"
    );
    let host = made
        .split_once('@')
        .and_then(|(_, rest)| rest.rsplit_once(':'))
        .map(|(host, _)| host)
        .expect("a host");
    assert!(!host.is_empty(), "a host name: {made}");
}

#[test]
fn a_ttl_is_given_to_the_database_in_seconds() {
    assert!(
        (seconds(Duration::from_secs(600)) - 600.0).abs() < f64::EPSILON,
        "ten minutes"
    );
    assert!(
        (seconds(Duration::from_millis(1_500)) - 1.5).abs() < f64::EPSILON,
        "a second and a half"
    );
}

#[test]
fn each_job_has_a_row_of_its_own() {
    assert_eq!(name(Job::Sweep), "sweep", "the purge sweep's row");
    assert_eq!(name(Job::Pack), "pack", "the pack run's row");
}

#[test]
fn a_lease_is_taken_and_renewed_by_the_database_s_clock_alone() {
    assert!(
        TAKE.contains("now() + make_interval(secs => $3)"),
        "the end is the database's now plus the ttl: {TAKE}"
    );
    assert!(
        TAKE.contains("vcon.lease.expires_at <= now()"),
        "another's lease is taken once it has ended by the database's clock: {TAKE}"
    );
    assert!(
        !TAKE.contains("$4") && !TAKE.contains("clock_timestamp"),
        "no time from the host, and one clock reading per transaction: {TAKE}"
    );
}

// What must be refused.

#[test]
fn another_holder_s_lease_is_taken_only_once_it_has_ended() {
    assert!(
        TAKE.contains(
            "WHERE vcon.lease.holder = EXCLUDED.holder OR vcon.lease.expires_at <= now()"
        ),
        "the renewal's and the take's condition: {TAKE}"
    );
    assert!(
        TAKE.ends_with("RETURNING holder"),
        "a row when held: {TAKE}"
    );
}

#[test]
fn a_lease_is_given_up_only_by_its_holder() {
    assert_eq!(
        GIVE_UP, "DELETE FROM vcon.lease WHERE name = $1 AND holder = $2",
        "the job and the holder both"
    );
}

#[test]
fn a_host_name_is_never_carried_with_bytes_an_operator_s_terminal_would_run() {
    assert_eq!(
        host_text(b"db 1\x1b[31m\n\xff"),
        "db_1__31m__",
        "each byte outside letters, digits, dot, dash and underscore is an underscore"
    );
}

// The edges.

#[test]
fn a_host_with_no_name_is_unknown() {
    assert_eq!(host_text(b""), "unknown", "an empty name");
}

#[test]
fn a_long_host_name_is_cut_to_its_first_64_bytes() {
    let long = [b'a'; 300];
    assert_eq!(host_text(&long).len(), HOST_LEN, "cut to {HOST_LEN}");
    assert_eq!(HOST_LEN, 64, "a host name's label is at most 63 bytes");
    let exact = [b'b'; 64];
    assert_eq!(host_text(&exact), "b".repeat(64), "64 bytes are kept whole");
}

#[test]
fn a_host_name_of_every_byte_it_keeps_is_kept_as_it_is() {
    assert_eq!(
        host_text(b"Az09.-_"),
        "Az09.-_",
        "letters, digits, dot, dash and underscore"
    );
}

#[test]
fn a_ttl_of_no_time_is_zero_seconds() {
    assert!(seconds(Duration::ZERO).abs() < f64::EPSILON, "zero");
}

#[test]
fn a_renewal_keeps_the_time_taken_only_while_the_lease_has_not_ended() {
    assert!(
        TAKE.contains(
            "taken_at = CASE WHEN vcon.lease.holder = EXCLUDED.holder AND vcon.lease.expires_at \
             > now() THEN vcon.lease.taken_at ELSE now() END"
        ),
        "a lapsed lease taken again is taken anew: {TAKE}"
    );
}

/// Live, against the servers of scripts/pg-test-server.sh.
#[cfg(feature = "backend-tests")]
pub(super) mod on_a_server {
    use std::sync::Arc;
    use std::time::Duration;

    use super::super::super::catalog::Database;
    use super::super::super::registry::PgRegistry;
    use super::super::super::schema::Holds;
    use super::super::schema_tests::live::{Db, in_scope};
    use crate::backend::{CommitError, Job, Registry};
    use crate::envelope::Key;
    use crate::tenants::TenantError;

    /// Runs `call` on a runtime of its own.
    fn wait<T>(call: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(call)
    }

    /// Two processes on one migrated database: a server, and a command
    /// beside it, each with its own connections and so its own holder,
    /// and each with the registry under one key.
    pub(crate) struct Two {
        pub(crate) db: Db,
        pub(crate) first: PgRegistry,
        pub(crate) second: PgRegistry,
        pub(crate) first_db: Arc<Database>,
        pub(crate) second_db: Arc<Database>,
        pub(crate) key: Key,
    }

    /// The schema owner's connection.
    async fn owner(db: &Db) -> tokio_postgres::Client {
        let client = db.admin().await;
        client
            .batch_execute("SET ROLE vcon_owner")
            .await
            .expect("the owner role");
        client
    }

    pub(crate) fn two() -> Two {
        let db = wait(Db::migrated());
        let (_dir, settings) = db.settings("vcon_app");
        let first_db = Database::connect(&settings).expect("the server connects");
        let second_db = Database::connect_as(&settings, Holds::Beside).expect("a command beside");
        let key = Key::generate().expect("a key");
        let first = PgRegistry::open(first_db.clone(), &key).expect("the first opens");
        let second = PgRegistry::open(second_db.clone(), &key).expect("the second opens");
        Two {
            db,
            first,
            second,
            first_db,
            second_db,
            key,
        }
    }

    impl Two {
        /// One value from `sql` over the lease table, read as the schema's
        /// owner in the registry's scope.
        pub(crate) fn read(&self, sql: &str) -> String {
            wait(async {
                let client = owner(&self.db).await;
                in_scope(&client, "registry", sql)
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
            })
        }

        /// The holder of `job`'s row, or `none`.
        pub(crate) fn holder_of(&self, job: &str) -> String {
            self.read(&format!(
                "SELECT coalesce((SELECT holder FROM vcon.lease WHERE name = '{job}'), 'none')"
            ))
        }

        /// Runs `sql` as the schema's owner in the registry's scope.
        pub(crate) fn as_owner(&self, sql: &str) {
            wait(async {
                let client = owner(&self.db).await;
                in_scope(&client, "registry", sql)
                    .await
                    .unwrap_or_else(|e| panic!("{sql}: {e}"));
            });
        }

        /// Ends `job`'s lease now, by the database's clock.
        pub(crate) fn end(&self, job: &str) {
            self.as_owner(&format!(
                "UPDATE vcon.lease SET expires_at = now() - interval '1 second' WHERE name = '{job}'"
            ));
        }
    }

    const MINUTE: Duration = Duration::from_secs(60);

    // What must work.

    #[test]
    fn a_free_lease_is_taken_and_its_row_names_the_holder() {
        let t = two();
        assert_eq!(t.first.lead(Job::Sweep, MINUTE), Ok(true), "taken");
        assert_eq!(
            t.holder_of("sweep"),
            t.first_db.holder(),
            "the row names the first"
        );
        assert!(
            t.first_db.holder().contains('@'),
            "a holder's name: {}",
            t.first_db.holder()
        );
    }

    #[test]
    fn a_lease_ends_its_ttl_after_the_database_s_now() {
        let t = two();
        assert_eq!(t.first.lead(Job::Sweep, MINUTE), Ok(true), "taken");
        assert_eq!(
            t.read("SELECT expires_at - taken_at FROM vcon.lease WHERE name = 'sweep'"),
            "00:01:00",
            "one reading of the database's clock, plus the ttl"
        );
        assert_eq!(
            t.read(
                "SELECT expires_at BETWEEN now() + interval '50 seconds' AND now() + interval \
                 '60 seconds' FROM vcon.lease WHERE name = 'sweep'"
            ),
            "t",
            "the end is by the database's clock"
        );
    }

    #[test]
    fn the_holder_renews_its_lease_and_keeps_the_time_it_took_it() {
        let t = two();
        assert_eq!(t.first.lead(Job::Sweep, MINUTE), Ok(true), "taken");
        let taken = t.read("SELECT taken_at FROM vcon.lease WHERE name = 'sweep'");
        assert_eq!(
            t.first.lead(Job::Sweep, Duration::from_secs(3_600)),
            Ok(true),
            "renewed"
        );
        assert_eq!(
            t.read("SELECT taken_at FROM vcon.lease WHERE name = 'sweep'"),
            taken,
            "the time taken is kept"
        );
        assert_eq!(
            t.read(
                "SELECT expires_at > now() + interval '59 minutes' FROM vcon.lease WHERE name \
                 = 'sweep'"
            ),
            "t",
            "the end moved to the new ttl"
        );
    }

    #[test]
    fn a_lease_that_ended_is_taken_by_another_holder() {
        let t = two();
        assert_eq!(t.first.lead(Job::Sweep, MINUTE), Ok(true), "the first");
        t.end("sweep");
        assert_eq!(t.second.lead(Job::Sweep, MINUTE), Ok(true), "the second");
        assert_eq!(
            t.holder_of("sweep"),
            t.second_db.holder(),
            "the row names the second"
        );
        assert_eq!(
            t.read("SELECT expires_at - taken_at FROM vcon.lease WHERE name = 'sweep'"),
            "00:01:00",
            "taken anew by the second"
        );
        assert_eq!(
            t.first.lead(Job::Sweep, MINUTE),
            Ok(false),
            "the first holds it no more"
        );
    }

    #[test]
    fn a_lease_given_up_is_taken_by_another_holder_at_once() {
        let t = two();
        assert_eq!(t.first.lead(Job::Pack, MINUTE), Ok(true), "the first");
        assert_eq!(t.first.step_down(Job::Pack), Ok(()), "given up");
        assert_eq!(t.holder_of("pack"), "none", "no row");
        assert_eq!(t.second.lead(Job::Pack, MINUTE), Ok(true), "the second");
    }

    // What must be refused.

    #[test]
    fn a_second_holder_is_refused_while_the_first_holds_the_lease() {
        let t = two();
        assert_eq!(t.first.lead(Job::Sweep, MINUTE), Ok(true), "the first");
        assert_eq!(t.second.lead(Job::Sweep, MINUTE), Ok(false), "the second");
        assert_eq!(
            t.holder_of("sweep"),
            t.first_db.holder(),
            "the row still names the first"
        );
    }

    #[test]
    fn a_holder_that_does_not_hold_the_lease_cannot_give_it_up() {
        let t = two();
        assert_eq!(t.first.lead(Job::Sweep, MINUTE), Ok(true), "the first");
        assert_eq!(t.second.step_down(Job::Sweep), Ok(()), "nothing to do");
        assert_eq!(
            t.holder_of("sweep"),
            t.first_db.holder(),
            "the first's lease is left"
        );
        assert_eq!(t.second.lead(Job::Sweep, MINUTE), Ok(false), "still held");
    }

    #[test]
    fn a_ttl_past_the_database_s_range_is_an_error_and_leaves_the_lease() {
        let t = two();
        assert_eq!(t.first.lead(Job::Sweep, MINUTE), Ok(true), "the first");
        let end = t.read("SELECT expires_at FROM vcon.lease WHERE name = 'sweep'");
        assert_eq!(
            t.first.lead(Job::Sweep, Duration::from_secs(u64::MAX)),
            Err(TenantError::Io),
            "an interval PostgreSQL cannot hold"
        );
        assert_eq!(
            t.read("SELECT expires_at FROM vcon.lease WHERE name = 'sweep'"),
            end,
            "the lease is as it was"
        );
    }

    #[test]
    fn a_restarted_server_is_a_new_holder_and_waits_for_its_old_lease() {
        let t = two();
        assert_eq!(t.first.lead(Job::Sweep, MINUTE), Ok(true), "the first run");
        let (_dir, settings) = t.db.settings("vcon_app");
        let again = Database::connect_as(&settings, Holds::Beside).expect("connects again");
        assert_ne!(again.holder(), t.first_db.holder(), "a new holder");
        let restarted = PgRegistry::open(again, &t.key).expect("opens");
        assert_eq!(
            restarted.lead(Job::Sweep, MINUTE),
            Ok(false),
            "the old lease is held until it ends"
        );
    }

    #[test]
    fn a_take_whose_commit_had_no_answer_is_an_error_not_a_lease_held() {
        let t = two();
        t.first.fail_commit(1, CommitError::Unknown);
        assert_eq!(
            t.first.lead(Job::Sweep, MINUTE),
            Err(TenantError::Io),
            "the caller does not know it holds the lease"
        );
        assert_eq!(
            t.first.lead(Job::Sweep, MINUTE),
            Ok(true),
            "the next call finds it held, by the row the commit kept"
        );
    }

    #[test]
    fn a_take_whose_commit_was_rolled_back_holds_nothing() {
        let t = two();
        t.first.fail_commit(1, CommitError::RolledBack);
        assert_eq!(
            t.first.lead(Job::Sweep, MINUTE),
            Err(TenantError::Io),
            "refused"
        );
        assert_eq!(t.holder_of("sweep"), "none", "no row");
        assert_eq!(
            t.second.lead(Job::Sweep, MINUTE),
            Ok(true),
            "free for another"
        );
    }

    // The edges.

    #[test]
    fn the_two_jobs_are_held_apart() {
        let t = two();
        assert_eq!(t.first.lead(Job::Sweep, MINUTE), Ok(true), "the sweep");
        assert_eq!(t.second.lead(Job::Pack, MINUTE), Ok(true), "the pack run");
        assert_eq!(t.holder_of("sweep"), t.first_db.holder(), "the first's");
        assert_eq!(t.holder_of("pack"), t.second_db.holder(), "the second's");
    }

    #[test]
    fn a_lease_of_no_time_is_taken_and_ended_at_once() {
        let t = two();
        assert_eq!(t.first.lead(Job::Sweep, Duration::ZERO), Ok(true), "taken");
        assert_eq!(
            t.second.lead(Job::Sweep, MINUTE),
            Ok(true),
            "and ended: another takes it"
        );
    }

    #[test]
    fn a_ttl_s_fraction_of_a_second_is_kept() {
        let t = two();
        assert_eq!(
            t.first.lead(Job::Sweep, Duration::from_millis(1_500)),
            Ok(true),
            "taken"
        );
        assert_eq!(
            t.read("SELECT expires_at - taken_at FROM vcon.lease WHERE name = 'sweep'"),
            "00:00:01.5",
            "a second and a half"
        );
    }

    #[test]
    fn a_holder_whose_lease_lapsed_takes_it_anew() {
        let t = two();
        assert_eq!(t.first.lead(Job::Sweep, MINUTE), Ok(true), "taken");
        t.as_owner(
            "UPDATE vcon.lease SET taken_at = now() - interval '2 hours', expires_at = now() - \
             interval '1 hour'",
        );
        assert_eq!(t.first.lead(Job::Sweep, MINUTE), Ok(true), "taken again");
        assert_eq!(
            t.read("SELECT expires_at - taken_at FROM vcon.lease WHERE name = 'sweep'"),
            "00:01:00",
            "taken now, not two hours ago"
        );
    }

    #[test]
    fn a_long_ttl_is_held_to_its_end() {
        let t = two();
        assert_eq!(
            t.first
                .lead(Job::Sweep, Duration::from_secs(100 * 365 * 86_400)),
            Ok(true),
            "a hundred years"
        );
        assert_eq!(t.second.lead(Job::Sweep, MINUTE), Ok(false), "held");
    }

    #[test]
    fn of_two_holders_that_take_a_free_lease_at_once_one_holds_it() {
        let t = Arc::new(two());
        let racers: Vec<_> = (0..2)
            .map(|n| {
                let t = t.clone();
                std::thread::spawn(move || {
                    let registry = if n == 0 { &t.first } else { &t.second };
                    (0..20)
                        .map(|_| registry.lead(Job::Pack, MINUTE).expect("answers"))
                        .collect::<Vec<bool>>()
                })
            })
            .collect();
        let answers: Vec<Vec<bool>> = racers
            .into_iter()
            .map(|r| r.join().expect("joins"))
            .collect();
        let held: Vec<bool> = answers.iter().map(|a| a.iter().all(|h| *h)).collect();
        let never: Vec<bool> = answers.iter().map(|a| a.iter().all(|h| !*h)).collect();
        assert!(
            held.iter().zip(&never).all(|(h, n)| h != n),
            "each holder either held it each time or never: {answers:?}"
        );
        assert_eq!(
            held.iter().filter(|h| **h).count(),
            1,
            "exactly one holds it: {answers:?}"
        );
    }
}
