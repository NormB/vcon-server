"""Tests scripts/check_test_runner.py: every test run in the commit hook and
in the workflows has the per-test deadline, because each runs through
cargo-nextest at the pinned version with .config/nextest.toml; the only
`cargo test` runs left are the doc tests, which nextest does not run, and
Miri's.

Run with: python3 -I scripts/test_check_test_runner.py
"""

import importlib.util
import pathlib
import sys
import unittest

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("check_test_runner", HERE / "check_test_runner.py")
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

CONFIG = '[profile.default]\nslow-timeout = { period = "60s", terminate-after = 3 }\n'
INSTALL = {"name": "Install cargo-nextest", "run": "scripts/nextest.sh install"}


def job(*runs, install=True):
    steps = [{"uses": "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"}]
    if install:
        steps.append(dict(INSTALL))
    steps += [{"name": f"step {i}", "run": run} for i, run in enumerate(runs)]
    return {"name": "a job", "runs-on": "ubuntu-latest", "steps": steps}


def problems(config=CONFIG, hook="scripts/nextest.sh run --workspace --locked\ncargo test --workspace --doc --locked\n", cargo_toml=None, **jobs):
    workflows = {"ci.yml": {"jobs": jobs or {"rust": job("scripts/nextest.sh run --workspace --locked\ncargo test --workspace --locked --doc")}}}
    return check.problems(config, hook, workflows, cargo_toml)


class Deadline(unittest.TestCase):
    def test_the_rule_holds_for_nextest_runs_and_doc_tests(self):
        self.assertEqual(problems(), [])

    def test_a_config_without_the_deadline_is_refused(self):
        self.assertTrue(any("slow-timeout" in p for p in problems(config="[profile.default]\n")))

    def test_another_deadline_is_refused(self):
        other = CONFIG.replace("terminate-after = 3", "terminate-after = 30")
        self.assertTrue(any("slow-timeout" in p for p in problems(config=other)))

    def test_a_config_that_is_not_toml_is_refused(self):
        self.assertTrue(any("nextest.toml" in p for p in problems(config="[profile.default\n")))


class CargoTest(unittest.TestCase):
    def test_cargo_test_in_the_hook_is_refused(self):
        found = problems(hook="cargo test --workspace --locked --quiet\n")
        self.assertTrue(any("pre-commit" in p and "cargo test" in p for p in found), found)

    def test_cargo_test_in_a_workflow_is_refused(self):
        found = problems(backends=job("cargo test -p vcon-server --lib backend::", install=False))
        self.assertTrue(any("backends" in p and "cargo test" in p for p in found), found)

    def test_cargo_test_with_a_toolchain_is_refused(self):
        found = problems(asan=job('cargo "+$NIGHTLY" test -Zbuild-std --lib', install=False))
        self.assertTrue(any("asan" in p for p in found), found)

    def test_cargo_test_held_in_a_bash_array_is_refused(self):
        found = problems(asan=job('test=(cargo "+$NIGHTLY" test -Zbuild-std\n  --locked -p vcon-server)\n"${test[@]}" -- --skip mlock_', install=False))
        self.assertTrue(any("asan" in p for p in found), found)

    def test_doc_tests_may_run_under_cargo_test(self):
        self.assertEqual(problems(hook="scripts/nextest.sh run --workspace\ncargo test --workspace --locked --quiet --doc\n"), [])

    def test_miri_may_run_under_cargo_test(self):
        self.assertEqual(problems(miri=job("cargo \"+$NIGHTLY\" miri test --locked -p vcon-server --lib", install=False)), [])

    def test_cargo_llvm_cov_without_nextest_is_refused(self):
        found = problems(coverage=job("cargo llvm-cov --workspace --locked --lcov --output-path lcov.info"))
        self.assertTrue(any("coverage" in p and "llvm-cov" in p for p in found), found)

    def test_cargo_llvm_cov_nextest_and_report_are_allowed(self):
        cov = job("scripts/nextest.sh check\ncargo llvm-cov nextest --workspace --locked --lcov --output-path lcov.info\ncargo llvm-cov report --summary-only")
        self.assertEqual(problems(coverage=cov), [])

    def test_a_comment_naming_cargo_test_is_not_a_run(self):
        self.assertEqual(problems(hook="# cargo test has no deadline\nscripts/nextest.sh run --workspace\ncargo test --workspace --doc\n"), [])


class Nextest(unittest.TestCase):
    def test_nextest_called_directly_is_refused(self):
        found = problems(rust=job("scripts/nextest.sh run --workspace\ncargo test --workspace --doc\ncargo nextest run --lib"))
        self.assertEqual(found, ["ci.yml job rust: cargo nextest called directly; use scripts/nextest.sh (pinned version, the repository's config): cargo nextest run --lib"])

    def test_a_job_that_runs_nextest_without_installing_it_is_refused(self):
        found = problems(rust=job("scripts/nextest.sh run --workspace", install=False))
        self.assertTrue(any("rust" in p and "install" in p for p in found), found)

    def test_a_job_that_installs_through_the_nextest_action_passes(self):
        steps = [{"uses": "./.github/actions/nextest"}, {"run": "scripts/nextest.sh run --workspace --locked\ncargo test --workspace --locked --doc"}]
        self.assertEqual(problems(rust={"steps": steps}), [])

    def test_the_nextest_action_after_the_run_does_not_count(self):
        steps = [{"run": "scripts/nextest.sh run --workspace --locked\ncargo test --workspace --locked --doc"}, {"uses": "./.github/actions/nextest"}]
        self.assertTrue(any("install" in p for p in problems(rust={"steps": steps})))

    def test_a_nextest_action_that_never_installs_is_refused(self):
        text = "runs:\n  using: composite\n  steps:\n    - shell: bash\n      run: bash scripts/nextest.sh pin\n"
        self.assertEqual(check.nextest_action_problems(text), [".github/actions/nextest/action.yml: no step runs `scripts/nextest.sh install`"])

    def test_the_real_nextest_action_installs(self):
        text = (HERE.parent / ".github/actions/nextest/action.yml").read_text(encoding="utf-8")
        self.assertEqual(check.nextest_action_problems(text), [])

    def test_a_tree_that_uses_a_missing_nextest_action_is_refused(self):
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            (root / ".github/workflows").mkdir(parents=True)
            (root / ".config").mkdir()
            (root / ".githooks").mkdir()
            (root / ".config/nextest.toml").write_text(CONFIG)
            (root / ".githooks/pre-commit").write_text("scripts/nextest.sh run --workspace --locked\ncargo test --workspace --doc --locked\n")
            (root / ".github/workflows/ci.yml").write_text("jobs:\n  t:\n    steps:\n      - uses: ./.github/actions/nextest\n      - run: scripts/nextest.sh run --lib\n")
            found = check.repository_problems(root)
        self.assertIn(".github/actions/nextest/action.yml is missing, and a job uses it", found)
        self.assertIn("crates/vcon-server/Cargo.toml is missing", found, "the manifest the feature rule reads")

    def test_llvm_cov_nextest_needs_the_version_check(self):
        found = problems(coverage=job("cargo llvm-cov nextest --workspace"))
        self.assertTrue(any("coverage" in p and "check" in p for p in found), found)

    def test_the_hook_must_run_the_workspace_under_nextest(self):
        found = problems(hook="cargo test --doc\n")
        self.assertTrue(any("pre-commit" in p and "nextest" in p for p in found), found)

    def test_the_hook_must_run_the_doc_tests(self):
        found = problems(hook="scripts/nextest.sh run --workspace\n")
        self.assertTrue(any("pre-commit" in p and "--doc" in p for p in found), found)

    def test_the_rust_job_must_run_the_doc_tests(self):
        found = problems(rust=job("scripts/nextest.sh run --workspace"))
        self.assertTrue(any("rust" in p and "--doc" in p for p in found), found)


CARGO = """[package]
name = "vcon-server"

[features]
default = ["s3", "tui"]
s3 = ["dep:aws-config"]
backend-tests = []
postgres = ["s3", "dep:tokio-postgres"]
tui = ["dep:ratatui"]
"""
HOOK = "scripts/nextest.sh run --workspace --locked\ncargo test --workspace --doc --locked\n"
PG = "scripts/nextest.sh run -p vcon-server --locked --features postgres\n"


def feature_problems(hook=HOOK + PG, cargo=CARGO):
    return problems(hook=hook, cargo_toml=cargo)


def runs(line, feature="postgres"):
    return check.runs_feature(line, feature)


class FeatureTests(unittest.TestCase):
    """The commit hook runs the vcon-server package's tests with each cargo
    feature that is not on by default (backend-tests aside, which needs
    live services): on 2026-11-16 a postgres-only test that reads a page
    failed in CI only, after the hook had passed the commit."""

    # Success.
    def test_a_hook_that_runs_the_package_with_postgres_passes(self):
        self.assertEqual(feature_problems(), [], "the hook runs it")

    def test_the_workspace_counts_as_the_package(self):
        self.assertTrue(runs("scripts/nextest.sh run --workspace --locked --features postgres"), "every package")

    def test_a_feature_list_counts(self):
        self.assertTrue(runs("scripts/nextest.sh run -p vcon-server --features s3,postgres"), "comma list")

    def test_the_equals_form_counts(self):
        self.assertTrue(runs("scripts/nextest.sh run -p vcon-server --features=postgres"), "--features=")

    def test_the_long_package_flag_counts(self):
        self.assertTrue(runs("scripts/nextest.sh run --package vcon-server --features postgres"), "--package")

    def test_no_fail_fast_counts(self):
        self.assertTrue(runs("scripts/nextest.sh run -p vcon-server --features postgres --no-fail-fast"), "a run option")

    def test_the_hooks_quiet_options_count(self):
        line = 'scripts/nextest.sh run -p vcon-server --locked --features postgres "${nextest_quiet[@]}"'
        self.assertTrue(runs(line), "the hook's array of output options")

    def test_the_status_options_count(self):
        line = "scripts/nextest.sh run -p vcon-server --features postgres --cargo-quiet --status-level slow --final-status-level slow"
        self.assertTrue(runs(line), "output options only")

    def test_a_continued_line_counts(self):
        hook = HOOK + "scripts/nextest.sh run -p vcon-server \\\n  --features postgres\n"
        self.assertEqual(feature_problems(hook=hook), [], "joined")

    def test_two_features_on_one_line_count_for_both(self):
        cargo = CARGO + 'mcp = []\n'
        hook = HOOK + "scripts/nextest.sh run -p vcon-server --features postgres,mcp\n"
        self.assertEqual(feature_problems(hook=hook, cargo=cargo), [], "one run, both features")

    def test_two_features_on_two_lines_count(self):
        cargo = CARGO + 'mcp = []\n'
        hook = HOOK + PG + "scripts/nextest.sh run -p vcon-server --features mcp\n"
        self.assertEqual(feature_problems(hook=hook, cargo=cargo), [], "a run each")

    def test_the_order_of_the_options_does_not_matter(self):
        self.assertTrue(runs("scripts/nextest.sh run --features postgres --locked -p vcon-server"), "any order")

    def test_the_real_manifest_asks_for_postgres(self):
        text = (HERE.parent / "crates/vcon-server/Cargo.toml").read_text(encoding="utf-8")
        self.assertEqual(check.extra_features(text), ["postgres"], "the one feature off by default")

    def test_the_features_come_sorted(self):
        self.assertEqual(check.extra_features('[features]\nzeta = []\nalpha = []\n'), ["alpha", "zeta"], "sorted")

    def test_the_default_features_beside_it_count(self):
        self.assertTrue(runs("scripts/nextest.sh run -p vcon-server --features s3,tui,postgres"), "a superset")

    def test_the_real_hook_runs_it(self):
        text = (HERE.parent / ".githooks/pre-commit").read_text(encoding="utf-8")
        self.assertTrue(any(runs(l) for l in check.commands(text)), "a postgres run of the package")

    # Failure.
    def test_a_hook_without_the_run_is_refused_and_told_the_command(self):
        self.assertEqual(
            feature_problems(hook=HOOK),
            [".githooks/pre-commit: does not run the tests the postgres feature compiles: add "
             "`scripts/nextest.sh run -p vcon-server --locked --features postgres` (none needs a database "
             "without backend-tests)"],
            "named, with the command",
        )

    def test_only_the_lib_is_refused(self):
        self.assertFalse(runs("scripts/nextest.sh run -p vcon-server --features postgres --lib"), "not every target")

    def test_one_test_target_is_refused(self):
        self.assertFalse(runs("scripts/nextest.sh run -p vcon-server --features postgres --test serve"), "one target")

    def test_the_binaries_only_are_refused(self):
        self.assertFalse(runs("scripts/nextest.sh run -p vcon-server --features postgres --bins"), "--bins")

    def test_a_name_filter_is_refused(self):
        self.assertFalse(runs('scripts/nextest.sh run -p vcon-server --features postgres "backend::postgres::"'), "a filter")

    def test_a_filterset_is_refused(self):
        self.assertFalse(runs("scripts/nextest.sh run -p vcon-server --features postgres -E 'test(v9)'"), "-E")

    def test_backend_tests_beside_it_is_refused(self):
        self.assertFalse(runs("scripts/nextest.sh run -p vcon-server --features postgres,backend-tests"), "needs services")

    def test_another_package_is_refused(self):
        self.assertFalse(runs("scripts/nextest.sh run -p vcon --features postgres"), "not the server")

    def test_a_build_is_not_a_test_run(self):
        self.assertFalse(runs("cargo build -p vcon-server --features postgres"), "builds, tests nothing")

    def test_a_listing_is_not_a_run(self):
        self.assertFalse(runs("scripts/nextest.sh list -p vcon-server --features postgres"), "lists, runs nothing")

    def test_a_commented_out_run_is_refused(self):
        hook = HOOK + "# scripts/nextest.sh run -p vcon-server --features postgres\n"
        self.assertEqual(len(feature_problems(hook=hook)), 1, "a comment runs nothing")

    def test_a_similar_feature_name_is_refused(self):
        self.assertFalse(runs("scripts/nextest.sh run -p vcon-server --features postgresql"), "another name")

    def test_a_manifest_that_is_not_toml_is_refused(self):
        found = feature_problems(cargo="[features\n")
        self.assertTrue(any("crates/vcon-server/Cargo.toml is not TOML" in p for p in found), found)

    def test_a_run_in_a_workflow_does_not_count_for_the_hook(self):
        workflows = {"ci.yml": {"jobs": {"rust": job("scripts/nextest.sh run --workspace --locked\ncargo test --workspace --locked --doc\n" + PG)}}}
        found = check.problems(CONFIG, HOOK, workflows, CARGO)
        self.assertEqual(len(found), 1, f"CI runs it 10 to 30 minutes later: {found}")

    def test_a_partition_is_refused(self):
        self.assertFalse(runs("scripts/nextest.sh run -p vcon-server --features postgres --partition count:1/2"), "half")

    def test_a_skip_after_the_separator_is_refused(self):
        self.assertFalse(runs("scripts/nextest.sh run -p vcon-server --features postgres -- --skip v9"), "skips")

    def test_the_workspace_without_the_server_is_refused(self):
        self.assertFalse(runs("scripts/nextest.sh run --workspace --exclude vcon-server --features postgres"), "excluded")

    def test_of_two_features_the_missing_one_is_named(self):
        found = feature_problems(cargo=CARGO + "mcp = []\n")
        self.assertEqual(len(found), 1, found)
        self.assertIn("the mcp feature compiles", found[0], found)

    # Edge cases.
    def test_no_default_features_still_counts(self):
        self.assertTrue(runs("scripts/nextest.sh run -p vcon-server --no-default-features --features postgres"), "its tests compile")

    def test_an_empty_feature_list_counts_for_nothing(self):
        self.assertFalse(runs("scripts/nextest.sh run -p vcon-server --features ''"), "no feature")

    def test_a_manifest_that_is_not_toml_is_one_problem(self):
        found = [p for p in feature_problems(cargo="[features\n") if "Cargo.toml" in p]
        self.assertEqual(len(found), 1, found)

    def test_backend_tests_is_never_asked_of_the_hook(self):
        self.assertNotIn("backend-tests", check.extra_features(CARGO), "it needs live services")

    def test_default_features_are_not_asked(self):
        self.assertEqual(check.extra_features(CARGO), ["postgres"], "s3 and tui run in the workspace run")

    def test_a_manifest_without_features_asks_nothing(self):
        self.assertEqual(check.extra_features('[package]\nname = "x"\n'), [], "no feature")

    def test_a_manifest_without_a_default_asks_every_feature(self):
        self.assertEqual(check.extra_features('[features]\na = []\nb = []\nbackend-tests = []\n'), ["a", "b"], "sorted")

    def test_a_feature_named_only_in_default_is_still_default(self):
        cargo = '[features]\ndefault = ["s3", "dep:x"]\ns3 = []\nextra = []\n'
        self.assertEqual(check.extra_features(cargo), ["extra"], "dep: entries are not features")

    def test_an_indented_run_counts(self):
        hook = HOOK + 'if true; then\n  scripts/nextest.sh run -p vcon-server --features postgres\nfi\n'
        self.assertEqual(feature_problems(hook=hook), [], "inside a block")

    def test_without_locked_it_still_counts(self):
        self.assertTrue(runs("scripts/nextest.sh run -p vcon-server --features postgres"), "--locked is cargo's")

    def test_the_features_flag_given_twice_counts(self):
        self.assertTrue(runs("scripts/nextest.sh run -p vcon-server --features s3 --features postgres"), "both read")

    def test_the_first_of_two_features_flags_is_kept(self):
        self.assertTrue(runs("scripts/nextest.sh run -p vcon-server --features postgres --features s3"), "both read")

    def test_backend_tests_in_a_second_flag_is_refused(self):
        self.assertFalse(runs("scripts/nextest.sh run -p vcon-server --features postgres --features backend-tests"), "services")

    def test_a_trailing_comment_does_not_hide_the_run(self):
        self.assertTrue(runs("scripts/nextest.sh run -p vcon-server --features postgres  # no database"), "comment")

    def test_a_quoted_feature_list_counts(self):
        self.assertTrue(runs('scripts/nextest.sh run -p vcon-server --features "s3,postgres"'), "quoted")

    def test_no_manifest_asks_nothing(self):
        self.assertEqual(problems(hook=HOOK), [], "the tests of other rules pass no manifest")

    def test_a_feature_list_ending_in_a_comma_counts(self):
        self.assertTrue(runs("scripts/nextest.sh run -p vcon-server --features postgres,"), "cargo accepts it")

    def test_a_package_with_a_version_suffix_is_another_spec(self):
        self.assertFalse(runs("scripts/nextest.sh run -p vcon-server:0.4.7 --features postgres"), "kept exact")


class Repository(unittest.TestCase):
    def test_this_repository_keeps_the_rule(self):
        root = HERE.parent
        self.assertEqual(check.repository_problems(root), [])


if __name__ == "__main__":
    unittest.main()
