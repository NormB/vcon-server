"""Tests scripts/make-format-fixtures.py: every step that turns its inputs
into the pins it uses, the test master key, the configuration, the vCons,
the versions a fixture must keep and the manifest.

The steps that run a release binary in Docker and drive it over HTTPS are
not reachable here (they need Docker and a release archive); the fixtures
they wrote are checked by crates/vcon-server/tests/formats.rs.

Run with: python3 -I scripts/test_make_format_fixtures.py
"""

import hashlib
import importlib.util
import json
import pathlib
import sys
import tempfile
import tomllib
import unittest
import uuid as uuidlib

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("make_format_fixtures", HERE / "make-format-fixtures.py")
mff = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mff)

SHA_A = "a" * 64
SHA_B = "b" * 64
BASE = "https://github.com/NormB/vcon-server/releases/download"


def pin(sha, release, arch):
    return f"{sha}  {BASE}/v{release}/vcon-server-{release}-{arch}-unknown-linux-gnu.tar.gz"


PINS = "\n".join(
    [
        "# a comment naming vcon-server-0.4.0-aarch64-unknown-linux-gnu.tar.gz",
        "",
        f"{SHA_B}  {BASE}/v0.4.0/SHA256SUMS.txt",
        pin(SHA_A, "0.4.0", "aarch64"),
        pin(SHA_B, "0.4.0", "x86_64"),
        pin(SHA_B, "0.4.10", "aarch64"),
    ]
)


class PinFor(unittest.TestCase):
    # success
    def test_the_archive_of_the_release_and_arch_is_found(self):
        sha, url = mff.pin_for(PINS, "0.4.0", "aarch64")
        self.assertEqual(sha, SHA_A, "the aarch64 pin")
        self.assertTrue(url.endswith("/v0.4.0/vcon-server-0.4.0-aarch64-unknown-linux-gnu.tar.gz"), url)

    def test_the_other_arch_is_its_own_pin(self):
        self.assertEqual(mff.pin_for(PINS, "0.4.0", "x86_64")[0], SHA_B, "the x86_64 pin")

    def test_the_real_pins_hold_every_release_for_both_arches(self):
        text = (HERE / "docs-test-pins.txt").read_text()
        for release in mff.RELEASES:
            for arch in ("aarch64", "x86_64"):
                sha, _ = mff.pin_for(text, release, arch)
                self.assertEqual(len(sha), 64, f"{release} {arch}")

    # failure
    def test_a_release_with_no_pin_is_refused(self):
        with self.assertRaisesRegex(mff.FixtureError, "0 pins name"):
            mff.pin_for(PINS, "0.4.1", "aarch64")

    def test_a_comment_naming_the_archive_is_not_a_pin(self):
        with self.assertRaisesRegex(mff.FixtureError, "0 pins name"):
            mff.pin_for("# " + pin(SHA_A, "0.4.0", "aarch64"), "0.4.0", "aarch64")

    def test_a_commented_out_pin_is_not_a_pin(self):
        # Two fields, the first "#" and a hash: skipped as a comment, not
        # refused as a bad hash.
        with self.assertRaisesRegex(mff.FixtureError, "0 pins name"):
            mff.pin_for("#" + pin(SHA_A, "0.4.0", "aarch64"), "0.4.0", "aarch64")

    def test_two_pins_of_one_archive_are_refused(self):
        with self.assertRaisesRegex(mff.FixtureError, "2 pins name"):
            mff.pin_for(PINS + "\n" + pin(SHA_B, "0.4.0", "aarch64"), "0.4.0", "aarch64")

    def test_a_pin_that_is_not_a_sha256_is_refused(self):
        with self.assertRaisesRegex(mff.FixtureError, "not a sha256"):
            mff.pin_for(pin("A" * 64, "0.4.0", "aarch64"), "0.4.0", "aarch64")

    def test_a_short_hash_is_refused(self):
        with self.assertRaisesRegex(mff.FixtureError, "not a sha256"):
            mff.pin_for(pin("a" * 63, "0.4.0", "aarch64"), "0.4.0", "aarch64")

    # edge
    def test_a_release_whose_name_starts_another_is_not_matched(self):
        # 0.4.1 is a prefix of 0.4.10's directory only without the slash.
        self.assertEqual(mff.pin_for(PINS, "0.4.10", "aarch64")[0], SHA_B, "0.4.10 alone")
        with self.assertRaisesRegex(mff.FixtureError, "0 pins name"):
            mff.pin_for(PINS, "0.4.1", "aarch64")

    def test_a_line_with_a_third_field_is_not_a_pin(self):
        with self.assertRaisesRegex(mff.FixtureError, "0 pins name"):
            mff.pin_for(pin(SHA_A, "0.4.0", "aarch64") + " extra", "0.4.0", "aarch64")


class MasterKey(unittest.TestCase):
    def test_the_key_is_32_bytes_and_the_same_each_time(self):
        self.assertEqual(len(mff.master_key("0.4.0")), 32, "32 bytes")
        self.assertEqual(mff.master_key("0.4.0"), mff.master_key("0.4.0"), "stable")

    def test_each_release_has_its_own_key(self):
        keys = {mff.master_key(r) for r in mff.RELEASES}
        self.assertEqual(len(keys), len(mff.RELEASES), "one key per release")

    def test_the_key_is_sha256_of_the_published_text(self):
        self.assertEqual(
            mff.master_key("0.4.8"), hashlib.sha256(mff.KEY_TEXT + b"0.4.8").digest(), "the documented derivation"
        )


class ConfigText(unittest.TestCase):
    def test_the_configuration_names_the_mounted_paths(self):
        doc = tomllib.loads(mff.config_text("0.0.0.0:8443"))
        self.assertEqual(doc["listen"], "0.0.0.0:8443", "listen")
        self.assertEqual(doc["data_dir"], "/w/data", "data_dir")
        self.assertEqual(doc["master_key"], "/w/master.key", "master_key")
        self.assertEqual(doc["tls"], {"cert": "/w/cert.pem", "key": "/w/key.pem"}, "tls")

    def test_the_configuration_sets_nothing_else(self):
        # A key 0.4.0 does not know would stop it: every table refuses one.
        doc = tomllib.loads(mff.config_text("x"))
        self.assertEqual(sorted(doc), ["data_dir", "listen", "master_key", "tls"], "only the required keys")


class Vcons(unittest.TestCase):
    def test_a_uuid_is_stable_and_has_the_version_4_layout(self):
        u = uuidlib.UUID(mff.vcon_uuid("tenant-a", 3))
        self.assertEqual(u.version, 4, "version 4")
        self.assertEqual(str(u), mff.vcon_uuid("tenant-a", 3), "stable")

    def test_uuids_differ_by_tenant_and_number(self):
        made = {mff.vcon_uuid(t, n) for t, _ in mff.TENANTS for n in range(mff.VCONS_PER_TENANT)}
        self.assertEqual(len(made), 2 * mff.VCONS_PER_TENANT, "every uuid distinct")

    def test_a_body_is_json_naming_its_uuid(self):
        doc = json.loads(mff.vcon_body("tenant-b", 7, 0))
        self.assertEqual(doc["uuid"], mff.vcon_uuid("tenant-b", 7), "its uuid")
        self.assertEqual(doc["vcon"], "0.4.0", "the core-04 version field")

    def test_each_edition_is_other_bytes(self):
        self.assertNotEqual(mff.vcon_body("tenant-a", 0, 0), mff.vcon_body("tenant-a", 0, 1), "editions differ")

    def test_a_body_holds_reserved_values_only(self):
        for n in range(mff.VCONS_PER_TENANT):
            doc = json.loads(mff.vcon_body("tenant-a", n, 0))
            tel = doc["parties"][0]["tel"]
            self.assertTrue(tel.startswith("+1555555") and 100 <= int(tel[-4:]) <= 199, tel)
            self.assertTrue(doc["parties"][1]["mailto"].endswith("@example.com"), doc["parties"][1])

    def test_a_body_is_compact_and_sorted(self):
        body = mff.vcon_body("tenant-a", 1, 0)
        self.assertNotIn(b": ", body, "no spaces after a colon")
        self.assertEqual(body, json.dumps(json.loads(body), separators=(",", ":"), sort_keys=True).encode(), "sorted")


class Expected(unittest.TestCase):
    def setUp(self):
        self.want = mff.expected()
        self.a, self.b = self.want["tenant-a"], self.want["tenant-b"]

    def rows(self, tenant, n):
        return self.want[tenant]["kept"][mff.vcon_uuid(tenant, n)]

    # success
    def test_a_replaced_vcon_keeps_its_hidden_version_under_7y(self):
        self.assertEqual(self.rows("tenant-a", 0), [(0, "hidden"), (1, "current")], "both editions")

    def test_a_deleted_vcon_is_kept_under_7y(self):
        self.assertEqual(self.rows("tenant-a", 2), [(0, "deleted")], "kept, deleted")

    def test_tenant_a_keeps_every_uuid(self):
        self.assertEqual(len(self.a["kept"]), mff.VCONS_PER_TENANT, "ten uuids")
        self.assertEqual(self.a["purged"], [], "nothing purged")

    def test_an_untouched_vcon_is_current(self):
        self.assertEqual(self.rows("tenant-a", 9), [(0, "current")], "one current version")

    # failure: what a 0d tenant loses
    def test_a_replaced_vcon_under_0d_loses_its_hidden_version(self):
        self.assertEqual(self.rows("tenant-b", 0), [(1, "current")], "edition 0 purged")

    def test_a_deleted_vcon_under_0d_is_purged(self):
        self.assertEqual(self.b["purged"], [mff.vcon_uuid("tenant-b", 1)], "the deleted uuid")
        self.assertNotIn(mff.vcon_uuid("tenant-b", 1), self.b["kept"], "not kept")

    def test_an_unknown_step_is_refused(self):
        with self.assertRaisesRegex(mff.FixtureError, "unknown step"):
            mff.expected({"tenant-a": [("pack", 0)]})

    # edge: holds
    def test_a_held_deleted_vcon_is_kept_under_0d(self):
        self.assertEqual(self.rows("tenant-b", 2), [(0, "deleted+held")], "held though due")

    def test_a_held_replaced_vcon_keeps_its_hidden_version_under_0d(self):
        self.assertEqual(self.rows("tenant-b", 3), [(0, "hidden+held"), (1, "current+held")], "both kept")

    def test_a_held_live_vcon_is_current_and_held(self):
        self.assertEqual(self.rows("tenant-a", 5), [(0, "current+held")], "held")

    def test_a_held_deleted_vcon_under_7y_is_deleted_and_held(self):
        self.assertEqual(self.rows("tenant-a", 4), [(0, "deleted+held")], "held")

    def test_the_counts_of_each_tenant(self):
        count = lambda t: sum(len(r) for r in self.want[t]["kept"].values())  # noqa: E731
        self.assertEqual((count("tenant-a"), count("tenant-b")), (12, 10), "versions kept")

    def test_an_empty_workload_keeps_one_current_version_each(self):
        want = mff.expected({})
        for tenant, _ in mff.TENANTS:
            self.assertEqual(
                {tuple(r) for r in want[tenant]["kept"].values()}, {((0, "current"),)}, tenant
            )


class Manifest(unittest.TestCase):
    def row(self, tenant, uuid, order):
        return {"tenant": tenant, "uuid": uuid, "version": "v", "order": order, "state": "current",
                "sha256": SHA_A, "bytes": 1}

    def test_the_manifest_sorts_versions_by_tenant_uuid_and_order(self):
        rows = [self.row("tenant-b", "1", 0), self.row("tenant-a", "2", 1), self.row("tenant-a", "2", 0)]
        doc = json.loads(mff.manifest("0.4.0", SHA_A, SHA_B, rows, [], [], []))
        self.assertEqual([(r["tenant"], r["order"]) for r in doc["versions"]],
                         [("tenant-a", 0), ("tenant-a", 1), ("tenant-b", 0)], "sorted")

    def test_the_manifest_names_what_made_it(self):
        doc = json.loads(mff.manifest("0.4.3", SHA_A, SHA_B, [], [], [], []))
        self.assertEqual((doc["release"], doc["archive_sha256"], doc["binary_sha256"], doc["image"]),
                         ("0.4.3", SHA_A, SHA_B, mff.IMAGE), "provenance")

    def test_the_manifest_text_ends_with_one_newline(self):
        text = mff.manifest("0.4.0", SHA_A, SHA_B, [], [], [], [])
        self.assertTrue(text.endswith("}\n") and not text.endswith("\n\n"), repr(text[-3:]))


class HasUsers(unittest.TestCase):
    def test_the_user_command_is_found(self):
        self.assertTrue(mff.has_users("Commands:\n  serve  Starts\n  user   Makes users\n"), "listed")

    def test_no_user_command_before_0_4_4(self):
        self.assertFalse(mff.has_users("Commands:\n  serve  Starts\n  tenant  Makes\n"), "absent")

    def test_a_word_that_starts_with_user_is_not_the_command(self):
        self.assertFalse(mff.has_users("  users  x\n  user-agent  y\n"), "only `user` itself")

    def test_user_in_a_description_is_not_the_command(self):
        self.assertFalse(mff.has_users("  membership  Grants a user roles\n"), "not first")


class TreeBytes(unittest.TestCase):
    def test_the_sizes_of_files_in_every_directory_are_added(self):
        with tempfile.TemporaryDirectory() as d:
            p = pathlib.Path(d)
            (p / "a").write_bytes(b"123")
            (p / "sub").mkdir()
            (p / "sub" / "b").write_bytes(b"4567")
            self.assertEqual(mff.tree_bytes(p), 7, "3 + 4, directories not counted")

    def test_an_empty_directory_is_zero(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertEqual(mff.tree_bytes(d), 0, "nothing")


if __name__ == "__main__":
    unittest.main()
