"""Tests scripts/check_release_build.py: the commit hook builds the libraries
and binaries with no test target, so a feature that only a dev-dependency
turns on is not unified into the build, and code that needs it is refused
before the commit, as the release build refuses it.

The Build cases run cargo on a workspace of their own (two crates, path
dependencies only, no network) shaped like the case of 2026-11-25:
crates/vcon-server/src/store/search.rs called `time::macros::datetime!` in
non-test code, `time`'s `macros` feature was on only under
[dev-dependencies], and every gate that builds test targets passed.

Run with: python3 -I scripts/test_check_release_build.py
"""

import importlib.util
import os
import pathlib
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
spec = importlib.util.spec_from_file_location("check_release_build", HERE / "check_release_build.py")
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

SERVER = """[package]
name = "server-fixture"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
leaf = { path = "../leaf" }

[dev-dependencies]
leaf = { path = "../leaf", features = ["extra"] }

[features]
default = ["s3"]
s3 = []
postgres = ["s3"]
backend-tests = []
"""

LEAF = """[package]
name = "leaf"
version = "0.1.0"
edition = "2024"
publish = false

[features]
extra = []
"""

LEAF2 = '[package]\nname = "leaf2"\nversion = "0.1.0"\nedition = "2024"\npublish = false\n'
WORKSPACE = '[workspace]\nresolver = "3"\nmembers = ["crates/vcon-server", "crates/leaf", "crates/leaf2"]\n'

LEAF_LIB = """/// Always there.
pub fn base() -> u32 {
    0
}

/// There only with the `extra` feature, which only the server's
/// dev-dependency turns on.
#[cfg(feature = "extra")]
pub fn extra() -> u32 {
    1
}
"""

CLEAN_LIB = "pub fn value() -> u32 {\n    leaf::base()\n}\n"


def manifest(features='default = ["s3"]\ns3 = []\npostgres = ["s3"]\nbackend-tests = []\n', name="vcon-server"):
    return f'[package]\nname = "{name}"\nversion = "0.1.0"\n\n[features]\n{features}'


def args(command):
    """The arguments of a command after `cargo check`."""
    assert command[:2] == ["cargo", "check"], command
    return command[2:]


class Commands(unittest.TestCase):
    def test_every_command_is_a_locked_check_of_libraries_and_binaries(self):
        for command in check.commands(manifest()):
            rest = args(command)
            self.assertIn("--locked", rest, f"{command} must build what Cargo.lock holds")
            self.assertIn("--lib", rest, f"{command} must build the libraries")
            self.assertIn("--bins", rest, f"{command} must build the binaries")

    def test_no_command_builds_a_test_target(self):
        test_targets = {"--all-targets", "--tests", "--test", "--benches", "--bench", "--examples", "--example"}
        for command in check.commands(manifest()):
            self.assertFalse(test_targets & set(command), f"{command} builds a target that turns on dev-dependency features")

    def test_the_first_command_checks_the_whole_workspace_with_default_features(self):
        first = args(check.commands(manifest())[0])
        self.assertIn("--workspace", first, f"the first command must cover every crate: {first}")
        self.assertNotIn("--features", first, f"the workspace command keeps the default features: {first}")
        self.assertNotIn("--no-default-features", first, f"the workspace command keeps the default features: {first}")

    def test_the_server_is_checked_without_default_features(self):
        found = [c for c in check.commands(manifest()) if "--no-default-features" in c]
        self.assertEqual(len(found), 1, f"one command without default features: {found}")
        self.assertEqual(args(found[0])[args(found[0]).index("-p") + 1], "vcon-server", f"it names the server package: {found}")

    def test_each_feature_off_by_default_is_checked(self):
        found = [args(c) for c in check.commands(manifest()) if "--features" in c]
        self.assertEqual([a[a.index("--features") + 1] for a in found], ["postgres"], f"the postgres feature, which the release binaries carry: {found}")

    def test_a_feature_that_needs_a_live_service_is_not_checked(self):
        joined = " ".join(" ".join(c) for c in check.commands(manifest()))
        self.assertNotIn("backend-tests", joined, "backend-tests compiles only test code, which no build without test targets reads")

    def test_a_default_feature_is_not_checked_on_its_own(self):
        joined = [" ".join(c) for c in check.commands(manifest())]
        self.assertFalse(any("--features s3" in c for c in joined), f"s3 is on by default, so the workspace command builds it: {joined}")

    def test_every_feature_off_by_default_gets_its_own_command(self):
        text = manifest('default = []\nalpha = []\nbeta = ["alpha"]\n')
        found = [args(c) for c in check.commands(text) if "--features" in c]
        self.assertEqual([a[a.index("--features") + 1] for a in found], ["alpha", "beta"], f"one command per feature, sorted: {found}")

    def test_a_manifest_with_no_feature_has_two_commands(self):
        commands = check.commands('[package]\nname = "vcon-server"\nversion = "0.1.0"\n')
        self.assertEqual(len(commands), 2, f"the workspace and the server without default features: {commands}")

    def test_the_package_name_is_read_from_the_manifest(self):
        commands = check.commands(manifest(name="hook-fixture"))
        self.assertTrue(all("vcon-server" not in c for c in commands), f"the hook tests' stand-in crate has another name: {commands}")
        self.assertTrue(any("hook-fixture" in c for c in commands), f"the stand-in crate is named: {commands}")

    def test_a_manifest_without_a_package_name_is_refused(self):
        with self.assertRaises(ValueError, msg="no [package] name"):
            check.commands("[features]\npostgres = []\n")

    def test_a_manifest_that_is_not_toml_is_refused(self):
        with self.assertRaises(ValueError, msg="not TOML"):
            check.commands("[package\nname = 1\n")

    def test_the_command_is_quoted_for_a_shell(self):
        self.assertEqual(check.shown(["cargo", "check", "--features", "a b"]), "cargo check --features 'a b'", "a shown command can be pasted into a shell")


class HookProblems(unittest.TestCase):
    EXPORT = 'export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}/pre-commit"\n'
    STEP = "python3 -I scripts/check_release_build.py\n"

    def test_a_hook_that_runs_the_step_after_the_target_directory_is_set_passes(self):
        self.assertEqual(check.hook_problems(self.EXPORT + self.STEP), [], "the step builds in the hook's own target directory")

    def test_a_hook_without_the_step_is_refused(self):
        found = check.hook_problems(self.EXPORT + "cargo clippy --workspace --all-targets\n")
        self.assertTrue(any("check_release_build.py" in p for p in found), f"the missing step is named: {found}")

    def test_a_commented_out_step_is_refused(self):
        found = check.hook_problems(self.EXPORT + "# " + self.STEP)
        self.assertTrue(found, "a comment runs nothing")

    def test_a_step_before_the_target_directory_is_set_is_refused(self):
        found = check.hook_problems(self.STEP + self.EXPORT)
        self.assertTrue(any("CARGO_TARGET_DIR" in p for p in found), f"before the export it builds in the deleted copy every run: {found}")

    def test_a_hook_without_the_target_directory_is_refused(self):
        found = check.hook_problems(self.STEP)
        self.assertTrue(any("CARGO_TARGET_DIR" in p for p in found), f"no target directory: {found}")

    def test_a_step_that_can_fail_without_stopping_the_hook_is_refused(self):
        for line in ("python3 -I scripts/check_release_build.py || true\n", "python3 -I scripts/check_release_build.py; true\n"):
            found = check.hook_problems(self.EXPORT + line)
            self.assertTrue(found, f"{line!r} lets a refused build through")

    def test_a_step_in_a_condition_is_refused(self):
        found = check.hook_problems(self.EXPORT + "if false; then\n  python3 -I scripts/check_release_build.py\nfi\n")
        self.assertTrue(found, "an indented step is inside a condition or a function, which may not run")

    def test_the_repository_hook_runs_the_step(self):
        text = (ROOT / ".githooks/pre-commit").read_text(encoding="utf-8")
        self.assertEqual(check.hook_problems(text), [], "the commit hook builds the libraries and binaries without test targets")

    def test_the_repository_server_manifest_gives_the_release_feature(self):
        text = (ROOT / "crates/vcon-server/Cargo.toml").read_text(encoding="utf-8")
        joined = [" ".join(c) for c in check.commands(text)]
        self.assertTrue(any("--features postgres" in c for c in joined), f"the release binaries are built with postgres (scripts/reproducible-build.sh): {joined}")


class Run(unittest.TestCase):
    def test_every_command_runs_and_a_pass_is_reported(self):
        seen = []
        code, lines = check.run([["a"], ["b"]], ROOT, runner=lambda c, cwd, deadline: seen.append(c) or 0)
        self.assertEqual((code, seen), (0, [["a"], ["b"]]), "both commands run and the run passes")
        self.assertTrue(any("ok" in line and "2" in line for line in lines), f"the pass names the count: {lines}")

    def test_a_failed_command_fails_the_run_and_is_named(self):
        code, lines = check.run([["a"], ["b"]], ROOT, runner=lambda c, cwd, deadline: 101 if c == ["b"] else 0)
        self.assertEqual(code, 1, "a refused build refuses the commit")
        self.assertTrue(any("`b`" in line and "dev-dependenc" in line for line in lines), f"the refused command and the likely cause are named: {lines}")

    def test_every_command_runs_after_a_failure(self):
        seen = []
        code, lines = check.run([["a"], ["b"]], ROOT, runner=lambda c, cwd, deadline: seen.append(c) or 1)
        self.assertEqual((code, seen), (1, [["a"], ["b"]]), "each feature set is reported in one run")
        self.assertEqual(sum("`a`" in l or "`b`" in l for l in lines), 2, f"both are named: {lines}")

    def test_a_command_past_its_deadline_fails_the_run(self):
        code, lines = check.run([["a"]], ROOT, runner=lambda c, cwd, deadline: None)
        self.assertEqual(code, 1, "a build that did not end is no pass")
        self.assertTrue(any("deadline" in line for line in lines), f"the deadline is named: {lines}")

    def test_no_command_is_no_pass(self):
        code, lines = check.run([], ROOT, runner=lambda c, cwd, deadline: 0)
        self.assertEqual(code, 1, "a run that built nothing proves nothing")

    def test_the_runner_gets_the_root_and_the_deadline(self):
        got = []
        check.run([["a"]], ROOT, runner=lambda c, cwd, deadline: got.append((cwd, deadline)) or 0, deadline=7)
        self.assertEqual(got, [(ROOT, 7)], "the commands run in the tree they check, with the deadline given")


# A process that runs until a signal ends it, and that nothing of its
# parent's ending (a closed pipe, a hang-up of its group) reaches.
GRANDCHILD = "[sys.executable, '-I', '-S', '-c', 'import signal; signal.signal(signal.SIGHUP, signal.SIG_IGN); signal.pause()']"


class Process(unittest.TestCase):
    """check.execute, which runs one command in a process group of its own
    and ends the whole group at the deadline."""

    def test_an_exit_status_is_returned(self):
        self.assertEqual(check.execute([sys.executable, "-I", "-S", "-c", "raise SystemExit(3)"], ROOT, 30), 3, "the command's own status")

    def test_a_pass_is_zero(self):
        self.assertEqual(check.execute([sys.executable, "-I", "-S", "-c", "pass"], ROOT, 30), 0, "a pass")

    def test_the_command_runs_in_the_directory_given(self):
        with tempfile.TemporaryDirectory() as tmp:
            probe = f"import os, sys; sys.exit(0 if os.getcwd() == {os.path.realpath(tmp)!r} else 9)"
            self.assertEqual(check.execute([sys.executable, "-I", "-S", "-c", probe], pathlib.Path(tmp), 30), 0, "cwd is the tree")

    def test_a_missing_program_is_a_failure_not_a_crash(self):
        status = check.execute(["/nonexistent/cargo-of-no-one"], ROOT, 30)
        self.assertNotIn(status, (0, None), "a program that cannot start fails the step")

    def test_the_whole_group_ends_at_the_deadline(self):
        with tempfile.TemporaryDirectory() as tmp:
            pid_file = pathlib.Path(tmp) / "grandchild"
            # The child starts a grandchild that would outlive it (it waits
            # for a signal, and nothing of the child's ending reaches it),
            # records its pid and waits on it: the deadline must end both.
            script = (
                "import subprocess, sys, pathlib\n"
                f"g = subprocess.Popen({GRANDCHILD}, stdin=subprocess.DEVNULL)\n"
                f"pathlib.Path({str(pid_file)!r}).write_text(str(g.pid))\n"
                "g.wait()\n"
            )
            started = time.monotonic()
            status = check.execute([sys.executable, "-I", "-S", "-c", script], pathlib.Path(tmp), 3)
            self.assertIsNone(status, "past the deadline there is no status")
            self.assertLess(time.monotonic() - started, 30, "the deadline ends the wait")
            pid = int(pid_file.read_text())
            for _ in range(100):
                try:
                    os.kill(pid, 0)
                except ProcessLookupError:
                    break
                time.sleep(0.05)
            else:
                os.kill(pid, signal.SIGKILL)
                self.fail(f"grandchild {pid} outlived the deadline")

    def ended_by(self, number):
        """Runs execute in a process of its own, sends it `number` while
        its command's grandchild runs, and returns (the process's status,
        whether the grandchild is gone)."""
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            pid_file = tmp / "grandchild"
            grandchild = (
                "import subprocess, sys, pathlib\n"
                f"g = subprocess.Popen({GRANDCHILD}, stdin=subprocess.DEVNULL)\n"
                f"pathlib.Path({str(pid_file)!r}).write_text(str(g.pid))\n"
                "g.wait()\n"
            )
            driver = tmp / "driver.py"
            driver.write_text(
                "import importlib.util, sys\n"
                "sys.dont_write_bytecode = True\n"
                f"spec = importlib.util.spec_from_file_location('c', {str(HERE / 'check_release_build.py')!r})\n"
                "c = importlib.util.module_from_spec(spec)\n"
                "spec.loader.exec_module(c)\n"
                f"c.execute([sys.executable, '-I', '-S', '-c', {grandchild!r}], '.', 60)\n",
                encoding="utf-8",
            )
            proc = subprocess.Popen([sys.executable, "-I", str(driver)], cwd=tmp, start_new_session=True)
            try:
                for _ in range(400):
                    if pid_file.exists() and pid_file.read_text():
                        break
                    time.sleep(0.05)
                else:
                    self.fail("the grandchild never started")
                pid = int(pid_file.read_text())
                proc.send_signal(number)
                status = proc.wait(timeout=30)
            finally:
                if proc.poll() is None:
                    os.killpg(proc.pid, signal.SIGKILL)
                    proc.wait()
            gone = False
            for _ in range(100):
                try:
                    os.kill(pid, 0)
                except ProcessLookupError:
                    gone = True
                    break
                time.sleep(0.05)
            if not gone:
                os.kill(pid, signal.SIGKILL)
            return status, gone

    def test_a_sigterm_to_the_check_ends_the_command_it_runs(self):
        status, gone = self.ended_by(signal.SIGTERM)
        self.assertTrue(gone, "heavy-run's or timeout's SIGTERM reaches the build, which runs in a session of its own")
        self.assertNotEqual(status, 0, "a check ended by a signal is no pass")

    def test_a_hangup_to_the_check_ends_the_command_it_runs(self):
        status, gone = self.ended_by(signal.SIGHUP)
        self.assertTrue(gone, "a closed terminal ends the build too")
        self.assertNotEqual(status, 0, "a check ended by a signal is no pass")

    def test_an_interrupt_to_the_check_ends_the_command_it_runs(self):
        status, gone = self.ended_by(signal.SIGINT)
        self.assertTrue(gone, "Ctrl-C ends the build too")
        self.assertNotEqual(status, 0, "a check ended by a signal is no pass")

    def test_the_signal_handlers_are_put_back_after_the_command(self):
        before = signal.getsignal(signal.SIGTERM)
        check.execute([sys.executable, "-I", "-S", "-c", "pass"], ROOT, 30)
        self.assertIs(signal.getsignal(signal.SIGTERM), before, "the caller's SIGTERM handler is its own again")


def cargo_available():
    return shutil.which("cargo") is not None


@unittest.skipUnless(cargo_available(), "cargo is not installed")
class Build(unittest.TestCase):
    """The real commands on a workspace of their own."""

    @classmethod
    def setUpClass(cls):
        cls.target = tempfile.TemporaryDirectory()

    @classmethod
    def tearDownClass(cls):
        cls.target.cleanup()

    def env(self):
        return dict(os.environ, CARGO_TARGET_DIR=self.target.name, CARGO_NET_OFFLINE="true")

    def workspace(self, lib=CLEAN_LIB, main=None, extra_files=None, server=SERVER, leaf_lib=LEAF_LIB):
        """Writes the workspace and its lock file; returns its root."""
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = pathlib.Path(tmp.name)
        shutil.copy(ROOT / "rust-toolchain.toml", root / "rust-toolchain.toml")
        files = {
            "Cargo.toml": WORKSPACE,
            "crates/vcon-server/Cargo.toml": server,
            "crates/vcon-server/src/lib.rs": lib,
            "crates/leaf/Cargo.toml": LEAF,
            "crates/leaf/src/lib.rs": leaf_lib,
            "crates/leaf2/Cargo.toml": LEAF2,
            "crates/leaf2/src/lib.rs": "pub fn two() -> u32 {\n    2\n}\n",
        }
        if main is not None:
            files["crates/vcon-server/src/main.rs"] = main
        files.update(extra_files or {})
        for name, text in files.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
        lock = subprocess.run(["cargo", "generate-lockfile", "--offline", "--quiet"], cwd=root, env=self.env(), capture_output=True, text=True, timeout=120, check=False)
        self.assertEqual(lock.returncode, 0, f"the fixture's lock file: {lock.stderr}")
        return root

    def build(self, **parts):
        """Runs the hook's commands on a workspace; returns (status, the
        lines printed)."""
        root = self.workspace(**parts)
        env = self.env()
        saved = os.environ.copy()
        os.environ.update(env)
        try:
            commands = check.commands((root / "crates/vcon-server/Cargo.toml").read_text(encoding="utf-8"))
            return check.run([c + ["--quiet"] for c in commands], root, deadline=300)
        finally:
            os.environ.clear()
            os.environ.update(saved)

    def refused(self, outcome, why):
        code, lines = outcome
        self.assertEqual(code, 1, f"{why}: {lines}")

    def passed(self, outcome, why):
        code, lines = outcome
        self.assertEqual(code, 0, f"{why}: {lines}")

    # The case the hook exists for, and the gates that missed it.

    def test_a_library_that_uses_a_dev_dependency_feature_is_refused(self):
        self.refused(self.build(lib="pub fn value() -> u32 {\n    leaf::extra()\n}\n"), "leaf::extra exists only with the dev-dependency's feature")

    def test_the_refusal_names_the_command_that_failed(self):
        code, lines = self.build(lib="pub fn value() -> u32 {\n    leaf::extra()\n}\n")
        self.assertTrue(any("cargo check" in line and "--workspace" in line for line in lines), f"the workspace command is named: {lines}")

    def test_a_test_target_build_accepts_what_the_hook_refuses(self):
        """The defect: cargo unifies dev-dependency features into a build of
        test targets, so a test or clippy --all-targets run passes."""
        lib = "pub fn value() -> u32 {\n    leaf::extra()\n}\n"
        self.refused(self.build(lib=lib), "the hook's commands refuse it")
        root = self.workspace(lib=lib)
        done = subprocess.run(["cargo", "check", "--quiet", "--locked", "--workspace", "--all-targets"], cwd=root, env=self.env(), capture_output=True, text=True, timeout=300, check=False)
        self.assertEqual(done.returncode, 0, f"with test targets cargo turns the feature on, which is why the other gates passed: {done.stderr}")

    def test_a_binary_that_uses_a_dev_dependency_feature_is_refused(self):
        self.refused(self.build(main="fn main() {\n    println!(\"{}\", leaf::extra());\n}\n"), "the server binary is built too")

    def test_code_only_the_release_feature_compiles_is_refused(self):
        lib = CLEAN_LIB + '\n#[cfg(feature = "postgres")]\npub fn more() -> u32 {\n    leaf::extra()\n}\n'
        self.refused(self.build(lib=lib), "the postgres build, which the release binaries are")

    def test_code_only_a_build_without_default_features_compiles_is_refused(self):
        lib = CLEAN_LIB + '\n#[cfg(not(feature = "s3"))]\npub fn more() -> u32 {\n    leaf::extra()\n}\n'
        self.refused(self.build(lib=lib), "the build without default features")

    def test_another_workspace_crate_that_uses_a_dev_dependency_feature_is_refused(self):
        leaf = LEAF_LIB + "\n#[cfg(feature = \"extra\")]\npub fn other() -> u32 {\n    extra()\n}\n"
        # leaf2 dev-depends on leaf with `extra`, and leaf2's library uses it.
        extra = {
            "crates/leaf2/Cargo.toml": LEAF2 + '\n[dependencies]\nleaf = { path = "../leaf" }\n\n[dev-dependencies]\nleaf = { path = "../leaf", features = ["extra"] }\n',
            "crates/leaf2/src/lib.rs": "pub fn two() -> u32 {\n    leaf::other()\n}\n",
        }
        self.refused(self.build(leaf_lib=leaf, extra_files=extra), "the workspace command builds every crate")

    def test_a_compile_error_of_any_kind_is_refused(self):
        self.refused(self.build(lib="pub fn value() -> u32 {\n    \"no number\"\n}\n"), "any build failure fails the step")

    # What the hook accepts.

    def test_a_clean_workspace_passes(self):
        self.passed(self.build(), "no dev-dependency feature is used outside test code")

    def test_a_clean_workspace_with_a_binary_passes(self):
        self.passed(self.build(main="fn main() {\n    println!(\"{}\", server_fixture::value());\n}\n"), "the binary uses the library")

    def test_a_dev_dependency_feature_in_a_unit_test_passes(self):
        lib = CLEAN_LIB + "\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn extra_is_one() {\n        assert_eq!(leaf::extra(), 1, \"one\");\n    }\n}\n"
        self.passed(self.build(lib=lib), "test code may use what dev-dependencies turn on")

    def test_a_dev_dependency_feature_in_an_integration_test_passes(self):
        extra = {"crates/vcon-server/tests/extra.rs": "#[test]\nfn extra_is_one() {\n    assert_eq!(leaf::extra(), 1, \"one\");\n}\n"}
        self.passed(self.build(extra_files=extra), "an integration test is a test target")

    def test_a_dev_dependency_feature_in_an_example_passes(self):
        extra = {"crates/vcon-server/examples/extra.rs": "fn main() {\n    println!(\"{}\", leaf::extra());\n}\n"}
        self.passed(self.build(extra_files=extra), "examples build with dev-dependencies, and no release ships them")

    def test_a_feature_a_normal_dependency_turns_on_passes(self):
        server = SERVER.replace('[dependencies]\nleaf = { path = "../leaf" }', '[dependencies]\nleaf = { path = "../leaf", features = ["extra"] }')
        self.passed(self.build(lib="pub fn value() -> u32 {\n    leaf::extra()\n}\n", server=server), "a feature of [dependencies] is in every build")

    def test_a_feature_the_server_turns_on_for_its_dependency_passes(self):
        server = SERVER.replace('backend-tests = []', 'backend-tests = []\nleafextra = ["leaf/extra"]').replace('default = ["s3"]', 'default = ["s3", "leafextra"]')
        lib = CLEAN_LIB + '\n#[cfg(feature = "leafextra")]\npub fn more() -> u32 {\n    leaf::extra()\n}\n'
        self.passed(self.build(lib=lib, server=server), "a feature of the package that turns the dependency's on is honest")


if __name__ == "__main__":
    unittest.main()
