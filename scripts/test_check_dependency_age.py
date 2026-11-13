"""Tests scripts/check_dependency_age.py: a crate version younger than 14
days is taken only under a dependency-age-exception line that names an
advisory.

No test here reaches crates.io. The command's `--registry` mode reads the
registry's index over HTTPS, and a test that did the same would fail with
the network and would send requests from every commit hook. The tests
drive the conversion instead: `published` turns the text of an index file
into a publish time, and `registry_problems` takes the function that
fetches that text as an argument, so a test hands it a dictionary. The
one line not driven is the `urllib` call in `fetch_index`; CI runs it on
every pull request (ci.yml, job `scan`).

Run with: python3 -I scripts/test_check_dependency_age.py
"""

import datetime
import importlib.util
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("check_dependency_age", HERE / "check_dependency_age.py")
age = importlib.util.module_from_spec(spec)
spec.loader.exec_module(age)

CRATES = "registry+https://github.com/rust-lang/crates.io-index"
UTC = datetime.timezone.utc
NOW = datetime.datetime(2026, 11, 13, 12, 0, 0, tzinfo=UTC)


def lock(*packages):
    """A Cargo.lock holding (name, version) or (name, version, source)."""
    text = "version = 4\n"
    for package in packages:
        name, version = package[0], package[1]
        source = package[2] if len(package) > 2 else CRATES
        text += f'\n[[package]]\nname = "{name}"\nversion = "{version}"\n'
        if source is not None:
            text += f'source = "{source}"\n'
    return text


def index(name, *versions):
    """An index file: one JSON line per (version, pubtime or None)."""
    lines = []
    for version, pubtime in versions:
        entry = {"name": name, "vers": version, "deps": [], "yanked": False}
        if pubtime is not None:
            entry["pubtime"] = pubtime
        lines.append(json.dumps(entry))
    return "\n".join(lines) + "\n"


def line(package="rusqlite", version="0.42.0", advisory="RUSTSEC-2026-0459", published="2026-11-11"):
    return f"# dependency-age-exception: {package} {version} {advisory} published {published}"


def exception(package="rusqlite", version="0.42.0", advisory="RUSTSEC-2026-0459", published="2026-11-11", path="Cargo.toml", number=1):
    return age.Exception_(package, version, advisory, published, path, number)


def parsed(text, path="Cargo.toml"):
    return age.exceptions_in(text, path)


def wire(files):
    """A fetch that answers from a dictionary and counts what it was asked."""
    asked = []

    def fetch(name):
        asked.append(name)
        if name not in files:
            raise age.NotInRegistry(name)
        return files[name]

    fetch.asked = asked
    return fetch


INDEX = {
    "rusqlite": index("rusqlite", ("0.41.1", "2026-09-01T10:00:00Z"), ("0.42.0", "2026-11-11T06:47:16Z")),
    "rmcp": index("rmcp", ("3.8.0", "2026-10-01T09:00:00Z"), ("3.9.0", "2026-11-05T16:07:10Z")),
    "serde": index("serde", ("1.0.300", "2026-10-30T12:00:00Z"), ("1.0.301", "2026-11-12T12:00:00Z")),
}


class Success(unittest.TestCase):
    def test_a_line_of_the_form_is_read_with_its_four_values(self):
        found, problems = parsed(line() + "\n")
        self.assertEqual(problems, [])
        self.assertEqual(found, [exception()])

    def test_each_kind_of_advisory_id_is_taken(self):
        for advisory in ("RUSTSEC-2026-0459", "GHSA-c9f9-cp8h-jcj5", "CVE-2026-119375", "CVE-2026-1234"):
            found, problems = parsed(line(advisory=advisory))
            self.assertEqual(problems, [], advisory)
            self.assertEqual(found[0].advisory, advisory)

    def test_a_line_is_found_between_other_manifest_lines_with_its_line_number(self):
        text = '[dependencies]\nserde = "1"\n' + line() + '\nrusqlite = "0.42.0"\n'
        found, problems = parsed(text, "crates/x/Cargo.toml")
        self.assertEqual(problems, [])
        self.assertEqual((found[0].path, found[0].line), ("crates/x/Cargo.toml", 3))

    def test_the_locked_registry_packages_are_read_from_a_lockfile(self):
        self.assertEqual(age.locked(lock(("serde", "1.0.300"), ("rmcp", "3.9.0"))), {("serde", "1.0.300"), ("rmcp", "3.9.0")})

    def test_a_version_the_base_did_not_lock_is_new(self):
        base = {"Cargo.lock": lock(("serde", "1.0.300"))}
        head = {"Cargo.lock": lock(("serde", "1.0.301"))}
        self.assertEqual(age.new_versions(base, head), [("serde", "1.0.301")])

    def test_the_publish_time_of_a_version_is_read_from_the_index_text(self):
        self.assertEqual(age.published(INDEX["rmcp"], "3.9.0"), datetime.datetime(2026, 11, 5, 16, 7, 10, tzinfo=UTC))

    def test_a_version_fourteen_days_old_or_older_needs_no_exception(self):
        self.assertIsNone(age.verdict("rmcp", "3.8.0", datetime.datetime(2026, 10, 1, 9, tzinfo=UTC), NOW, []))

    def test_a_young_version_is_taken_when_an_exception_names_it(self):
        when = datetime.datetime(2026, 11, 11, 6, 47, 16, tzinfo=UTC)
        self.assertIsNone(age.verdict("rusqlite", "0.42.0", when, NOW, [exception()]))

    def test_the_offline_rules_pass_when_the_lock_holds_the_fixed_version(self):
        self.assertEqual(age.offline_problems([exception()], {"Cargo.lock": lock(("rusqlite", "0.42.0"))}), [])

    def test_the_offline_rules_pass_when_the_lock_has_moved_past_the_fixed_version(self):
        self.assertEqual(age.offline_problems([exception()], {"Cargo.lock": lock(("rusqlite", "0.43.1"))}), [])

    def test_the_registry_run_passes_for_a_young_version_with_its_exception(self):
        fetch = wire(INDEX)
        self.assertEqual(age.registry_problems([("rusqlite", "0.42.0")], [exception()], fetch, NOW), [])

    def test_the_registry_run_passes_for_an_old_version_without_an_exception(self):
        self.assertEqual(age.registry_problems([("serde", "1.0.300")], [], wire(INDEX), NOW), [])

    def test_a_recorded_date_equal_to_the_registrys_passes_with_no_new_version(self):
        both = [exception(), exception("rmcp", "3.9.0", "RUSTSEC-2026-0462", "2026-11-05", number=2)]
        self.assertEqual(age.registry_problems([], both, wire(INDEX), NOW), [])

    def test_the_index_path_follows_the_registrys_layout(self):
        self.assertEqual(age.index_path("rusqlite"), "ru/sq/rusqlite")
        self.assertEqual(age.index_path("rmcp"), "rm/cp/rmcp")

    def test_the_whole_run_passes_on_a_bump_with_its_exception(self):
        head = {"Cargo.toml": line() + "\n", "Cargo.lock": lock(("rusqlite", "0.42.0"))}
        base = {"Cargo.lock": lock(("rusqlite", "0.41.1"))}
        found = age.run(sorted(head), head.get, base.get, wire(INDEX), NOW)
        self.assertEqual(found, [])

    def test_the_trees_own_manifests_hold_the_two_exceptions_in_the_form(self):
        # rmcp 3.9.0 (#150) and rusqlite 0.42.0 were taken inside the 14
        # days by hand; each brought crates of its own that were as young.
        root = HERE.parent
        found, problems = [], []
        for path in ("Cargo.toml", "crates/vcon-server/Cargo.toml", "fuzz/Cargo.toml"):
            one, bad = parsed((root / path).read_text(encoding="utf-8"), path)
            found += one
            problems += bad
        self.assertEqual(problems, [])
        self.assertEqual(
            sorted((e.package, e.version, e.advisory, e.published) for e in found),
            [
                ("libsqlite3-sys", "0.40.0", "RUSTSEC-2026-0459", "2026-11-11"),
                ("rmcp", "3.9.0", "RUSTSEC-2026-0462", "2026-11-05"),
                ("rmcp-macros", "3.9.0", "RUSTSEC-2026-0462", "2026-11-05"),
                ("rusqlite", "0.42.0", "RUSTSEC-2026-0459", "2026-11-11"),
                ("sqlite-wasm-rs", "0.6.3", "RUSTSEC-2026-0459", "2026-11-10"),
            ],
        )

    def test_a_crate_that_comes_with_the_fix_is_taken_under_its_own_line(self):
        when = datetime.datetime(2026, 11, 11, 6, 40, tzinfo=UTC)
        both = [exception(), exception("libsqlite3-sys", "0.40.0", number=2)]
        self.assertIsNone(age.verdict("libsqlite3-sys", "0.40.0", when, NOW, both))
        self.assertIn("no dependency-age-exception line names it", age.verdict("libsqlite3-sys", "0.40.0", when, NOW, [exception()]))

    def test_contributing_states_the_form_and_the_number_of_days(self):
        text = (HERE.parent / "CONTRIBUTING.md").read_text(encoding="utf-8")
        self.assertIn(age.FORM, text)
        self.assertIn(f"at least {age.MIN_AGE_DAYS} days", text)
        self.assertIn("scripts/check_dependency_age.py", text)

    def test_the_number_of_days_is_the_one_dependabot_is_held_to(self):
        other = importlib.util.spec_from_file_location("check_dependabot", HERE / "check_dependabot.py")
        module = importlib.util.module_from_spec(other)
        other.loader.exec_module(module)
        self.assertEqual(age.MIN_AGE_DAYS, module.MIN_AGE_DAYS)


class Failure(unittest.TestCase):
    def test_a_young_version_with_no_exception_is_refused(self):
        when = datetime.datetime(2026, 11, 12, 12, tzinfo=UTC)
        self.assertEqual(
            age.verdict("serde", "1.0.301", when, NOW, []),
            "serde 1.0.301 was published on 2026-11-12, 1 day ago, under the 14 days CONTRIBUTING.md asks; "
            "no dependency-age-exception line names it with an advisory",
        )

    def test_an_exception_for_another_version_does_not_cover_a_young_one(self):
        when = datetime.datetime(2026, 11, 12, 12, tzinfo=UTC)
        found = age.verdict("rusqlite", "0.42.1", when, NOW, [exception()])
        self.assertIn("rusqlite 0.42.1 was published on 2026-11-12", found)
        self.assertIn("the exception at Cargo.toml:1 names 0.42.0", found)

    def test_an_exception_for_another_package_does_not_cover_a_young_one(self):
        when = datetime.datetime(2026, 11, 12, 12, tzinfo=UTC)
        found = age.verdict("serde", "0.42.0", when, NOW, [exception()])
        self.assertIn("no dependency-age-exception line names it", found)

    def test_a_line_with_no_advisory_id_is_refused(self):
        found, problems = parsed("# dependency-age-exception: rusqlite 0.42.0 published 2026-11-11\n")
        self.assertEqual(found, [])
        self.assertEqual(len(problems), 1)
        self.assertIn("Cargo.toml:1: not of the form", problems[0])
        self.assertIn(age.FORM, problems[0])

    def test_a_word_that_is_not_an_advisory_id_is_refused(self):
        for advisory in ("security", "RUSTSEC-2026", "GHSA-c9f9-cp8h", "CVE-26-1", "rustsec-2026-0459", "GHSA-C9F9-CP8H-JCJ5", "RUSTSEC-2026-04590"):
            found, problems = parsed(line(advisory=advisory))
            self.assertEqual(found, [], advisory)
            self.assertEqual(len(problems), 1, advisory)

    def test_a_line_with_no_publish_date_is_refused(self):
        found, problems = parsed("# dependency-age-exception: rusqlite 0.42.0 RUSTSEC-2026-0459\n")
        self.assertEqual(found, [])
        self.assertEqual(len(problems), 1)

    def test_a_date_that_is_no_day_of_the_calendar_is_refused(self):
        for published in ("2026-02-30", "2026-13-01", "2026-11-1", "11/11/2026"):
            found, problems = parsed(line(published=published))
            self.assertEqual(found, [], published)
            self.assertEqual(len(problems), 1, published)

    def test_words_after_the_date_or_before_the_marker_are_refused(self):
        for text in (line() + " (see the changelog)", line() + "T06:47:16Z", 'rusqlite = "0.42.0" ' + line()):
            found, problems = parsed(text)
            self.assertEqual(found, [], text)
            self.assertEqual(len(problems), 1, text)

    def test_a_version_requirement_in_place_of_a_version_is_refused(self):
        for version in ("0.42", "^0.42.0", ">=0.42.0", "0.42.x", "v0.42.0"):
            found, problems = parsed(line(version=version))
            self.assertEqual(found, [], version)
            self.assertEqual(len(problems), 1, version)

    def test_a_misspelled_marker_is_refused_and_not_passed_over(self):
        for text in (
            "#dependency-age-exception: rusqlite 0.42.0 RUSTSEC-2026-0459 published 2026-11-11",
            "# Dependency-Age-Exception: rusqlite 0.42.0 RUSTSEC-2026-0459 published 2026-11-11",
            "# dependency-age-exception rusqlite 0.42.0 RUSTSEC-2026-0459 published 2026-11-11",
            "# dependency_age_exception: rusqlite 0.42.0 RUSTSEC-2026-0459 published 2026-11-11",
        ):
            found, problems = parsed(text)
            self.assertEqual(found, [], text)
            self.assertEqual(len(problems), 1, text)

    def test_the_same_package_and_version_excepted_twice_is_refused(self):
        twice = [exception(), exception(path="fuzz/Cargo.toml", number=7)]
        found = age.offline_problems(twice, {"Cargo.lock": lock(("rusqlite", "0.42.0"))})
        self.assertEqual(found, ["fuzz/Cargo.toml:7: rusqlite 0.42.0 already has an exception at Cargo.toml:1"])

    def test_an_exception_whose_fix_the_lock_does_not_hold_is_refused(self):
        found = age.offline_problems([exception()], {"Cargo.lock": lock(("rusqlite", "0.41.1"))})
        self.assertEqual(found, ["Cargo.toml:1: rusqlite 0.42.0 is named as the fixed version, and no Cargo.lock holds rusqlite 0.42.0 or later"])

    def test_an_exception_for_a_package_no_lock_holds_is_refused(self):
        found = age.offline_problems([exception()], {"Cargo.lock": lock(("serde", "1.0.300"))})
        self.assertEqual(found, ["Cargo.toml:1: no Cargo.lock holds a crates.io package named rusqlite"])

    def test_a_recorded_date_that_differs_from_the_registrys_is_refused(self):
        wrong = [exception(published="2026-10-11")]
        found = age.registry_problems([], wrong, wire(INDEX), NOW)
        self.assertEqual(found, ["Cargo.toml:1: rusqlite 0.42.0 is recorded as published on 2026-10-11; crates.io says 2026-11-11"])

    def test_an_exception_for_a_version_the_registry_does_not_hold_is_refused(self):
        found = age.registry_problems([], [exception(version="0.42.9")], wire(INDEX), NOW)
        self.assertEqual(found, ["Cargo.toml:1: crates.io holds no rusqlite 0.42.9"])

    def test_an_exception_for_a_package_the_registry_does_not_hold_is_refused(self):
        found = age.registry_problems([], [exception(package="rusqlit")], wire(INDEX), NOW)
        self.assertEqual(found, ["Cargo.toml:1: crates.io holds no package named rusqlit"])

    def test_a_new_version_the_registry_gives_no_time_for_is_refused(self):
        files = {"old": index("old", ("0.1.0", None))}
        found = age.registry_problems([("old", "0.1.0")], [], wire(files), NOW)
        self.assertEqual(found, ["old 0.1.0: crates.io gives no publish time, so its age is not known"])

    def test_a_registry_that_does_not_answer_is_a_problem_and_not_a_pass(self):
        def fetch(name):
            raise OSError("timed out")

        found = age.registry_problems([("serde", "1.0.301")], [], fetch, NOW)
        self.assertEqual(found, ["serde 1.0.301: crates.io did not answer (timed out)"])

    def test_the_registry_run_refuses_a_young_version_with_no_exception(self):
        found = age.registry_problems([("serde", "1.0.301")], [], wire(INDEX), NOW)
        self.assertEqual(len(found), 1)
        self.assertIn("serde 1.0.301 was published on 2026-11-12, 1 day ago", found[0])

    def test_the_whole_run_refuses_a_bump_to_a_young_version_with_no_line(self):
        head = {"Cargo.toml": 'serde = "1"\n', "Cargo.lock": lock(("serde", "1.0.301"))}
        base = {"Cargo.lock": lock(("serde", "1.0.300"))}
        found = age.run(sorted(head), head.get, base.get, wire(INDEX), NOW)
        self.assertEqual(len(found), 1)
        self.assertIn("serde 1.0.301", found[0])

    def test_the_whole_run_names_a_malformed_line_before_asking_the_registry(self):
        head = {"Cargo.toml": "# dependency-age-exception: rusqlite 0.42.0\n", "Cargo.lock": lock(("rusqlite", "0.42.0"))}
        fetch = wire(INDEX)
        found = age.run(sorted(head), head.get, {}.get, fetch, NOW)
        self.assertEqual(len(found), 1)
        self.assertIn("not of the form", found[0])
        self.assertEqual(fetch.asked, [])

    def test_a_package_name_the_index_cannot_hold_is_refused(self):
        for name in ("", "a/b", "../x", "a b", "naïve"):
            with self.assertRaises(ValueError, msg=name):
                age.index_path(name)

    def test_a_lockfile_that_is_not_toml_is_named(self):
        with self.assertRaises(ValueError) as raised:
            age.run(["Cargo.lock"], {"Cargo.lock": "[[package"}.get, {}.get, wire({}), NOW)
        self.assertIn("Cargo.lock", str(raised.exception))


class Edge(unittest.TestCase):
    def test_exactly_fourteen_days_is_old_enough(self):
        when = NOW - datetime.timedelta(days=14)
        self.assertIsNone(age.verdict("serde", "1.0.0", when, NOW, []))

    def test_one_second_under_fourteen_days_is_refused(self):
        when = NOW - datetime.timedelta(days=14) + datetime.timedelta(seconds=1)
        found = age.verdict("serde", "1.0.0", when, NOW, [])
        self.assertIn("13 days ago", found)

    def test_a_publish_time_after_the_clock_is_refused_as_young(self):
        when = NOW + datetime.timedelta(hours=3)
        found = age.verdict("serde", "1.0.0", when, NOW, [])
        self.assertIn("0 days ago", found)

    def test_an_exception_stays_valid_after_its_version_is_fourteen_days_old(self):
        later = NOW + datetime.timedelta(days=60)
        self.assertEqual(age.registry_problems([], [exception()], wire(INDEX), later), [])

    def test_a_git_or_path_package_is_not_a_registry_version(self):
        text = lock(("rusqlite", "0.42.0", "git+https://example.com/fork?rev=1#abc"), ("vcon-server", "0.5.0", None), ("serde", "1.0.300"))
        self.assertEqual(age.locked(text), {("serde", "1.0.300")})

    def test_a_package_of_another_registry_is_not_a_cratesio_version(self):
        text = lock(("inner", "1.0.0", "registry+https://example.com/index"))
        self.assertEqual(age.locked(text), set())

    def test_a_lockfile_with_no_packages_locks_nothing(self):
        self.assertEqual(age.locked("version = 4\n"), set())

    def test_two_locked_versions_of_one_package_are_both_seen(self):
        text = lock(("syn", "1.0.109"), ("syn", "2.0.90"))
        self.assertEqual(age.locked(text), {("syn", "1.0.109"), ("syn", "2.0.90")})

    def test_a_version_new_in_two_lockfiles_is_asked_about_once(self):
        base = {"Cargo.lock": lock(("serde", "1.0.300")), "fuzz/Cargo.lock": lock(("serde", "1.0.300"))}
        head = {"Cargo.lock": lock(("serde", "1.0.301")), "fuzz/Cargo.lock": lock(("serde", "1.0.301"))}
        self.assertEqual(age.new_versions(base, head), [("serde", "1.0.301")])

    def test_a_version_the_base_held_in_another_lockfile_is_still_new_to_this_one(self):
        base = {"Cargo.lock": lock(("serde", "1.0.300")), "fuzz/Cargo.lock": lock(("serde", "1.0.301"))}
        head = {"Cargo.lock": lock(("serde", "1.0.301")), "fuzz/Cargo.lock": lock(("serde", "1.0.301"))}
        self.assertEqual(age.new_versions(base, head), [("serde", "1.0.301")])

    def test_a_lockfile_the_base_did_not_have_is_all_new(self):
        head = {"fuzz/Cargo.lock": lock(("serde", "1.0.300"), ("rmcp", "3.9.0"))}
        self.assertEqual(age.new_versions({}, head), [("rmcp", "3.9.0"), ("serde", "1.0.300")])

    def test_a_version_removed_or_unchanged_is_not_new(self):
        base = {"Cargo.lock": lock(("serde", "1.0.300"), ("rmcp", "3.9.0"))}
        head = {"Cargo.lock": lock(("serde", "1.0.300"))}
        self.assertEqual(age.new_versions(base, head), [])

    def test_with_no_base_no_version_is_new_and_the_recorded_dates_are_still_checked(self):
        head = {"Cargo.toml": line(published="2026-11-10") + "\n", "Cargo.lock": lock(("rusqlite", "0.42.0"), ("serde", "1.0.301"))}
        fetch = wire(INDEX)
        found = age.run(sorted(head), head.get, None, fetch, NOW)
        self.assertEqual(found, ["Cargo.toml:1: rusqlite 0.42.0 is recorded as published on 2026-11-10; crates.io says 2026-11-11"])
        self.assertEqual(fetch.asked, ["rusqlite"])

    def test_with_no_fetch_the_run_is_offline_and_asks_nothing(self):
        head = {"Cargo.toml": line(published="2026-11-10") + "\n", "Cargo.lock": lock(("rusqlite", "0.42.0"))}
        self.assertEqual(age.run(sorted(head), head.get, None, None, NOW), [])

    def test_each_package_is_fetched_once_for_two_of_its_versions(self):
        files = {"syn": index("syn", ("1.0.109", "2025-01-01T00:00:00Z"), ("2.0.90", "2025-06-01T00:00:00Z"))}
        fetch = wire(files)
        self.assertEqual(age.registry_problems([("syn", "1.0.109"), ("syn", "2.0.90")], [], fetch, NOW), [])
        self.assertEqual(fetch.asked, ["syn"])

    def test_index_paths_of_short_names_follow_the_registrys_layout(self):
        self.assertEqual(age.index_path("a"), "1/a")
        self.assertEqual(age.index_path("cc"), "2/cc")
        self.assertEqual(age.index_path("syn"), "3/s/syn")
        self.assertEqual(age.index_path("Inflector"), "in/fl/inflector")

    def test_a_publish_time_with_fractions_or_an_offset_is_read_as_utc(self):
        text = index("x", ("1.0.0", "2026-11-05T16:07:10.311069Z"), ("1.0.1", "2026-11-06T01:00:00+02:00"))
        self.assertEqual(age.published(text, "1.0.0"), datetime.datetime(2026, 11, 5, 16, 7, 10, 311069, tzinfo=UTC))
        self.assertEqual(age.published(text, "1.0.1"), datetime.datetime(2026, 11, 5, 23, 0, 0, tzinfo=UTC))

    def test_the_recorded_date_is_the_utc_day_of_the_publish_time(self):
        files = {"x": index("x", ("1.0.0", "2026-11-06T01:00:00+02:00"))}
        one = [exception("x", "1.0.0", published="2026-11-05")]
        self.assertEqual(age.registry_problems([], one, wire(files), NOW), [])

    def test_a_version_absent_from_the_index_text_has_no_time_and_raises_nothing(self):
        self.assertIs(age.published(INDEX["rmcp"], "9.9.9"), age.ABSENT)
        self.assertIsNone(age.published(index("old", ("0.1.0", None)), "0.1.0"))

    def test_blank_lines_in_the_index_text_are_passed_over(self):
        text = "\n" + INDEX["rmcp"] + "\n\n"
        self.assertEqual(age.published(text, "3.8.0"), datetime.datetime(2026, 10, 1, 9, 0, 0, tzinfo=UTC))

    def test_versions_compare_by_number_and_a_prerelease_is_before_its_release(self):
        self.assertLess(age.version_key("0.9.0"), age.version_key("0.10.0"))
        self.assertLess(age.version_key("1.0.0-rc.1"), age.version_key("1.0.0"))
        self.assertEqual(age.version_key("1.0.0+build.5"), age.version_key("1.0.0"))

    def test_a_prerelease_of_the_fixed_version_does_not_hold_the_fix(self):
        found = age.offline_problems([exception()], {"Cargo.lock": lock(("rusqlite", "0.42.0-rc.1"))})
        self.assertEqual(len(found), 1)
        self.assertIn("no Cargo.lock holds rusqlite 0.42.0 or later", found[0])

    def test_the_fix_held_by_one_of_two_lockfiles_is_enough(self):
        locks = {"Cargo.lock": lock(("rusqlite", "0.42.0")), "fuzz/Cargo.lock": lock(("serde", "1.0.300"))}
        self.assertEqual(age.offline_problems([exception()], locks), [])

    def test_a_line_indented_or_with_spaces_at_its_end_is_read(self):
        found, problems = parsed("    " + line() + "   \n")
        self.assertEqual(problems, [])
        self.assertEqual(found, [exception()])

    def test_a_manifest_with_no_line_has_no_exception_and_no_problem(self):
        self.assertEqual(parsed('[package]\nname = "x"\n# 0.42.0 is the first with the fix\n'), ([], []))

    def test_one_day_is_written_in_the_singular(self):
        when = NOW - datetime.timedelta(days=2)
        self.assertIn("2 days ago", age.verdict("serde", "1.0.0", when, NOW, []))
        when = NOW - datetime.timedelta(days=1, hours=5)
        self.assertIn("1 day ago", age.verdict("serde", "1.0.0", when, NOW, []))

    def test_only_files_named_cargo_toml_and_cargo_lock_are_read(self):
        files = {"docs/Cargo.toml.md": line(), "Cargo.lock.bak": "[[package", "Cargo.toml": "", "Cargo.lock": lock()}
        self.assertEqual(age.run(sorted(files), files.get, None, None, NOW), [])


class Command(unittest.TestCase):
    """The command in a repository of its own, without --registry."""

    def repository(self, tmp, manifest, lockfile):
        root = pathlib.Path(tmp)
        (root / "Cargo.toml").write_text(manifest, encoding="utf-8")
        (root / "Cargo.lock").write_text(lockfile, encoding="utf-8")
        env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        subprocess.run(["git", "init", "-q"], cwd=root, env=env, check=True)
        subprocess.run(["git", "add", "."], cwd=root, env=env, check=True)
        return subprocess.run([sys.executable, "-I", str(HERE / "check_dependency_age.py")], cwd=root, env=env, capture_output=True, text=True)

    def test_success_the_command_passes_and_counts_the_exceptions(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = self.repository(tmp, line() + "\n", lock(("rusqlite", "0.42.0")))
        self.assertEqual((run.returncode, run.stderr), (0, ""))
        self.assertEqual(run.stdout, "check_dependency_age: ok (1 exception, 1 lockfile; publish dates not checked without --registry)\n")

    def test_failure_the_command_names_a_malformed_line_and_exits_1(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = self.repository(tmp, "# dependency-age-exception: rusqlite 0.42.0\n", lock(("rusqlite", "0.42.0")))
        self.assertEqual(run.returncode, 1)
        self.assertIn("check_dependency_age: Cargo.toml:1: not of the form", run.stderr)

    def test_edge_an_unknown_argument_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            run = subprocess.run([sys.executable, "-I", str(HERE / "check_dependency_age.py"), "--registery"], cwd=root, capture_output=True, text=True)
        self.assertEqual(run.returncode, 2)

    def plain(self, tmp, files, *arguments):
        """The command with --walk in a directory that is no repository:
        the commit hook checks a copy of the index, which has no .git."""
        root = pathlib.Path(tmp)
        for path, text in files.items():
            (root / path).parent.mkdir(parents=True, exist_ok=True)
            (root / path).write_text(text, encoding="utf-8")
        env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        env["GIT_CEILING_DIRECTORIES"] = str(root.parent)
        return subprocess.run([sys.executable, "-I", str(HERE / "check_dependency_age.py"), *arguments], cwd=root, env=env, capture_output=True, text=True)

    def test_success_with_walk_the_command_needs_no_repository(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = self.plain(tmp, {"Cargo.toml": line() + "\n", "Cargo.lock": lock(("rusqlite", "0.42.0")), "fuzz/Cargo.lock": lock()}, "--walk")
        self.assertEqual((run.returncode, run.stderr), (0, ""))
        self.assertEqual(run.stdout, "check_dependency_age: ok (1 exception, 2 lockfiles; publish dates not checked without --registry)\n")

    def test_failure_with_walk_a_line_in_a_nested_manifest_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = self.plain(tmp, {"Cargo.lock": lock(), "crates/x/Cargo.toml": "# dependency-age-exception: rusqlite\n"}, "--walk")
        self.assertEqual(run.returncode, 1)
        self.assertIn("check_dependency_age: crates/x/Cargo.toml:1: not of the form", run.stderr)

    def test_failure_outside_a_repository_without_walk_the_command_says_so(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = self.plain(tmp, {"Cargo.toml": "", "Cargo.lock": lock()})
        self.assertEqual(run.returncode, 2)
        self.assertEqual(run.stderr, "check_dependency_age: not in a git repository; --walk reads the files under this directory\n")

    def test_failure_walk_with_a_base_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = self.plain(tmp, {"Cargo.lock": lock()}, "--walk", "--registry", "--base", "HEAD")
        self.assertEqual(run.returncode, 2)
        self.assertIn("--base needs a repository, not --walk", run.stderr)

    def test_edge_walk_passes_over_build_output_and_installed_packages(self):
        bad = "# dependency-age-exception: rusqlite\n"
        files = {"Cargo.toml": "", "Cargo.lock": lock(), "target/package/x/Cargo.toml": bad, "site/node_modules/y/Cargo.toml": bad, ".git/Cargo.toml": bad}
        with tempfile.TemporaryDirectory() as tmp:
            run = self.plain(tmp, files, "--walk")
        self.assertEqual((run.returncode, run.stderr), (0, ""))

    def test_failure_a_base_without_the_registry_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = subprocess.run([sys.executable, "-I", str(HERE / "check_dependency_age.py"), "--base", "HEAD"], cwd=tmp, capture_output=True, text=True)
        self.assertEqual(run.returncode, 2)
        self.assertIn("--base needs --registry", run.stderr)


if __name__ == "__main__":
    unittest.main()
