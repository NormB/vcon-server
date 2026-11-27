"""Tests deny.toml, CI's `deny` job and scripts/deny_yanked_probe.py: a
yanked crate version fails cargo-deny's advisories check, a GPL-family
license is never on the allow list, and an advisory ignored for cargo-deny
is one OSV-Scanner ignores too.

No test here runs cargo-deny or reaches crates.io: the commit hook has no
network. The probe's conversions (the crate it writes, the command, the
verdict on cargo-deny's answer) are driven here; the probe itself runs in
CI's `deny` job, which has the network and the pinned cargo-deny.

Run with: python3 -I scripts/test_deny_config.py
"""

import importlib.util
import pathlib
import re
import sys
import tempfile
import tomllib
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
spec = importlib.util.spec_from_file_location("deny_yanked_probe", HERE / "deny_yanked_probe.py")
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)

GPL = re.compile(r"\b[AL]?GPL\b|\bGPL-", re.IGNORECASE)


def gpl_ids(allow):
    """The license ids of `allow` that are of the GPL family."""
    return [i for i in allow if GPL.search(i)]


def deny_step(ci_text):
    """The run text of CI's `deny` job, or None."""
    marker = "\n  deny:\n"
    start = ci_text.find(marker)
    if start < 0:
        return None
    rest = ci_text[start + len(marker):]
    end = re.search(r"\n  [A-Za-z0-9_-]+:\n", rest)
    return rest[: end.start()] if end else rest


def check_lines(job_text):
    """The `cargo-deny ... check ...` command lines of a job."""
    return [line.strip() for line in job_text.splitlines() if re.search(r"cargo-deny .*\bcheck\b", line)]


def ignored_ids(text):
    """The ids of an osv-scanner.toml's IgnoredVulns."""
    return [e.get("id") for e in tomllib.loads(text).get("IgnoredVulns", [])]


def deny_ignore_problems(deny, osv_ids):
    """What is wrong with deny.toml's advisories `ignore`, given the ids
    osv-scanner.toml ignores: each entry must be one of them."""
    problems = []
    for entry in deny.get("advisories", {}).get("ignore", []):
        advisory = entry if isinstance(entry, str) else entry.get("id")
        if advisory not in osv_ids:
            problems.append(f"{advisory} is ignored for cargo-deny and not in osv-scanner.toml")
    return problems


def advisories_problems(deny):
    """What is wrong with deny.toml's [advisories] table."""
    table = deny.get("advisories")
    if not isinstance(table, dict):
        return ["no [advisories] table: a yanked version is not refused"]
    problems = []
    if table.get("yanked") != "deny":
        problems.append(f"advisories.yanked is {table.get('yanked')!r}, not \"deny\"")
    if table.get("unmaintained") != "none":
        problems.append(f"advisories.unmaintained is {table.get('unmaintained')!r}: unmaintained crates are OSV's")
    return problems


def check_line_problems(lines):
    """What is wrong with the deny job's check lines: one for the workspace
    and one for the fuzz package, each running every check."""
    want = "check advisories licenses bans sources"
    problems = []
    fuzz = [line for line in lines if "fuzz/Cargo.toml" in line]
    workspace = [line for line in lines if "fuzz/Cargo.toml" not in line]
    for label, found in (("the workspace", workspace), ("the fuzz package", fuzz)):
        if not any(line.endswith(want) for line in found):
            problems.append(f"no `cargo-deny ... {want}` line for {label}")
    return problems


DENY = tomllib.loads((ROOT / "deny.toml").read_text(encoding="utf-8"))
CI = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")


class TheRepository(unittest.TestCase):
    def test_success_a_yanked_version_is_denied(self):
        self.assertEqual(advisories_problems(DENY), [], "deny.toml [advisories]")

    def test_success_no_gpl_license_is_allowed(self):
        self.assertEqual(gpl_ids(DENY["licenses"]["allow"]), [], "deny.toml licenses.allow")

    def test_success_the_deny_job_runs_the_advisories_check_on_both_lockfiles(self):
        job = deny_step(CI)
        self.assertIsNotNone(job, "ci.yml has a deny job")
        self.assertEqual(check_line_problems(check_lines(job)), [], "the deny job's check lines")

    def test_success_the_deny_job_runs_the_probe(self):
        self.assertIn("python3 -I scripts/deny_yanked_probe.py", deny_step(CI), "the deny job runs the probe")

    def test_success_every_cargo_deny_ignore_is_an_osv_ignore(self):
        osv = ignored_ids((ROOT / "osv-scanner.toml").read_text(encoding="utf-8"))
        self.assertEqual(deny_ignore_problems(DENY, osv), [], "deny.toml advisories.ignore")


class Gpl(unittest.TestCase):
    def test_failure_each_gpl_family_id_is_found(self):
        ids = ["GPL-2.0", "GPL-3.0-only", "GPL-3.0-or-later", "LGPL-2.1", "LGPL-3.0-or-later", "AGPL-3.0", "gpl-2.0"]
        self.assertEqual(gpl_ids(ids), ids, "every GPL-family id")

    def test_failure_a_gpl_id_with_an_exception_is_found(self):
        self.assertEqual(gpl_ids(["GPL-2.0 WITH Classpath-exception-2.0"]), ["GPL-2.0 WITH Classpath-exception-2.0"], "with an exception")

    def test_edge_permissive_ids_are_not_gpl(self):
        self.assertEqual(gpl_ids(["MIT", "Apache-2.0", "MPL-2.0", "Unlicense", "blessing", "0BSD", "CDLA-Permissive-2.0"]), [], "permissive")


class Advisories(unittest.TestCase):
    def test_failure_no_table(self):
        self.assertEqual(advisories_problems({}), ["no [advisories] table: a yanked version is not refused"], "no table")

    def test_failure_yanked_warn(self):
        self.assertEqual(
            advisories_problems({"advisories": {"yanked": "warn", "unmaintained": "none"}}),
            ['advisories.yanked is \'warn\', not "deny"'],
            "warn",
        )

    def test_failure_yanked_missing(self):
        self.assertEqual(
            advisories_problems({"advisories": {"unmaintained": "none"}}),
            ['advisories.yanked is None, not "deny"'],
            "missing",
        )

    def test_failure_unmaintained_reported_twice(self):
        self.assertEqual(
            advisories_problems({"advisories": {"yanked": "deny", "unmaintained": "all"}}),
            ["advisories.unmaintained is 'all': unmaintained crates are OSV's"],
            "all",
        )

    def test_edge_a_table_that_is_not_a_table(self):
        self.assertEqual(advisories_problems({"advisories": "deny"}), ["no [advisories] table: a yanked version is not refused"], "a string")


class Ignores(unittest.TestCase):
    def test_success_an_ignore_osv_also_has(self):
        deny = {"advisories": {"ignore": [{"id": "RUSTSEC-2026-0001", "reason": "x"}]}}
        self.assertEqual(deny_ignore_problems(deny, ["RUSTSEC-2026-0001"]), [], "in both")

    def test_failure_an_ignore_osv_lacks(self):
        deny = {"advisories": {"ignore": [{"id": "RUSTSEC-2026-0002", "reason": "x"}]}}
        self.assertEqual(
            deny_ignore_problems(deny, ["RUSTSEC-2026-0001"]),
            ["RUSTSEC-2026-0002 is ignored for cargo-deny and not in osv-scanner.toml"],
            "deny only",
        )

    def test_failure_a_bare_id_osv_lacks(self):
        deny = {"advisories": {"ignore": ["RUSTSEC-2026-0003"]}}
        self.assertEqual(
            deny_ignore_problems(deny, []),
            ["RUSTSEC-2026-0003 is ignored for cargo-deny and not in osv-scanner.toml"],
            "a bare id",
        )

    def test_edge_an_osv_ignore_needs_no_deny_entry(self):
        self.assertEqual(deny_ignore_problems({"advisories": {"ignore": []}}, ["RUSTSEC-2026-0543"]), [], "osv only")

    def test_edge_osv_ids_are_read(self):
        text = '[[IgnoredVulns]]\nid = "RUSTSEC-2026-0543"\nreason = "r"\n'
        self.assertEqual(ignored_ids(text), ["RUSTSEC-2026-0543"], "ids")


class CheckLines(unittest.TestCase):
    JOB = (
        "    steps:\n"
        "      - run: |\n"
        "          cargo-deny check advisories licenses bans sources\n"
        "          cargo-deny --manifest-path fuzz/Cargo.toml --config deny.toml check advisories licenses bans sources\n"
    )

    def test_success_both_lines(self):
        self.assertEqual(check_line_problems(check_lines(self.JOB)), [], "both")

    def test_failure_the_workspace_without_advisories(self):
        job = self.JOB.replace("cargo-deny check advisories licenses", "cargo-deny check licenses")
        self.assertEqual(
            check_line_problems(check_lines(job)),
            ["no `cargo-deny ... check advisories licenses bans sources` line for the workspace"],
            "workspace",
        )

    def test_failure_the_fuzz_package_without_advisories(self):
        job = self.JOB.replace("deny.toml check advisories licenses", "deny.toml check licenses")
        self.assertEqual(
            check_line_problems(check_lines(job)),
            ["no `cargo-deny ... check advisories licenses bans sources` line for the fuzz package"],
            "fuzz",
        )

    def test_failure_no_line(self):
        self.assertEqual(len(check_line_problems([])), 2, "neither")

    def test_edge_the_job_ends_at_the_next_job(self):
        ci = "jobs:\n  deny:\n    a: 1\n  site:\n    cargo-deny check advisories licenses bans sources\n"
        self.assertEqual(check_lines(deny_step(ci)), [], "the next job's lines are not the deny job's")

    def test_edge_no_deny_job(self):
        self.assertIsNone(deny_step("jobs:\n  site:\n    a: 1\n"), "no job")


class Probe(unittest.TestCase):
    def test_success_a_refusal_naming_the_crate_passes(self):
        out = "error[yanked]: detected yanked crate (try `cargo update -p autocfg`)\n  autocfg v0.1.3\n"
        self.assertIsNone(probe.verdict(1, out), "refused")

    def test_success_the_other_spelling_of_the_version_passes(self):
        self.assertIsNone(probe.verdict(1, "yanked: autocfg 0.1.3"), "name and version")

    def test_failure_an_accepted_version(self):
        self.assertEqual(
            probe.verdict(0, ""),
            "cargo-deny accepted autocfg 0.1.3, which crates.io lists as yanked",
            "accepted",
        )

    def test_failure_another_error(self):
        self.assertIn("without naming autocfg 0.1.3 as yanked", probe.verdict(1, "error: failed to fetch the advisory database"), "other error")

    def test_failure_a_yank_of_another_crate(self):
        self.assertIn("without naming", probe.verdict(1, "error[yanked]: detected yanked crate\n  serde v1.0.0\n"), "another crate")

    def test_edge_the_crate_is_a_workspace_of_its_own(self):
        with tempfile.TemporaryDirectory() as tmp:
            manifest = probe.write_crate(tmp)
            text = manifest.read_text(encoding="utf-8")
            lock = tomllib.loads((manifest.parent / "Cargo.lock").read_text(encoding="utf-8"))
        parsed = tomllib.loads(text)
        self.assertIn("workspace", parsed, "no parent workspace adopts the probe")
        self.assertEqual(parsed["dependencies"], {"autocfg": "=0.1.3"}, "the yanked version, exactly")
        held = {(p["name"], p["version"], p.get("checksum")) for p in lock["package"]}
        self.assertIn(probe.YANKED, held, "the lockfile holds the yanked version with its checksum")

    def test_edge_the_command_reads_this_repositorys_deny_toml(self):
        cmd = probe.command("cargo-deny", pathlib.Path("/x/Cargo.toml"))
        self.assertEqual(cmd[-2:], ["check", "advisories"], "the advisories check")
        self.assertEqual(cmd[cmd.index("--config") + 1], str(ROOT / "deny.toml"), "deny.toml")


if __name__ == "__main__":
    unittest.main()
