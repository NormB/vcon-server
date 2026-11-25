"""Tests scripts/make-format-fixtures.py: every step that turns its inputs
into the pins it uses, the test master key, the configuration, the vCons,
the versions a fixture must keep and the manifest.

The steps that run a release binary in Docker and drive it over HTTPS are
not reachable here (they need Docker and a release archive); the fixtures
they wrote are checked by crates/vcon-server/tests/formats.rs.

Run with: python3 -I scripts/test_make_format_fixtures.py
"""

import contextlib
import hashlib
import importlib.util
import io
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


CI = (HERE.parent / ".github" / "workflows" / "ci.yml").read_text()


class PostgresReleases(unittest.TestCase):
    # success
    def test_the_postgres_releases_are_0_4_6_to_0_4_17(self):
        self.assertEqual(mff.PG_RELEASES, ("0.4.6", "0.4.7", "0.4.8", "0.4.9", "0.4.10", "0.4.11", "0.4.12", "0.4.13",
                                           "0.4.14", "0.4.15", "0.4.16", "0.4.17"),
                         "the release binaries with `postgres`")

    def test_every_release_from_0_4_6_on_is_a_postgres_release(self):
        # A release added to RELEASES gets its PostgreSQL data too.
        self.assertEqual(mff.PG_RELEASES, mff.RELEASES[mff.RELEASES.index("0.4.6"):], "one list, not two")

    def test_each_postgres_release_is_a_release(self):
        for release in mff.PG_RELEASES:
            self.assertIn(release, mff.RELEASES, release)

    def test_the_postgres_image_is_the_oldest_one_ci_runs(self):
        # pg_dump of the oldest major loads into every newer one.
        self.assertIn(f"image: {mff.PG_IMAGE} # 16.15-alpine", CI, "the 16.15 pin of backends-postgres")

    def test_the_rustfs_image_is_the_one_ci_runs(self):
        self.assertIn(f"image: {mff.RUSTFS_IMAGE} # 1.0.4", CI, "the RustFS pin of backends-postgres")

    # edge
    def test_both_images_are_pinned_by_digest(self):
        for image in (mff.PG_IMAGE, mff.RUSTFS_IMAGE):
            name, _, digest = image.partition("@sha256:")
            self.assertEqual(len(digest), 64, image)
            self.assertNotIn(":", name, f"no tag beside the digest: {image}")


class PgConfigText(unittest.TestCase):
    def doc(self, user="vcon_app", parquet=False):
        return tomllib.loads(mff.pg_config_text(f"0.0.0.0:{mff.PORT}", user, parquet))

    # success
    def test_the_catalogs_are_in_the_primary_over_tls(self):
        pg = self.doc()["catalog"]
        self.assertEqual(pg["backend"], "postgres", "backend")
        self.assertEqual(
            pg["postgres"],
            {"hosts": ["primary"], "port": 5432, "dbname": "vcon", "user": "vcon_app",
             "ca_file": "/w/pg-ca.pem", "password_file": "/w/pg.password"},
            "the primary pg-test-server.sh starts, by its certificate's name",
        )

    def test_the_objects_are_in_the_rustfs_bucket(self):
        doc = self.doc()
        self.assertEqual(doc["storage"]["objects"], "s3", "objects in a bucket")
        self.assertEqual(
            doc["storage"]["s3"],
            {"endpoint": "http://rustfs:9000", "region": "us-east-1", "bucket": mff.BUCKET,
             "path_style": True, "allow_http": True},
            "the RustFS container, by its network alias",
        )

    def test_migrate_logs_in_as_vcon_migrate(self):
        self.assertEqual(self.doc("vcon_migrate")["catalog"]["postgres"]["user"], "vcon_migrate", "the migrate role")

    def test_the_parquet_table_names_the_pack_interval(self):
        self.assertEqual(self.doc(parquet=True)["storage"]["parquet"], {"pack_interval": mff.PACK_INTERVAL}, "parquet")

    def test_the_base_keys_are_those_of_the_data_directory_fixtures(self):
        base = tomllib.loads(mff.config_text("x"))
        doc = self.doc()
        for key in ("listen", "data_dir", "master_key", "tls"):
            self.assertEqual(doc[key], base[key] if key != "listen" else f"0.0.0.0:{mff.PORT}", key)

    # failure
    def test_another_login_role_is_refused(self):
        with self.assertRaisesRegex(mff.FixtureError, "no role vcon_owner"):
            mff.pg_config_text("x", "vcon_owner", False)

    # edge
    def test_without_parquet_there_is_no_parquet_table(self):
        self.assertNotIn("parquet", self.doc()["storage"], "objects only until the pack phase")

    def test_the_pack_interval_outlasts_the_late_writes(self):
        # The late vCons must stay objects: no second pack run comes while
        # the script stores them and stops the server.
        self.assertGreaterEqual(mff.PACK_INTERVAL, 3600, "an hour at least")


class Places(unittest.TestCase):
    # success
    def test_the_workload_vcons_are_packed(self):
        for n in range(mff.VCONS_PER_TENANT):
            self.assertEqual(mff.place(n), "packed", n)

    def test_the_late_vcons_stay_objects(self):
        for n in mff.late_numbers():
            self.assertEqual(mff.place(n), "object", n)

    def test_there_are_late_vcons_after_the_workload_ones(self):
        self.assertEqual(list(mff.late_numbers()), [10, 11], "two per tenant, numbered on")

    # failure
    def test_a_negative_number_is_refused(self):
        with self.assertRaisesRegex(mff.FixtureError, "no vCon -1"):
            mff.place(-1)

    def test_a_number_past_the_late_ones_is_refused(self):
        with self.assertRaisesRegex(mff.FixtureError, "no vCon 12"):
            mff.place(12)

    # edge
    def test_the_late_uuids_are_not_workload_uuids(self):
        workload = {mff.vcon_uuid(t, n) for t, _ in mff.TENANTS for n in range(mff.VCONS_PER_TENANT)}
        late = {mff.vcon_uuid(t, n) for t, _ in mff.TENANTS for n in mff.late_numbers()}
        self.assertFalse(workload & late, "distinct uuids")

    def test_a_late_body_holds_reserved_values_only(self):
        for n in mff.late_numbers():
            doc = json.loads(mff.vcon_body("tenant-b", n, 0))
            self.assertTrue(100 <= int(doc["parties"][0]["tel"][-4:]) <= 199, doc["parties"][0])


class StripDump(unittest.TestCase):
    DUMP = (
        "--\n-- PostgreSQL database dump\n--\n\n"
        "\\restrict AbC123xyz\n\n"
        "SET statement_timeout = 0;\n"
        "INSERT INTO vcon.audit VALUES (1, '\\x00ff');\n"
        "\\unrestrict AbC123xyz\n"
    )

    # success
    def test_the_restrict_lines_are_removed(self):
        out = mff.strip_dump(self.DUMP)
        self.assertNotIn("restrict", out, "both meta-commands gone")

    def test_every_other_line_is_kept_as_it_was(self):
        out = mff.strip_dump(self.DUMP)
        kept = [l for l in self.DUMP.splitlines(keepends=True) if "restrict" not in l]
        self.assertEqual(out, "".join(kept), "byte-equal otherwise")

    # failure
    def test_another_meta_command_is_refused(self):
        with self.assertRaisesRegex(mff.FixtureError, r"line 2: \\connect vcon"):
            mff.strip_dump("SET x = 0;\n\\connect vcon\n")

    def test_copy_data_is_refused(self):
        with self.assertRaisesRegex(mff.FixtureError, "--inserts"):
            mff.strip_dump("COPY vcon.audit FROM stdin;\n1\tx\n\\.\n")

    def test_a_restrict_line_without_a_key_is_refused(self):
        with self.assertRaisesRegex(mff.FixtureError, "line 1"):
            mff.strip_dump("\\restrict\n")

    def test_a_dump_with_no_restrict_line_is_refused(self):
        # pg_dump 16.10 and later write one; its absence means another tool.
        with self.assertRaisesRegex(mff.FixtureError, "no \\\\restrict line"):
            mff.strip_dump("SET x = 0;\n")

    # edge
    def test_a_backslash_inside_a_value_is_kept(self):
        out = mff.strip_dump(self.DUMP)
        self.assertIn("'\\x00ff'", out, "a bytea literal is data")

    def test_an_indented_restrict_is_a_statement_line_and_refused(self):
        # psql reads a meta-command only at a line's start; anything else
        # is SQL the server would refuse, so the script refuses it first.
        with self.assertRaisesRegex(mff.FixtureError, "line 2"):
            mff.strip_dump("\\restrict K\n  \\restrict K\n")


LISTING = (
    '<?xml version="1.0" encoding="UTF-8"?>'
    '<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">'
    "<Name>b</Name><IsTruncated>{t}</IsTruncated>{keys}</ListBucketResult>"
)


def listing(keys, truncated="false"):
    return LISTING.format(t=truncated, keys="".join(f"<Contents><Key>{k}</Key></Contents>" for k in keys))


class ListedKeys(unittest.TestCase):
    # success
    def test_every_key_is_read_in_order(self):
        self.assertEqual(mff.listed_keys(listing(["a/1", "b/2"])), ["a/1", "b/2"], "both")

    def test_an_empty_bucket_lists_nothing(self):
        self.assertEqual(mff.listed_keys(listing([])), [], "none")

    # failure
    def test_a_truncated_listing_is_refused(self):
        with self.assertRaisesRegex(mff.FixtureError, "truncated"):
            mff.listed_keys(listing(["a"], "true"))

    def test_an_answer_that_is_not_a_listing_is_refused(self):
        with self.assertRaisesRegex(mff.FixtureError, "not a bucket listing"):
            mff.listed_keys("<Error><Code>NoSuchBucket</Code></Error>")

    # edge
    def test_a_listing_without_the_namespace_is_read(self):
        text = "<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>k</Key></Contents></ListBucketResult>"
        self.assertEqual(mff.listed_keys(text), ["k"], "no xmlns")

    def test_an_escaped_key_is_unescaped(self):
        self.assertEqual(mff.listed_keys(listing(["a&amp;b"])), ["a&b"], "XML entities")


class BucketPath(unittest.TestCase):
    # success
    def test_a_key_is_a_relative_path(self):
        self.assertEqual(mff.bucket_path("tenants/ab/objects/cd"), pathlib.PurePosixPath("tenants/ab/objects/cd"), "as is")

    def test_a_parquet_key_is_a_relative_path(self):
        key = "tenants/ab/parquet/date=2026-11-17/part-01.parquet"
        self.assertEqual(str(mff.bucket_path(key)), key, "as is")

    # failure
    def test_a_parent_step_is_refused(self):
        with self.assertRaisesRegex(mff.FixtureError, "not a plain path"):
            mff.bucket_path("tenants/../x")

    def test_an_absolute_key_is_refused(self):
        with self.assertRaisesRegex(mff.FixtureError, "not a plain path"):
            mff.bucket_path("/etc/x")

    def test_an_empty_step_is_refused(self):
        with self.assertRaisesRegex(mff.FixtureError, "not a plain path"):
            mff.bucket_path("a//b")

    def test_a_dot_step_is_refused(self):
        with self.assertRaisesRegex(mff.FixtureError, "not a plain path"):
            mff.bucket_path("a/./b")

    # edge
    def test_an_empty_key_is_refused(self):
        with self.assertRaisesRegex(mff.FixtureError, "not a plain path"):
            mff.bucket_path("")

    def test_a_trailing_slash_is_refused(self):
        with self.assertRaisesRegex(mff.FixtureError, "not a plain path"):
            mff.bucket_path("a/")


class PgManifest(unittest.TestCase):
    def test_the_postgres_section_is_written_when_given(self):
        section = {"schema": 13, "dump": "dump.sql"}
        doc = json.loads(mff.manifest("0.4.6", SHA_A, SHA_B, [], [], [], [], postgres=section))
        self.assertEqual(doc["postgres"], section, "as given")

    def test_no_postgres_section_by_default(self):
        doc = json.loads(mff.manifest("0.4.6", SHA_A, SHA_B, [], [], [], []))
        self.assertNotIn("postgres", doc, "the data directory fixtures are unchanged")


class CacheDir(unittest.TestCase):
    # success
    def test_a_missing_cache_is_made_with_its_parents(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "a" / "cache"
            self.assertEqual(mff.cache_dir(str(path)), path, "the path given")
            self.assertTrue(path.is_dir(), "made, with its parent")

    def test_an_existing_cache_is_kept_with_what_it_holds(self):
        with tempfile.TemporaryDirectory() as tmp:
            (pathlib.Path(tmp) / "archive.tar.gz").write_bytes(b"x")
            mff.cache_dir(tmp)
            self.assertEqual((pathlib.Path(tmp) / "archive.tar.gz").read_bytes(), b"x", "unchanged")

    # failure
    def test_a_file_where_the_cache_should_be_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "cache"
            path.write_bytes(b"")
            with self.assertRaises(SystemExit) as caught:
                mff.cache_dir(str(path))
            self.assertIn("not a directory", str(caught.exception), "named")

    def test_main_makes_the_cache_before_a_release_is_made(self):
        # make() itself needs Docker; a stand-in records the cache it got.
        seen = []
        real = mff.make
        mff.make = lambda release, arch, cache, out: seen.append(pathlib.Path(cache).is_dir()) or 0
        try:
            with tempfile.TemporaryDirectory() as tmp:
                with contextlib.redirect_stdout(io.StringIO()):
                    rc = mff.main(["--cache", f"{tmp}/new/cache", "--release", "0.4.0", "--out", tmp])
        finally:
            mff.make = real
        self.assertEqual((rc, seen), (0, [True]), "the cache was there when make ran")

    # edge
    def test_a_cache_named_with_a_trailing_slash_is_the_same_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            made = mff.cache_dir(tmp + "/c/")
            self.assertEqual(made, pathlib.Path(tmp) / "c", "one directory")


if __name__ == "__main__":
    unittest.main()
