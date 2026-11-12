//! How long an access key lives, and when it was last used.
//!
//! A new key expires 90 days after it is made unless its maker asks for
//! another number of days, from 1 to 365, or for no expiry. The
//! configuration can refuse a key with no expiry (`keys.no_expiry`). A key
//! made before keys had an expiry has none.
//!
//! A key's last-used time is the start of the minute of an accepted
//! request. It is written when a request is accepted in a later minute
//! than the one stored, so one key causes at most one write to the registry
//! in a minute, however many requests it makes.

use std::sync::atomic::Ordering;

use super::{TenantError, Tenants, commit};
use crate::clock::nanos;
use crate::ids::KeyId;

/// The days a new key lives when its maker names none.
pub const DEFAULT_DAYS: u32 = 90;

/// The most days a new key with an expiry may live.
pub const MAX_DAYS: u32 = 365;

/// The nanoseconds in a second: the resolution of an expiry.
const SECOND: i64 = 1_000_000_000;

/// The nanoseconds in a day.
const DAY: i64 = 86_400 * SECOND;

/// The nanoseconds in a minute: the resolution of a last-used time.
const MINUTE: i64 = 60 * 1_000_000_000;

/// What the maker of a key asked for its expiry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyLife {
    /// 90 days.
    #[default]
    Default,
    /// This many days, from 1 to 365.
    Days(u32),
    /// No expiry.
    Never,
    /// An expiry no key can have, as a request asked it: days outside 1
    /// to 365, or days and no expiry together. It is refused where
    /// the key would be made, so the refusal is recorded as the others are
    /// and comes after that of the key's scopes.
    Invalid,
}

impl KeyLife {
    /// The life a request's two fields ask for: `days` (`expires_in_days`)
    /// and `never` (`no_expiry`), each absent or given.
    /// [`KeyLife::Invalid`] when the days are not from 1 to 365, or
    /// when days are given and no expiry is asked as well.
    #[must_use]
    pub fn asked(days: Option<i64>, never: Option<bool>) -> KeyLife {
        match (days, never.unwrap_or(false)) {
            (None, false) => KeyLife::Default,
            (None, true) => KeyLife::Never,
            (Some(_), true) => KeyLife::Invalid,
            (Some(days), false) => u32::try_from(days)
                .ok()
                .filter(|days| (1..=MAX_DAYS).contains(days))
                .map_or(KeyLife::Invalid, KeyLife::Days),
        }
    }

    /// When a key made at `now` (nanoseconds) with this life expires, cut
    /// down to the whole second; `Ok(None)` for no expiry.
    ///
    /// # Errors
    ///
    /// [`TenantError::BadExpiry`] for days outside 1 to [`MAX_DAYS`], for
    /// no expiry when `never_allowed` is false, and for a time past the
    /// range of the clock.
    pub(crate) fn expires_at(
        self,
        now: i64,
        never_allowed: bool,
    ) -> Result<Option<i64>, TenantError> {
        let days = match self {
            KeyLife::Never if never_allowed => return Ok(None),
            KeyLife::Never => return Err(TenantError::BadExpiry),
            KeyLife::Default => DEFAULT_DAYS,
            KeyLife::Days(days) if (1..=MAX_DAYS).contains(&days) => days,
            KeyLife::Days(_) | KeyLife::Invalid => return Err(TenantError::BadExpiry),
        };
        // Cut down to the whole second: the time is then written with no
        // fraction, so the answer that carries it has one length whenever
        // the key is made, and the key never lives longer than asked.
        i64::from(days)
            .checked_mul(DAY)
            .and_then(|life| now.checked_add(life))
            .and_then(|at| at.checked_sub(at.rem_euclid(SECOND)))
            .map(Some)
            .ok_or(TenantError::BadExpiry)
    }
}

/// Whether a key with this expiry is refused at `now`: from the instant of
/// its expiry on. A key with no expiry never is.
#[must_use]
pub(crate) fn expired(expires_at: Option<i64>, now: i64) -> bool {
    expires_at.is_some_and(|at| now >= at)
}

/// The start of the minute `at` (nanoseconds) is in.
#[must_use]
pub(crate) fn minute(at: i64) -> i64 {
    at - at.rem_euclid(MINUTE)
}

/// The last-used time to write for a request accepted at `now`, given the
/// one `stored`: the start of the minute of `now` when that is later than
/// the stored time, and None when there is nothing to write.
#[must_use]
pub(crate) fn last_used_due(stored: Option<i64>, now: i64) -> Option<i64> {
    let due = minute(now);
    stored.is_none_or(|stored| stored < due).then_some(due)
}

impl Tenants {
    /// Whether a key may be made with no expiry (`keys.no_expiry`): true
    /// until [`Tenants::allow_no_expiry`] says otherwise.
    pub(crate) fn no_expiry_allowed(&self) -> bool {
        self.no_expiry.load(Ordering::Relaxed)
    }

    /// Sets whether a key may be made with no expiry, as the
    /// configuration says (`keys.no_expiry`). Keys made earlier are not
    /// changed.
    pub fn allow_no_expiry(&self, allowed: bool) {
        self.no_expiry.store(allowed, Ordering::Relaxed);
    }

    /// Notes that a request by the key `id` was accepted now, given the
    /// last-used time read with the key: writes the minute of now when it
    /// is later than the one stored, and nothing otherwise. Whether a row
    /// was changed. A failure to write is not an error of the request: the
    /// time is written by a later request.
    pub(crate) fn key_used(&self, id: KeyId, stored: Option<i64>) -> bool {
        let Some(due) = last_used_due(stored, nanos(self.now())) else {
            return false;
        };
        let written = || {
            let mut tx = self.registry.immediate()?;
            let changed = tx.touch_key(id, due)?;
            commit(tx)?;
            Ok::<bool, TenantError>(changed)
        };
        written().unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use time::Duration;
    use time::macros::datetime;

    use super::*;
    use crate::access::{self, AuthError, Scope, Scopes};
    use crate::audit::Actor;
    use crate::backend::{CommitError, KeyRow};
    use crate::clock::from_nanos;
    use crate::ids::TenantId;
    use crate::tenants::NewKey;
    use crate::tenants::tests::{Fixture, fixture};

    /// The fixture's clock starts here.
    const START: time::OffsetDateTime = datetime!(2026-10-23 12:00 UTC);

    fn start() -> i64 {
        nanos(START)
    }

    fn vcon_scopes() -> Scopes {
        Scopes::of(&[Scope::Write, Scope::Read])
    }

    fn tenant(f: &Fixture) -> TenantId {
        f.tenants.create(Actor::Cli, "acme", None).expect("made")
    }

    fn key(f: &Fixture, life: KeyLife) -> NewKey {
        let t = tenant(f);
        f.tenants
            .create_key_for(Actor::Cli, Some(t), vcon_scopes(), "ingest", life)
            .expect("made")
    }

    fn row(f: &Fixture, id: KeyId) -> KeyRow {
        f.tenants
            .registry
            .snapshot()
            .expect("a snapshot")
            .key(id)
            .expect("read")
            .expect("the key's row")
    }

    fn auth(f: &Fixture, key: &NewKey) -> Result<access::Caller, AuthError> {
        access::authenticate(&f.tenants, Some(&format!("Bearer {}", key.token)))
    }

    /// The failed authentications counted in the tenant's log.
    fn failures(f: &Fixture, tenant: TenantId) -> i64 {
        f.tenant_catalog(tenant)
            .query_row(
                "SELECT count(*) FROM audit WHERE action = 'auth_failed'",
                [],
                |r| r.get(0),
            )
            .expect("counted")
    }

    // What must work: the request's fields.

    #[test]
    fn a_request_that_names_no_expiry_field_asks_for_the_default() {
        assert_eq!(KeyLife::asked(None, None), KeyLife::Default, "none");
        assert_eq!(
            KeyLife::asked(None, Some(false)),
            KeyLife::Default,
            "no_expiry false is the same as absent"
        );
    }

    #[test]
    fn a_request_may_ask_for_any_number_of_days_from_1_to_365() {
        for days in [1, 2, 30, 90, 364, 365] {
            assert_eq!(
                KeyLife::asked(Some(i64::from(days)), None),
                KeyLife::Days(days),
                "{days} days"
            );
        }
    }

    #[test]
    fn a_request_may_ask_for_no_expiry() {
        assert_eq!(
            KeyLife::asked(None, Some(true)),
            KeyLife::Never,
            "no_expiry true"
        );
    }

    #[test]
    fn days_with_no_expiry_false_are_days() {
        assert_eq!(
            KeyLife::asked(Some(7), Some(false)),
            KeyLife::Days(7),
            "the false changes nothing"
        );
    }

    // What must work: the expiry of a new key.

    #[test]
    fn the_default_life_is_90_days() {
        assert_eq!(DEFAULT_DAYS, 90, "the approved default");
        assert_eq!(
            KeyLife::Default.expires_at(1_000 * SECOND, true),
            Ok(Some(1_000 * SECOND + 90 * DAY)),
            "90 days after the key is made"
        );
        assert_eq!(
            KeyLife::Default.expires_at(1_000 * SECOND, false),
            Ok(Some(1_000 * SECOND + 90 * DAY)),
            "the same where no expiry is refused"
        );
    }

    #[test]
    fn a_life_in_days_ends_that_many_days_after_the_key_is_made() {
        for days in [1_u32, 30, 365] {
            assert_eq!(
                KeyLife::Days(days).expires_at(5 * SECOND, true),
                Ok(Some(5 * SECOND + i64::from(days) * DAY)),
                "{days} days"
            );
        }
    }

    // An expiry is a whole second, so the answer that carries it has one
    // length whenever the key is made.

    #[test]
    fn an_expiry_is_cut_to_the_whole_second() {
        assert_eq!(
            KeyLife::Default.expires_at(7 * SECOND + 999_999_999, true),
            Ok(Some(7 * SECOND + 90 * DAY)),
            "the last nanosecond of a second"
        );
        assert_eq!(
            KeyLife::Days(1).expires_at(7 * SECOND + 1, true),
            Ok(Some(7 * SECOND + DAY)),
            "the first nanosecond after a second"
        );
    }

    #[test]
    fn an_expiry_made_on_the_second_is_not_moved() {
        assert_eq!(
            KeyLife::Days(30).expires_at(7 * SECOND, true),
            Ok(Some(7 * SECOND + 30 * DAY)),
            "nothing to cut"
        );
    }

    #[test]
    fn an_expiry_is_never_after_the_days_asked() {
        for nanos in [0, 1, 500_000_000, 999_999_999] {
            let now = 1_762_000_000 * SECOND + nanos;
            let at = KeyLife::Days(2)
                .expires_at(now, true)
                .expect("two days")
                .expect("a time");
            assert_eq!(at % SECOND, 0, "a whole second at {nanos} ns");
            assert!(
                at <= now + 2 * DAY && now + 2 * DAY - at < SECOND,
                "cut down, by less than a second, at {nanos} ns"
            );
        }
    }

    #[test]
    fn an_expiry_before_1970_is_cut_down_too() {
        assert_eq!(
            KeyLife::Days(1).expires_at(-2 * DAY - 1, true),
            Ok(Some(-DAY - SECOND)),
            "toward the earlier second, not toward zero"
        );
    }

    #[test]
    fn a_key_made_within_a_second_expires_on_a_whole_second() {
        let f = fixture();
        let t = tenant(&f);
        f.clock.set(START + Duration::nanoseconds(123_456_789));
        let made = f
            .tenants
            .create_key(Actor::Cli, Some(t), vcon_scopes(), "ingest")
            .expect("made");
        assert_eq!(
            row(&f, made.id).expires_at,
            Some(start() + 90 * DAY),
            "the stored expiry has no part of a second"
        );
        assert_eq!(
            made.expires_at,
            Some(START + Duration::days(90)),
            "nor has the expiry its maker is told"
        );
    }

    #[test]
    fn the_longest_life_in_days_is_365() {
        assert_eq!(MAX_DAYS, 365, "the approved maximum, one year");
        assert!(
            KeyLife::Days(MAX_DAYS).expires_at(0, false).is_ok(),
            "the maximum itself is allowed"
        );
    }

    #[test]
    fn a_key_with_no_expiry_has_no_time_where_that_is_allowed() {
        assert_eq!(KeyLife::Never.expires_at(77, true), Ok(None), "no time");
    }

    #[test]
    fn a_key_is_not_expired_before_its_time_and_never_without_one() {
        assert!(!expired(Some(100), 99), "one nanosecond before");
        assert!(!expired(Some(100), i64::MIN), "long before");
        assert!(!expired(None, i64::MAX), "no expiry, at the end of time");
    }

    // What must work: the last-used time.

    #[test]
    fn a_time_is_cut_to_the_start_of_its_minute() {
        assert_eq!(minute(0), 0, "the epoch");
        assert_eq!(minute(MINUTE), MINUTE, "the start of a minute");
        assert_eq!(minute(MINUTE + 1), MINUTE, "just inside it");
        assert_eq!(minute(2 * MINUTE - 1), MINUTE, "its last nanosecond");
        assert_eq!(minute(start() + 59 * 1_000_000_000), start(), "59 s in");
    }

    #[test]
    fn a_key_never_used_is_due_the_minute_of_its_first_request() {
        assert_eq!(
            last_used_due(None, 3 * MINUTE + 5),
            Some(3 * MINUTE),
            "the minute of now"
        );
    }

    #[test]
    fn a_request_in_a_later_minute_is_due_that_minute() {
        assert_eq!(
            last_used_due(Some(3 * MINUTE), 4 * MINUTE),
            Some(4 * MINUTE),
            "the first nanosecond of the next minute"
        );
        assert_eq!(
            last_used_due(Some(3 * MINUTE), 90 * MINUTE + 17),
            Some(90 * MINUTE),
            "much later"
        );
    }

    // What must be refused: the request's fields and the life.

    #[test]
    fn days_below_1_are_refused() {
        for days in [0, -1, -365, i64::MIN] {
            assert_eq!(
                KeyLife::asked(Some(days), None),
                KeyLife::Invalid,
                "{days} days"
            );
        }
    }

    #[test]
    fn days_above_365_are_refused() {
        for days in [366, 367, 1_000, i64::from(u32::MAX) + 90, i64::MAX] {
            assert_eq!(
                KeyLife::asked(Some(days), None),
                KeyLife::Invalid,
                "{days} days"
            );
        }
    }

    #[test]
    fn days_and_no_expiry_together_are_refused() {
        assert_eq!(
            KeyLife::asked(Some(30), Some(true)),
            KeyLife::Invalid,
            "the two contradict each other"
        );
    }

    #[test]
    fn no_expiry_is_refused_where_the_configuration_refuses_it() {
        assert_eq!(
            KeyLife::Never.expires_at(77, false),
            Err(TenantError::BadExpiry),
            "keys.no_expiry is false"
        );
    }

    #[test]
    fn an_invalid_life_makes_no_expiry_and_no_key() {
        for allowed in [true, false] {
            assert_eq!(
                KeyLife::Invalid.expires_at(0, allowed),
                Err(TenantError::BadExpiry),
                "no expiry allowed: {allowed}"
            );
        }
    }

    #[test]
    fn a_life_of_days_outside_the_range_is_refused_however_it_was_made() {
        for days in [0, 366, u32::MAX] {
            assert_eq!(
                KeyLife::Days(days).expires_at(0, true),
                Err(TenantError::BadExpiry),
                "{days} days"
            );
        }
    }

    #[test]
    fn a_key_is_expired_from_the_instant_of_its_expiry() {
        assert!(expired(Some(100), 100), "at the instant");
        assert!(expired(Some(100), 101), "one nanosecond after");
        assert!(expired(Some(100), i64::MAX), "long after");
    }

    // The edges of the pure rules.

    #[test]
    fn an_expiry_past_the_range_of_the_clock_is_refused_not_wrapped() {
        assert_eq!(
            KeyLife::Default.expires_at(i64::MAX, true),
            Err(TenantError::BadExpiry),
            "the default from the last instant"
        );
        assert_eq!(
            KeyLife::Days(1).expires_at(i64::MAX - DAY + 1, true),
            Err(TenantError::BadExpiry),
            "one nanosecond too far"
        );
        assert_eq!(
            KeyLife::Days(1).expires_at(i64::MAX - DAY, true),
            Ok(Some(i64::MAX - i64::MAX % SECOND)),
            "the second of the last instant"
        );
    }

    #[test]
    fn a_request_in_the_stored_minute_is_due_nothing() {
        assert_eq!(last_used_due(Some(3 * MINUTE), 3 * MINUTE), None, "start");
        assert_eq!(
            last_used_due(Some(3 * MINUTE), 4 * MINUTE - 1),
            None,
            "the minute's last nanosecond"
        );
    }

    #[test]
    fn a_clock_that_went_back_is_due_nothing() {
        assert_eq!(
            last_used_due(Some(10 * MINUTE), 3 * MINUTE),
            None,
            "the stored time is later than now"
        );
    }

    #[test]
    fn a_time_before_the_epoch_is_cut_toward_the_past() {
        assert_eq!(minute(-1), -MINUTE, "the minute before the epoch");
        assert_eq!(minute(-MINUTE), -MINUTE, "its start");
        assert_eq!(minute(-MINUTE - 1), -2 * MINUTE, "the one before");
    }

    #[test]
    fn the_default_is_the_life_a_caller_gets_without_asking() {
        assert_eq!(KeyLife::default(), KeyLife::Default, "Default");
    }

    // What must work: keys in the registry.

    #[test]
    fn a_key_made_without_a_life_expires_90_days_after_the_clock() {
        let f = fixture();
        let t = tenant(&f);
        let made = f
            .tenants
            .create_key(Actor::Cli, Some(t), vcon_scopes(), "ingest")
            .expect("made");
        assert_eq!(
            row(&f, made.id).expires_at,
            Some(start() + 90 * DAY),
            "the stored expiry"
        );
        assert_eq!(
            made.expires_at,
            Some(START + Duration::days(90)),
            "the expiry its maker is told"
        );
        assert_eq!(row(&f, made.id).last_used_at, None, "never used");
    }

    #[test]
    fn a_key_made_for_30_days_expires_then() {
        let f = fixture();
        let made = key(&f, KeyLife::Days(30));
        assert_eq!(
            row(&f, made.id).expires_at,
            Some(start() + 30 * DAY),
            "the stored expiry"
        );
        assert_eq!(
            made.expires_at,
            Some(START + Duration::days(30)),
            "the expiry its maker is told"
        );
    }

    #[test]
    fn a_key_made_with_no_expiry_stores_none() {
        let f = fixture();
        let made = key(&f, KeyLife::Never);
        assert_eq!(row(&f, made.id).expires_at, None, "the stored expiry");
        assert_eq!(made.expires_at, None, "the expiry its maker is told");
        f.clock.set(START + Duration::days(4_000));
        assert!(auth(&f, &made).is_ok(), "accepted years later");
    }

    #[test]
    fn an_operator_key_has_an_expiry_as_a_tenant_key_has() {
        let f = fixture();
        let made = f
            .tenants
            .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "operator")
            .expect("made");
        assert_eq!(
            row(&f, made.id).expires_at,
            Some(start() + 90 * DAY),
            "90 days"
        );
    }

    #[test]
    fn a_key_is_accepted_until_the_nanosecond_before_its_expiry() {
        let f = fixture();
        let made = key(&f, KeyLife::Days(1));
        f.clock
            .set(START + Duration::days(1) - Duration::nanoseconds(1));
        assert!(auth(&f, &made).is_ok(), "the last instant of its life");
    }

    #[test]
    fn a_list_shows_each_keys_expiry_and_last_used_time() {
        let f = fixture();
        let made = key(&f, KeyLife::Days(10));
        f.clock.set(START + Duration::seconds(125));
        assert!(auth(&f, &made).is_ok(), "used");
        let listed = f.tenants.list_keys(Actor::Cli, None).expect("listed");
        assert_eq!(listed.len(), 1, "one key");
        assert_eq!(
            listed[0].expires_at,
            Some(START + Duration::days(10)),
            "the expiry"
        );
        assert_eq!(
            listed[0].last_used_at,
            Some(START + Duration::minutes(2)),
            "the minute of its use"
        );
        let json = listed[0].to_json().expect("written");
        assert_eq!(json["expires_at"], "2026-11-02T12:00:00Z", "as JSON");
        assert_eq!(json["last_used_at"], "2026-10-23T12:02:00Z", "as JSON");
    }

    #[test]
    fn a_key_never_used_and_with_no_expiry_lists_both_as_null() {
        let f = fixture();
        key(&f, KeyLife::Never);
        let listed = f.tenants.list_keys(Actor::Cli, None).expect("listed");
        let json = listed[0].to_json().expect("written");
        assert!(json["expires_at"].is_null(), "no expiry is null");
        assert!(json["last_used_at"].is_null(), "never used is null");
        assert!(
            json.as_object()
                .expect("an object")
                .contains_key("expires_at")
                && json
                    .as_object()
                    .expect("an object")
                    .contains_key("last_used_at"),
            "both names are present"
        );
    }

    #[test]
    fn the_first_accepted_request_writes_the_minute_it_came_in() {
        let f = fixture();
        let made = key(&f, KeyLife::Default);
        f.clock.set(START + Duration::seconds(61));
        assert!(auth(&f, &made).is_ok(), "accepted");
        assert_eq!(
            row(&f, made.id).last_used_at,
            Some(start() + MINUTE),
            "the start of the minute of the request"
        );
    }

    #[test]
    fn a_request_in_a_later_minute_moves_the_time() {
        let f = fixture();
        let made = key(&f, KeyLife::Default);
        assert!(auth(&f, &made).is_ok(), "the first");
        f.clock.set(START + Duration::minutes(7));
        assert!(auth(&f, &made).is_ok(), "the second");
        assert_eq!(
            row(&f, made.id).last_used_at,
            Some(start() + 7 * MINUTE),
            "the later minute"
        );
    }

    #[test]
    fn the_times_are_kept_over_a_restart() {
        let mut f = fixture();
        let made = key(&f, KeyLife::Days(5));
        assert!(auth(&f, &made).is_ok(), "used");
        let before = row(&f, made.id);
        f.restart();
        assert_eq!(row(&f, made.id), before, "the row after a restart");
        assert_eq!(before.last_used_at, Some(start()), "the minute of the use");
    }

    #[test]
    fn using_a_key_changes_no_other_column_of_its_row() {
        let f = fixture();
        let made = key(&f, KeyLife::Days(5));
        let before = row(&f, made.id);
        f.clock.set(START + Duration::minutes(3));
        assert!(auth(&f, &made).is_ok(), "used");
        let after = row(&f, made.id);
        assert_eq!(
            KeyRow {
                last_used_at: None,
                ..after
            },
            before,
            "everything but the last-used time"
        );
    }

    #[test]
    fn a_last_used_write_is_not_an_audit_record() {
        let f = fixture();
        let made = key(&f, KeyLife::Default);
        let before = f.count("SELECT count(*) FROM audit");
        assert!(auth(&f, &made).is_ok(), "used");
        assert!(row(&f, made.id).last_used_at.is_some(), "written");
        assert_eq!(
            f.count("SELECT count(*) FROM audit"),
            before,
            "the admin log is as it was"
        );
    }

    // What must be refused: keys in the registry.

    #[test]
    fn an_expired_key_is_refused_as_expired() {
        let f = fixture();
        let made = key(&f, KeyLife::Days(1));
        f.clock.set(START + Duration::days(1));
        assert_eq!(
            auth(&f, &made),
            Err(AuthError::Expired),
            "at the instant of its expiry"
        );
        f.clock.set(START + Duration::days(400));
        assert_eq!(auth(&f, &made), Err(AuthError::Expired), "long after");
    }

    #[test]
    fn a_key_made_by_default_is_refused_after_90_days() {
        let f = fixture();
        let made = key(&f, KeyLife::Default);
        f.clock.set(START + Duration::days(89));
        assert!(auth(&f, &made).is_ok(), "on day 89");
        f.clock.set(START + Duration::days(90));
        assert_eq!(auth(&f, &made), Err(AuthError::Expired), "on day 90");
    }

    #[test]
    fn an_expired_keys_refusal_is_counted_under_the_key() {
        let f = fixture();
        let made = key(&f, KeyLife::Days(1));
        f.clock.set(START + Duration::days(2));
        let t = row(&f, made.id).tenant.expect("a tenant key");
        let before = failures(&f, t);
        assert_eq!(auth(&f, &made), Err(AuthError::Expired), "refused");
        assert_eq!(failures(&f, t), before + 1, "one failed authentication");
    }

    #[test]
    fn a_wrong_secret_for_an_expired_key_learns_nothing_of_the_expiry() {
        let f = fixture();
        let made = key(&f, KeyLife::Days(1));
        f.clock.set(START + Duration::days(2));
        let mut token = made.token.clone();
        // The first character of the secret, after `vcs_`, the id and `_`.
        let first = token.remove(21);
        token.insert(21, if first == 'A' { 'B' } else { 'A' });
        assert_eq!(
            access::authenticate(&f.tenants, Some(&format!("Bearer {token}"))),
            Err(AuthError::Unknown),
            "the same answer as for a key nobody has"
        );
    }

    #[test]
    fn a_key_both_revoked_and_expired_is_refused_as_revoked() {
        let f = fixture();
        let made = key(&f, KeyLife::Days(1));
        f.tenants.revoke_key(Actor::Cli, made.id).expect("revoked");
        f.clock.set(START + Duration::days(2));
        assert_eq!(
            auth(&f, &made),
            Err(AuthError::Revoked),
            "revocation is told first"
        );
    }

    #[test]
    fn a_key_with_no_expiry_is_refused_and_recorded_where_the_configuration_refuses_it() {
        let f = fixture();
        let t = tenant(&f);
        f.tenants.allow_no_expiry(false);
        let refused = f.tenants.create_key_for(
            Actor::Cli,
            Some(t),
            vcon_scopes(),
            "forever",
            KeyLife::Never,
        );
        assert_eq!(refused.err(), Some(TenantError::BadExpiry), "refused");
        assert_eq!(f.count("SELECT count(*) FROM api_key"), 0, "no key is made");
        let recorded = f.admin_records("key_create");
        assert_eq!(recorded.len(), 1, "one record");
        assert_eq!(recorded[0]["status"], 422, "the refusal's status");
    }

    #[test]
    fn a_life_of_days_outside_the_range_makes_no_key() {
        let f = fixture();
        let t = tenant(&f);
        for days in [0, 366] {
            let refused = f.tenants.create_key_for(
                Actor::Cli,
                Some(t),
                vcon_scopes(),
                "ingest",
                KeyLife::Days(days),
            );
            assert_eq!(refused.err(), Some(TenantError::BadExpiry), "{days} days");
        }
        assert_eq!(f.count("SELECT count(*) FROM api_key"), 0, "no key is made");
    }

    #[test]
    fn an_operator_key_with_no_expiry_is_refused_where_the_configuration_refuses_it() {
        let f = fixture();
        f.tenants.allow_no_expiry(false);
        let refused = f.tenants.create_key_for(
            Actor::Cli,
            None,
            Scopes::of(&[Scope::Admin]),
            "operator",
            KeyLife::Never,
        );
        assert_eq!(refused.err(), Some(TenantError::BadExpiry), "refused");
    }

    #[test]
    fn a_refused_request_does_not_move_the_last_used_time() {
        let f = fixture();
        let made = key(&f, KeyLife::Days(1));
        assert!(auth(&f, &made).is_ok(), "used once");
        let used = row(&f, made.id).last_used_at;
        f.clock.set(START + Duration::days(3));
        assert_eq!(auth(&f, &made), Err(AuthError::Expired), "expired");
        assert_eq!(row(&f, made.id).last_used_at, used, "after the expiry");
    }

    #[test]
    fn a_wrong_secret_does_not_move_the_last_used_time() {
        let f = fixture();
        let made = key(&f, KeyLife::Default);
        let mut token = made.token.clone();
        // The first character of the secret, after `vcs_`, the id and `_`.
        let first = token.remove(21);
        token.insert(21, if first == 'A' { 'B' } else { 'A' });
        assert!(
            access::authenticate(&f.tenants, Some(&format!("Bearer {token}"))).is_err(),
            "refused"
        );
        assert_eq!(row(&f, made.id).last_used_at, None, "never used");
    }

    #[test]
    fn a_revoked_key_does_not_move_the_last_used_time() {
        let f = fixture();
        let made = key(&f, KeyLife::Default);
        f.tenants.revoke_key(Actor::Cli, made.id).expect("revoked");
        assert_eq!(auth(&f, &made), Err(AuthError::Revoked), "refused");
        assert_eq!(row(&f, made.id).last_used_at, None, "never used");
    }

    // The edges: keys in the registry.

    #[test]
    fn requests_in_one_minute_write_once() {
        let f = fixture();
        let made = key(&f, KeyLife::Default);
        assert!(f.tenants.key_used(made.id, None), "the first writes");
        let stored = row(&f, made.id).last_used_at;
        assert_eq!(stored, Some(start()), "the minute");
        f.clock.set(START + Duration::seconds(59));
        assert!(
            !f.tenants.key_used(made.id, stored),
            "a request 59 s later writes nothing"
        );
        assert!(auth(&f, &made).is_ok(), "and is accepted");
        assert_eq!(
            row(&f, made.id).last_used_at,
            stored,
            "the time is the same"
        );
    }

    #[test]
    fn two_servers_that_read_the_same_row_write_the_minute_once() {
        let f = fixture();
        let made = key(&f, KeyLife::Default);
        assert!(
            f.tenants.key_used(made.id, None),
            "the first changes the row"
        );
        assert!(
            !f.tenants.key_used(made.id, None),
            "the second read the row before the first wrote, and changes nothing"
        );
    }

    #[test]
    fn a_clock_that_went_back_does_not_move_the_time_back() {
        let f = fixture();
        let made = key(&f, KeyLife::Default);
        f.clock.set(START + Duration::minutes(10));
        assert!(auth(&f, &made).is_ok(), "used");
        f.clock.set(START + Duration::minutes(2));
        assert!(auth(&f, &made).is_ok(), "used at an earlier time");
        assert!(
            !f.tenants.key_used(made.id, None),
            "even a writer that read no time changes nothing"
        );
        assert_eq!(
            row(&f, made.id).last_used_at,
            Some(start() + 10 * MINUTE),
            "the later time is kept"
        );
    }

    #[test]
    fn a_last_used_write_that_fails_does_not_refuse_the_request() {
        let f = fixture();
        let made = key(&f, KeyLife::Default);
        f.tenants.registry.fail_commit(1, CommitError::RolledBack);
        assert!(auth(&f, &made).is_ok(), "accepted all the same");
        assert_eq!(row(&f, made.id).last_used_at, None, "nothing was written");
        f.clock.set(START + Duration::seconds(5));
        assert!(auth(&f, &made).is_ok(), "the next request");
        assert_eq!(
            row(&f, made.id).last_used_at,
            Some(start()),
            "the next request writes the time"
        );
    }

    #[test]
    fn a_write_that_fails_is_reported_as_no_write() {
        let f = fixture();
        let made = key(&f, KeyLife::Default);
        f.tenants.registry.fail_commit(1, CommitError::RolledBack);
        assert!(
            !f.tenants.key_used(made.id, None),
            "the commit failed, so no row was changed"
        );
    }

    #[test]
    fn a_second_request_in_the_minute_commits_nothing_to_the_registry() {
        let f = fixture();
        let made = key(&f, KeyLife::Default);
        let t = row(&f, made.id).tenant.expect("a tenant key");
        assert!(auth(&f, &made).is_ok(), "the first request writes the time");
        // The next commit on the registry fails, whichever it is.
        f.tenants.registry.fail_commit(1, CommitError::RolledBack);
        f.clock.set(START + Duration::seconds(30));
        assert!(auth(&f, &made).is_ok(), "the second request is accepted");
        assert!(
            f.tenants
                .create_key(Actor::Cli, Some(t), vcon_scopes(), "next")
                .is_err(),
            "the failure was still waiting: the second request committed nothing"
        );
    }

    #[test]
    fn using_a_key_nobody_has_writes_nothing() {
        let f = fixture();
        assert!(
            !f.tenants.key_used(KeyId([7; 8]), None),
            "no row has the id"
        );
    }

    #[test]
    fn a_key_stored_without_an_expiry_is_accepted_as_one_made_before_version_9() {
        let f = fixture();
        let t = tenant(&f);
        let (id, token) = access::new_token();
        let mut tx = f.tenants.registry.immediate().expect("a transaction");
        tx.insert_key(&KeyRow {
            id,
            tenant: Some(t),
            token_hash: access::token_hash(&token).to_vec(),
            scopes: vcon_scopes().to_string(),
            label: "old".to_owned(),
            created_at: start(),
            revoked_at: None,
            expires_at: None,
            last_used_at: None,
        })
        .expect("inserted");
        commit(tx).expect("committed");
        f.clock.set(START + Duration::days(3_650));
        assert!(
            access::authenticate(&f.tenants, Some(&format!("Bearer {token}"))).is_ok(),
            "ten years later"
        );
        assert_eq!(
            row(&f, id).last_used_at.and_then(from_nanos),
            Some(START + Duration::days(3_650)),
            "and its use is written"
        );
    }

    #[test]
    fn allowing_no_expiry_again_makes_such_a_key_and_changes_no_other() {
        let f = fixture();
        let t = tenant(&f);
        f.tenants.allow_no_expiry(false);
        let dated = f
            .tenants
            .create_key(Actor::Cli, Some(t), vcon_scopes(), "dated")
            .expect("a dated key is made while no expiry is refused");
        f.tenants.allow_no_expiry(true);
        let forever = f
            .tenants
            .create_key_for(
                Actor::Cli,
                Some(t),
                vcon_scopes(),
                "forever",
                KeyLife::Never,
            )
            .expect("made");
        assert_eq!(row(&f, forever.id).expires_at, None, "no expiry");
        assert_eq!(
            row(&f, dated.id).expires_at,
            Some(start() + 90 * DAY),
            "the earlier key keeps its expiry"
        );
    }

    #[test]
    fn a_key_with_no_expiry_made_earlier_still_works_after_the_configuration_refuses_them() {
        let f = fixture();
        let made = key(&f, KeyLife::Never);
        f.tenants.allow_no_expiry(false);
        assert!(auth(&f, &made).is_ok(), "the setting is about new keys");
    }
}
