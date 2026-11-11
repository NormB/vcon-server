//! A tenant's settings: the fixed list of names a tenant administrator may
//! set, what each may hold, and what each is while unset.
//!
//! This is the one list of names. The registry keeps a row for each
//! setting that was set (`tenant_setting`: tenant, name, value as
//! [`Value::text`] writes it); a name with no row has its default. A row
//! whose name or value this list does not read is damage, not a default.
//!
//! The names are `masking.<role>` for each tenant role (whether the
//! console masks personal data for a user holding the role; on while
//! unset), and `session.idle_seconds` and `session.absolute_seconds` (how
//! long a session lasts without a request, and at most), each within the
//! bounds this file states, with the idle limit never above the absolute
//! one.

use std::fmt;
use std::ops::RangeInclusive;

use crate::roles::{Role, Roles};

/// The bounds of `session.idle_seconds`: one minute to 15 minutes, which
/// is also what an unset limit is.
pub const IDLE_SECONDS: RangeInclusive<u32> = 60..=900;

/// The bounds of `session.absolute_seconds`: 5 minutes to 8 hours, which
/// is also what an unset limit is.
pub const ABSOLUTE_SECONDS: RangeInclusive<u32> = 300..=28_800;

/// The name of a setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Name {
    /// Whether the console masks personal data for a user holding this
    /// tenant role. The operator role has no setting: [`Name::ALL`] and
    /// [`Name::parse`] never give it, and it reads as masked.
    Masking(Role),
    /// Seconds a session lasts without a request.
    SessionIdle,
    /// Seconds a session lasts at most.
    SessionAbsolute,
}

/// What a setting holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value {
    /// A masking name's value: true while the console masks.
    Flag(bool),
    /// A limit's value.
    Seconds(u32),
}

/// Why a change to the settings was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingError {
    /// A name outside [`Name::ALL`].
    UnknownName,
    /// A value that is not a flag for a masking name or a whole number
    /// for a limit, or a change that is not an object of names.
    BadValue,
    /// A limit outside [`IDLE_SECONDS`] or [`ABSOLUTE_SECONDS`].
    OutOfBounds,
    /// The change would leave the idle limit above the absolute one.
    IdleOverAbsolute,
}

impl fmt::Display for SettingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SettingError::UnknownName => "a name is not a setting",
            SettingError::BadValue => "a value is not of its setting's kind",
            SettingError::OutOfBounds => "a limit is outside its bounds",
            SettingError::IdleOverAbsolute => "the idle limit is above the absolute limit",
        })
    }
}

/// One name of a change with the value to store for it, or None to remove
/// its row, which puts the name back to its default.
pub type Change = (Name, Option<Value>);

impl Name {
    /// Every name, in the order a reader sees them.
    pub const ALL: [Name; 8] = [
        Name::Masking(Role::Reader),
        Name::Masking(Role::Producer),
        Name::Masking(Role::Records),
        Name::Masking(Role::Legal),
        Name::Masking(Role::Auditor),
        Name::Masking(Role::TenantAdmin),
        Name::SessionIdle,
        Name::SessionAbsolute,
    ];

    /// The name as the registry, a body and an answer write it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Name::Masking(Role::Reader) => "masking.reader",
            Name::Masking(Role::Producer) => "masking.producer",
            Name::Masking(Role::Records) => "masking.records",
            Name::Masking(Role::Legal) => "masking.legal",
            Name::Masking(Role::Auditor) => "masking.auditor",
            Name::Masking(Role::TenantAdmin) => "masking.tenant-admin",
            Name::Masking(Role::Operator) => "masking.operator",
            Name::SessionIdle => "session.idle_seconds",
            Name::SessionAbsolute => "session.absolute_seconds",
        }
    }

    /// The name written exactly as `text`, if the list has it.
    #[must_use]
    pub fn parse(text: &str) -> Option<Name> {
        Name::ALL.into_iter().find(|name| name.as_str() == text)
    }

    /// The bounds of a limit; None for a masking flag.
    fn bounds(self) -> Option<RangeInclusive<u32>> {
        match self {
            Name::Masking(_) => None,
            Name::SessionIdle => Some(IDLE_SECONDS),
            Name::SessionAbsolute => Some(ABSOLUTE_SECONDS),
        }
    }

    /// The value a stored row's text holds for this name: only text that
    /// [`Value::text`] writes, within the name's bounds.
    #[must_use]
    pub fn read(self, text: &str) -> Option<Value> {
        let value = match self.bounds() {
            None => Value::Flag(text.parse().ok()?),
            Some(bounds) => {
                let seconds = text.parse().ok()?;
                bounds
                    .contains(&seconds)
                    .then_some(Value::Seconds(seconds))?
            }
        };
        (value.text() == text).then_some(value)
    }

    /// The value a change's JSON holds for this name.
    fn taken(self, value: &serde_json::Value) -> Result<Value, SettingError> {
        let Some(bounds) = self.bounds() else {
            return value
                .as_bool()
                .map(Value::Flag)
                .ok_or(SettingError::BadValue);
        };
        let seconds = value.as_u64().ok_or(SettingError::BadValue)?;
        u32::try_from(seconds)
            .ok()
            .filter(|seconds| bounds.contains(seconds))
            .map(Value::Seconds)
            .ok_or(SettingError::OutOfBounds)
    }
}

impl Value {
    /// The value as a stored row keeps it.
    #[must_use]
    pub fn text(self) -> String {
        match self {
            Value::Flag(flag) => flag.to_string(),
            Value::Seconds(seconds) => seconds.to_string(),
        }
    }

    fn to_json(self) -> serde_json::Value {
        match self {
            Value::Flag(flag) => flag.into(),
            Value::Seconds(seconds) => seconds.into(),
        }
    }
}

/// The names of a change, comma-separated, in the list's order: what the
/// audit record of the change keeps. It never holds a value.
#[must_use]
pub fn named(changes: &[Change]) -> String {
    let names: Vec<&str> = changes.iter().map(|(name, _)| name.as_str()).collect();
    names.join(",")
}

/// Every setting of one tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// The roles whose masking is off.
    unmasked: Roles,
    idle: u32,
    absolute: u32,
}

impl Default for Settings {
    /// What a tenant that set nothing has.
    fn default() -> Settings {
        Settings {
            unmasked: Roles::default(),
            idle: *IDLE_SECONDS.end(),
            absolute: *ABSOLUTE_SECONDS.end(),
        }
    }
}

impl Settings {
    /// The value of one name.
    #[must_use]
    pub fn get(&self, name: Name) -> Value {
        match name {
            Name::Masking(role) => Value::Flag(!self.unmasked.contains(role)),
            Name::SessionIdle => Value::Seconds(self.idle),
            Name::SessionAbsolute => Value::Seconds(self.absolute),
        }
    }

    fn set(&mut self, name: Name, value: Value) {
        match (name, value) {
            (Name::Masking(role), Value::Flag(true)) => {
                self.unmasked = self.unmasked.without(Roles::of(&[role]));
            }
            (Name::Masking(role), Value::Flag(false)) => {
                self.unmasked = self.unmasked.with(Roles::of(&[role]));
            }
            (Name::SessionIdle, Value::Seconds(seconds)) => self.idle = seconds,
            (Name::SessionAbsolute, Value::Seconds(seconds)) => self.absolute = seconds,
            // `Name::read` and `Name::taken` give each name its own kind.
            (Name::Masking(_), Value::Seconds(_))
            | (Name::SessionIdle | Name::SessionAbsolute, Value::Flag(_)) => {}
        }
    }

    /// The one rule that spans two names.
    fn whole(self) -> Option<Settings> {
        (self.idle <= self.absolute).then_some(self)
    }

    /// The settings a tenant's stored rows (name, value) make over the
    /// defaults. None for damage: a name outside the list or stored twice,
    /// a value its name does not read, or an idle limit above the absolute
    /// one.
    #[must_use]
    pub fn stored(rows: &[(String, String)]) -> Option<Settings> {
        let mut settings = Settings::default();
        let mut seen = Vec::new();
        for (name, value) in rows {
            let name = Name::parse(name)?;
            if seen.contains(&name) {
                return None;
            }
            seen.push(name);
            settings.set(name, name.read(value)?);
        }
        settings.whole()
    }

    /// These settings after a change: a JSON object of names, each with
    /// its new value or null for its default. Also gives the rows to
    /// write and remove, in the list's order. Nothing of a refused change
    /// is taken.
    ///
    /// # Errors
    ///
    /// A [`SettingError`] for a change that is not an object, names a
    /// setting outside the list, gives a value of the wrong kind or a limit
    /// outside its bounds, or would leave the idle limit above the absolute
    /// one.
    pub fn changed(
        &self,
        change: &serde_json::Value,
    ) -> Result<(Settings, Vec<Change>), SettingError> {
        let change = change.as_object().ok_or(SettingError::BadValue)?;
        if change.keys().any(|name| Name::parse(name).is_none()) {
            return Err(SettingError::UnknownName);
        }
        let mut settings = *self;
        let mut changes = Vec::new();
        for name in Name::ALL {
            let Some(value) = change.get(name.as_str()) else {
                continue;
            };
            let stored = if value.is_null() {
                settings.set(name, Settings::default().get(name));
                None
            } else {
                let value = name.taken(value)?;
                settings.set(name, value);
                Some(value)
            };
            changes.push((name, stored));
        }
        let settings = settings.whole().ok_or(SettingError::IdleOverAbsolute)?;
        Ok((settings, changes))
    }

    /// One object with every name and its value.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        Name::ALL
            .into_iter()
            .map(|name| (name.as_str().to_owned(), self.get(name).to_json()))
            .collect::<serde_json::Map<_, _>>()
            .into()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::roles::Role;

    fn rows(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    // Success.

    #[test]
    fn the_names_are_a_masking_flag_for_each_tenant_role_and_two_limits() {
        let names: Vec<&str> = Name::ALL.iter().map(|name| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "masking.reader",
                "masking.producer",
                "masking.records",
                "masking.legal",
                "masking.auditor",
                "masking.tenant-admin",
                "session.idle_seconds",
                "session.absolute_seconds",
            ],
            "the fixed list of names, in the order a reader sees them"
        );
    }

    #[test]
    fn each_name_reads_back_from_its_text() {
        for name in Name::ALL {
            assert_eq!(
                Name::parse(name.as_str()),
                Some(name),
                "{} reads back",
                name.as_str()
            );
        }
    }

    #[test]
    fn unset_settings_mask_every_role_and_keep_the_longest_limits() {
        let unset = Settings::default();
        for role in Role::ALL.into_iter().filter(|role| *role != Role::Operator) {
            assert_eq!(
                unset.get(Name::Masking(role)),
                Value::Flag(true),
                "masking is on for {} while unset",
                role.name()
            );
        }
        assert_eq!(
            unset.get(Name::SessionIdle),
            Value::Seconds(900),
            "15 minutes idle while unset"
        );
        assert_eq!(
            unset.get(Name::SessionAbsolute),
            Value::Seconds(28_800),
            "8 hours at most while unset"
        );
    }

    #[test]
    fn settings_are_shown_as_one_object_with_every_name() {
        assert_eq!(
            Settings::default().to_json(),
            json!({
                "masking.reader": true,
                "masking.producer": true,
                "masking.records": true,
                "masking.legal": true,
                "masking.auditor": true,
                "masking.tenant-admin": true,
                "session.idle_seconds": 900,
                "session.absolute_seconds": 28_800,
            }),
            "every name with its value"
        );
    }

    #[test]
    fn a_change_sets_the_names_it_holds_and_leaves_the_others() {
        let body = json!({ "masking.legal": false, "session.idle_seconds": 300 });
        let (now, changes) = Settings::default().changed(&body).expect("a good change");
        assert_eq!(
            now.get(Name::Masking(Role::Legal)),
            Value::Flag(false),
            "set"
        );
        assert_eq!(now.get(Name::SessionIdle), Value::Seconds(300), "set");
        assert_eq!(
            now.get(Name::Masking(Role::Reader)),
            Value::Flag(true),
            "a name the change does not hold stays"
        );
        assert_eq!(
            changes,
            vec![
                (Name::Masking(Role::Legal), Some(Value::Flag(false))),
                (Name::SessionIdle, Some(Value::Seconds(300))),
            ],
            "the rows to write, in the list's order"
        );
    }

    #[test]
    fn a_null_puts_a_name_back_to_its_default() {
        let was = Settings::stored(&rows(&[("session.idle_seconds", "120")])).expect("good rows");
        let (now, changes) = was
            .changed(&json!({ "session.idle_seconds": null }))
            .expect("a good change");
        assert_eq!(now, Settings::default(), "the default again");
        assert_eq!(
            changes,
            vec![(Name::SessionIdle, None)],
            "the row to remove"
        );
    }

    #[test]
    fn stored_rows_read_back_over_the_defaults() {
        let read = Settings::stored(&rows(&[
            ("masking.auditor", "false"),
            ("session.absolute_seconds", "3600"),
            ("session.idle_seconds", "600"),
        ]))
        .expect("good rows");
        assert_eq!(
            read.get(Name::Masking(Role::Auditor)),
            Value::Flag(false),
            "stored"
        );
        assert_eq!(
            read.get(Name::SessionAbsolute),
            Value::Seconds(3600),
            "stored"
        );
        assert_eq!(read.get(Name::SessionIdle), Value::Seconds(600), "stored");
        assert_eq!(
            read.get(Name::Masking(Role::Reader)),
            Value::Flag(true),
            "no row: the default"
        );
    }

    #[test]
    fn a_value_is_written_as_the_text_it_reads_back_from() {
        for (name, value, text) in [
            (Name::Masking(Role::Reader), Value::Flag(false), "false"),
            (Name::Masking(Role::Reader), Value::Flag(true), "true"),
            (Name::SessionIdle, Value::Seconds(60), "60"),
            (Name::SessionAbsolute, Value::Seconds(28_800), "28800"),
        ] {
            assert_eq!(value.text(), text, "{value:?} as text");
            assert_eq!(
                name.read(text),
                Some(value),
                "{text} read as {}",
                name.as_str()
            );
        }
    }

    #[test]
    fn both_limits_take_each_end_of_their_bounds() {
        for (body, name, seconds) in [
            (json!({ "session.idle_seconds": 60 }), Name::SessionIdle, 60),
            (
                json!({ "session.idle_seconds": 900 }),
                Name::SessionIdle,
                900,
            ),
            (
                json!({ "session.absolute_seconds": 900 }),
                Name::SessionAbsolute,
                900,
            ),
            (
                json!({ "session.absolute_seconds": 28_800 }),
                Name::SessionAbsolute,
                28_800,
            ),
        ] {
            let (now, _) = Settings::default()
                .changed(&body)
                .unwrap_or_else(|error| panic!("{body} is within the bounds: {error:?}"));
            assert_eq!(now.get(name), Value::Seconds(seconds), "{body}");
        }
    }

    #[test]
    fn a_change_that_states_the_value_a_name_has_still_writes_its_row() {
        let (now, changes) = Settings::default()
            .changed(&json!({ "masking.reader": true }))
            .expect("a good change");
        assert_eq!(now, Settings::default(), "nothing differs");
        assert_eq!(
            changes,
            vec![(Name::Masking(Role::Reader), Some(Value::Flag(true)))],
            "the name is set, not unset: its row is written"
        );
    }

    #[test]
    fn both_limits_may_change_together_to_a_pair_neither_could_reach_alone() {
        let was = Settings::stored(&rows(&[
            ("session.absolute_seconds", "300"),
            ("session.idle_seconds", "120"),
        ]))
        .expect("good rows");
        let body = json!({ "session.idle_seconds": 600, "session.absolute_seconds": 1200 });
        let (now, _) = was.changed(&body).expect("the pair is good");
        assert_eq!(now.get(Name::SessionIdle), Value::Seconds(600), "idle");
        assert_eq!(
            now.get(Name::SessionAbsolute),
            Value::Seconds(1200),
            "absolute"
        );
    }

    #[test]
    fn the_idle_limit_may_equal_the_absolute_one() {
        let body = json!({ "session.idle_seconds": 600, "session.absolute_seconds": 600 });
        let (now, _) = Settings::default().changed(&body).expect("equal is good");
        assert_eq!(now.get(Name::SessionIdle), Value::Seconds(600), "idle");
    }

    // Failure.

    #[test]
    fn a_name_outside_the_list_is_refused() {
        for name in [
            "masking",
            "masking.operator",
            "masking.nobody",
            "session.idle",
            "bulk_export",
            "",
        ] {
            assert_eq!(
                Settings::default().changed(&json!({ name: true })),
                Err(SettingError::UnknownName),
                "{name:?} is not a setting"
            );
        }
    }

    #[test]
    fn a_masking_flag_takes_only_true_or_false() {
        for value in [json!("off"), json!(0), json!(1), json!([]), json!({})] {
            assert_eq!(
                Settings::default().changed(&json!({ "masking.reader": value })),
                Err(SettingError::BadValue),
                "{value} is not a flag"
            );
        }
    }

    #[test]
    fn a_limit_takes_only_a_whole_number() {
        for value in [
            json!("300"),
            json!(300.5),
            json!(-300),
            json!(true),
            json!([300]),
        ] {
            assert_eq!(
                Settings::default().changed(&json!({ "session.idle_seconds": value })),
                Err(SettingError::BadValue),
                "{value} is not a whole number of seconds"
            );
        }
    }

    #[test]
    fn an_idle_limit_outside_its_bounds_is_refused() {
        for seconds in [0, 59, 901, 28_800] {
            assert_eq!(
                Settings::default().changed(&json!({ "session.idle_seconds": seconds })),
                Err(SettingError::OutOfBounds),
                "{seconds} s idle"
            );
        }
    }

    #[test]
    fn an_absolute_limit_outside_its_bounds_is_refused() {
        for seconds in [0, 299, 28_801, 86_400] {
            assert_eq!(
                Settings::default().changed(&json!({ "session.absolute_seconds": seconds })),
                Err(SettingError::OutOfBounds),
                "{seconds} s at most"
            );
        }
    }

    #[test]
    fn an_idle_limit_above_the_absolute_one_is_refused() {
        let body = json!({ "session.idle_seconds": 900, "session.absolute_seconds": 600 });
        assert_eq!(
            Settings::default().changed(&body),
            Err(SettingError::IdleOverAbsolute),
            "both in one change"
        );
        let was = Settings::stored(&rows(&[("session.absolute_seconds", "300")]));
        assert_eq!(was, None, "stored rows hold the rule too: idle is 900");
        let was = Settings::stored(&rows(&[
            ("session.absolute_seconds", "300"),
            ("session.idle_seconds", "120"),
        ]))
        .expect("good rows");
        assert_eq!(
            was.changed(&json!({ "session.idle_seconds": 301 })),
            Err(SettingError::IdleOverAbsolute),
            "against the absolute limit already set"
        );
    }

    #[test]
    fn a_change_that_is_not_an_object_is_refused() {
        for body in [json!([]), json!("masking.reader"), json!(null), json!(7)] {
            assert_eq!(
                Settings::default().changed(&body),
                Err(SettingError::BadValue),
                "{body} is not an object of names"
            );
        }
    }

    #[test]
    fn one_bad_name_refuses_the_whole_change() {
        let body = json!({ "masking.reader": false, "masking.nobody": false });
        assert_eq!(
            Settings::default().changed(&body),
            Err(SettingError::UnknownName),
            "nothing of the change is taken"
        );
    }

    #[test]
    fn a_stored_row_with_an_unknown_name_is_damage() {
        assert_eq!(
            Settings::stored(&rows(&[("masking.operator", "true")])),
            None,
            "not a default"
        );
    }

    #[test]
    fn a_stored_value_that_does_not_read_back_is_damage() {
        for (name, value) in [
            ("masking.reader", "on"),
            ("masking.reader", "TRUE"),
            ("masking.reader", ""),
            ("session.idle_seconds", "0300"),
            ("session.idle_seconds", "+300"),
            ("session.idle_seconds", "300 "),
            ("session.idle_seconds", "59"),
            ("session.absolute_seconds", "28801"),
            ("session.absolute_seconds", "4294967296"),
        ] {
            assert_eq!(
                Settings::stored(&rows(&[(name, value)])),
                None,
                "{name} = {value:?}"
            );
        }
    }

    #[test]
    fn a_name_stored_twice_is_damage() {
        assert_eq!(
            Settings::stored(&rows(&[
                ("masking.reader", "false"),
                ("masking.reader", "true"),
            ])),
            None,
            "one row for a name"
        );
    }

    #[test]
    fn a_name_in_other_letters_is_not_the_name() {
        for name in [
            "Masking.reader",
            "masking.Reader",
            " masking.reader",
            "masking.reader ",
            "masking.tenant_admin",
            "session.idle_seconds\n",
        ] {
            assert_eq!(Name::parse(name), None, "{name:?}");
        }
    }

    // Edge.

    #[test]
    fn an_empty_change_changes_nothing() {
        let (now, changes) = Settings::default()
            .changed(&json!({}))
            .expect("an empty object is a change of nothing");
        assert_eq!(now, Settings::default(), "as it was");
        assert!(changes.is_empty(), "no row to write: {changes:?}");
    }

    #[test]
    fn no_rows_are_the_defaults() {
        assert_eq!(
            Settings::stored(&[]),
            Some(Settings::default()),
            "a tenant that set nothing"
        );
    }

    #[test]
    fn a_null_for_a_name_never_set_removes_nothing_and_is_not_an_error() {
        let (now, changes) = Settings::default()
            .changed(&json!({ "masking.legal": null }))
            .expect("a good change");
        assert_eq!(now, Settings::default(), "as it was");
        assert_eq!(
            changes,
            vec![(Name::Masking(Role::Legal), None)],
            "the removal is still asked of the table"
        );
    }

    #[test]
    fn putting_the_absolute_limit_back_under_a_shorter_idle_limit_is_good() {
        let was = Settings::stored(&rows(&[
            ("session.absolute_seconds", "300"),
            ("session.idle_seconds", "120"),
        ]))
        .expect("good rows");
        let (now, _) = was
            .changed(&json!({ "session.absolute_seconds": null }))
            .expect("8 hours is above 120 s");
        assert_eq!(
            now.get(Name::SessionAbsolute),
            Value::Seconds(28_800),
            "the default"
        );
        assert_eq!(now.get(Name::SessionIdle), Value::Seconds(120), "kept");
    }

    #[test]
    fn putting_the_idle_limit_back_above_a_shorter_absolute_limit_is_refused() {
        let was = Settings::stored(&rows(&[
            ("session.absolute_seconds", "300"),
            ("session.idle_seconds", "120"),
        ]))
        .expect("good rows");
        assert_eq!(
            was.changed(&json!({ "session.idle_seconds": null })),
            Err(SettingError::IdleOverAbsolute),
            "the default idle limit, 900 s, is above 300 s"
        );
    }

    #[test]
    fn the_changes_come_in_the_lists_order_whatever_the_bodys_order() {
        let body = json!({
            "session.absolute_seconds": 3600,
            "masking.tenant-admin": false,
            "masking.reader": false,
        });
        let (_, changes) = Settings::default().changed(&body).expect("a good change");
        let names: Vec<&str> = changes.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "masking.reader",
                "masking.tenant-admin",
                "session.absolute_seconds"
            ],
            "one order for the rows and the audit target"
        );
    }

    #[test]
    fn a_whole_number_written_with_a_fraction_of_zero_is_not_taken() {
        assert_eq!(
            Settings::default().changed(&json!({ "session.idle_seconds": 300.0 })),
            Err(SettingError::BadValue),
            "300.0 is a float in JSON"
        );
    }

    #[test]
    fn a_number_past_32_bits_is_out_of_bounds_not_a_crash() {
        assert_eq!(
            Settings::default().changed(&json!({ "session.absolute_seconds": 4_294_967_296_u64 })),
            Err(SettingError::OutOfBounds),
            "a whole number, too large"
        );
    }

    #[test]
    fn the_bounds_are_the_ones_the_pages_state() {
        assert_eq!(IDLE_SECONDS, 60..=900, "one minute to 15 minutes");
        assert_eq!(ABSOLUTE_SECONDS, 300..=28_800, "5 minutes to 8 hours");
    }

    #[test]
    fn each_error_names_itself_for_a_problem_detail() {
        assert_eq!(
            SettingError::UnknownName.to_string(),
            "a name is not a setting",
            "unknown name"
        );
        assert_eq!(
            SettingError::BadValue.to_string(),
            "a value is not of its setting's kind",
            "bad value"
        );
        assert_eq!(
            SettingError::OutOfBounds.to_string(),
            "a limit is outside its bounds",
            "out of bounds"
        );
        assert_eq!(
            SettingError::IdleOverAbsolute.to_string(),
            "the idle limit is above the absolute limit",
            "idle over absolute"
        );
    }

    #[test]
    fn the_names_written_together_are_the_audit_target() {
        let body = json!({ "masking.reader": false, "session.idle_seconds": null });
        let (_, changes) = Settings::default()
            .changed(&json!({ "masking.reader": false }))
            .expect("a good change");
        assert_eq!(named(&changes), "masking.reader", "one name");
        let was = Settings::default();
        let (_, changes) = was.changed(&body).expect("a good change");
        assert_eq!(
            named(&changes),
            "masking.reader,session.idle_seconds",
            "names only, never a value"
        );
        assert_eq!(named(&[]), "", "no name");
    }
}
