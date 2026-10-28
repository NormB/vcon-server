"""Tests scripts/check_package.py: what a crate published from this
repository may hold, and what each refusal says.

Run with: python3 -I scripts/test_check_package.py
Forbidden samples are assembled at run time so that this file never
contains them itself.
"""

import importlib.util
import pathlib
import sys
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("check_package", HERE / "check_package.py")
check_package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check_package)

MIT = b"MIT License\n"
APACHE = b"Apache License\n"
ROOT = {"LICENSE-MIT": MIT, "LICENSE-APACHE": APACHE}

MANIFEST = b"""[package]
edition = "2024"
rust-version = "1.99"
name = "demo"
version = "0.1.0"
description = "A demo crate."
homepage = "https://vcon-server.com"
documentation = "https://docs.rs/demo"
readme = "README.md"
keywords = ["vcon"]
categories = ["parser-implementations"]
license = "MIT OR Apache-2.0"
repository = "https://github.com/NormB/vcon-server"

[dependencies.serde]
version = "1.0.229"

[dev-dependencies.tempfile]
version = "3.27.0"
"""

README = b"# demo\n\nSee [the docs](https://vcon-server.com/docs/).\n"


def package(**changes):
    files = {
        "Cargo.toml": MANIFEST,
        "Cargo.toml.orig": b"[package]\n",
        ".cargo_vcs_info.json": b"{}\n",
        "README.md": README,
        "LICENSE-MIT": MIT,
        "LICENSE-APACHE": APACHE,
        "src/lib.rs": b"//! Demo.\n",
        "src/inner/mod.rs": b"// Inner.\n",
    }
    for path, content in changes.items():
        path = path.replace("__", "/").replace("_DOT_", ".")
        if content is None:
            files.pop(path, None)
        else:
            files[path] = content
    return files


def findings(files):
    return check_package.findings("demo-0.1.0", files, ROOT)


class Allowed(unittest.TestCase):
    def test_a_clean_package_has_no_findings(self):
        self.assertEqual(findings(package()), [])

    def test_a_binary_crate_may_hold_its_lock_file(self):
        self.assertEqual(findings(package(**{"Cargo_DOT_lock": b"version = 4\n"})), [])


PUBLISHED = ["vcon", "vcon-server"]


class Selection(unittest.TestCase):
    def test_a_member_marked_publish_false_is_not_packaged(self):
        members = [{"name": "vcon", "publish": None}, {"name": "vcon-server", "publish": None}, {"name": "vcon-client", "publish": []}]
        crates, found = check_package.to_publish(members, PUBLISHED)
        self.assertEqual([c["name"] for c in crates], ["vcon", "vcon-server"])
        self.assertEqual(found, [])

    def test_each_published_crate_must_stay_publishable(self):
        members = [{"name": "vcon", "publish": []}, {"name": "vcon-server", "publish": None}]
        crates, found = check_package.to_publish(members, PUBLISHED)
        self.assertEqual([c["name"] for c in crates], ["vcon-server"])
        self.assertTrue(any("vcon" in f and "publish" in f for f in found), found)

    def test_a_missing_published_crate_is_refused(self):
        crates, found = check_package.to_publish([{"name": "vcon", "publish": None}], PUBLISHED)
        self.assertTrue(any("vcon-server" in f for f in found), found)

    def test_the_list_is_read_from_the_workspace_manifest(self):
        manifest = '[workspace]\nmembers = ["a"]\n\n[workspace.metadata]\npublished = ["a", "b"]\n'
        self.assertEqual(check_package.published_crates(manifest), ["a", "b"])

    def test_a_workspace_without_the_list_is_refused(self):
        with self.assertRaises(SystemExit):
            check_package.published_crates('[workspace]\nmembers = ["a"]\n')


class Patches(unittest.TestCase):
    def test_each_published_crate_is_patched_in_by_path(self):
        args = check_package.patches(pathlib.Path("/run/x"), [("vcon-0.1.0", "vcon"), ("vcon-server-0.1.0", "vcon-server")])
        self.assertEqual(
            args,
            ["--config", 'patch.crates-io.vcon.path="/run/x/vcon-0.1.0"',
             "--config", 'patch.crates-io.vcon-server.path="/run/x/vcon-server-0.1.0"'],
        )


class Paths(unittest.TestCase):
    def refused(self, path):
        files = package()
        files[path] = b"x\n"
        found = findings(files)
        self.assertTrue(any(path in f and "not allowed" in f for f in found), found)

    def test_website_pages_are_refused(self):
        self.refused("website/content/_index.md")

    def test_assets_are_refused(self):
        self.refused("assets/brand/mark.svg")

    def test_test_fixtures_are_refused(self):
        self.refused("tests/fixtures/ietf/ab_call_ext_rec.vcon")

    def test_tests_are_refused(self):
        self.refused("tests/smoke.rs")

    def test_an_env_file_is_refused(self):
        self.refused(".env")

    def test_a_non_rust_file_under_src_is_refused(self):
        self.refused("src/notes.md")

    def test_a_document_export_is_refused(self):
        self.refused("vcon-server — Plan.pdf")


class BuildData(unittest.TestCase):
    def test_a_file_the_sources_include_is_allowed(self):
        files = package(**{"src__lib_DOT_rs": b'pub const API: &str = include_str!("../openapi.json");\n'})
        files["openapi.json"] = b"{}\n"
        self.assertEqual(findings(files), [])

    def test_bytes_included_from_a_nested_module_are_allowed(self):
        files = package(**{"src__inner__mod_DOT_rs": b'const PAGE: &[u8] = include_bytes!("../../console/index.html");\n'})
        files["console/index.html"] = b"<!doctype html>\n"
        self.assertEqual(findings(files), [])

    def test_the_same_file_without_an_include_is_refused(self):
        files = package()
        files["openapi.json"] = b"{}\n"
        found = findings(files)
        self.assertTrue(any("openapi.json" in f and "not allowed" in f for f in found), found)

    def test_an_include_that_leaves_the_package_allows_nothing(self):
        files = package(**{"src__lib_DOT_rs": b'const X: &str = include_str!("../../../secret.txt");\n'})
        files["secret.txt"] = b"x\n"
        found = findings(files)
        self.assertTrue(any("secret.txt" in f and "not allowed" in f for f in found), found)


class Required(unittest.TestCase):
    def missing(self, path):
        files = package()
        del files[path]
        found = findings(files)
        self.assertTrue(any(path in f and "missing" in f for f in found), found)

    def test_the_readme_is_required(self):
        self.missing("README.md")

    def test_the_mit_licence_is_required(self):
        self.missing("LICENSE-MIT")

    def test_the_apache_licence_is_required(self):
        self.missing("LICENSE-APACHE")

    def test_src_lib_rs_is_required(self):
        self.missing("src/lib.rs")

    def test_a_licence_must_match_the_root_copy(self):
        found = findings(package(**{"LICENSE-MIT": b"MIT License, changed\n"}))
        self.assertTrue(any("LICENSE-MIT" in f and "differs" in f for f in found), found)


class Manifest(unittest.TestCase):
    def manifest_with(self, old, new):
        assert old in MANIFEST, old
        return findings(package(**{"Cargo_DOT_toml": MANIFEST.replace(old, new)}))

    def test_a_git_dependency_is_refused(self):
        found = self.manifest_with(
            b'[dependencies.serde]\nversion = "1.0.229"',
            b'[dependencies.serde]\ngit = "https://github.com/serde-rs/serde"',
        )
        self.assertTrue(any("serde" in f and "git" in f for f in found), found)

    def test_a_git_dev_dependency_is_refused(self):
        found = self.manifest_with(
            b'[dev-dependencies.tempfile]\nversion = "3.27.0"',
            b'[dev-dependencies.tempfile]\ngit = "https://github.com/Stebalien/tempfile"',
        )
        self.assertTrue(any("tempfile" in f and "git" in f for f in found), found)

    def test_a_git_dependency_for_one_target_is_refused(self):
        found = findings(
            package(
                **{
                    "Cargo_DOT_toml": MANIFEST
                    + b'\n[target."cfg(unix)".dependencies.libc]\ngit = "https://github.com/rust-lang/libc"\n'
                }
            )
        )
        self.assertTrue(any("libc" in f and "git" in f for f in found), found)

    def test_a_path_dependency_is_refused(self):
        found = self.manifest_with(
            b'[dependencies.serde]\nversion = "1.0.229"',
            b'[dependencies.serde]\npath = "../serde"',
        )
        self.assertTrue(any("serde" in f and "path" in f for f in found), found)

    def test_a_git_source_in_the_lock_file_is_refused(self):
        lock = b'version = 4\n\n[[package]]\nname = "rusqlite"\nversion = "0.41.0"\nsource = "git+https://github.com/NormB/rusqlite?rev=a2936ca#a2936ca"\n'
        found = findings(package(**{"Cargo_DOT_lock": lock}))
        self.assertTrue(any("Cargo.lock" in f and "git" in f for f in found), found)

    def test_registry_sources_in_the_lock_file_are_allowed(self):
        lock = b'version = 4\n\n[[package]]\nname = "serde"\nversion = "1.0.229"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n'
        self.assertEqual(findings(package(**{"Cargo_DOT_lock": lock})), [])

    def test_publish_false_is_refused(self):
        found = self.manifest_with(b'name = "demo"\n', b'name = "demo"\npublish = false\n')
        self.assertTrue(any("publish" in f for f in found), found)

    def test_each_page_field_is_required(self):
        for field in (
            b"description",
            b"homepage",
            b"documentation",
            b"readme",
            b"keywords",
            b"categories",
            b"license",
            b"repository",
            b"rust-version",
        ):
            with self.subTest(field=field):
                lines = [line for line in MANIFEST.split(b"\n") if not line.startswith(field + b" =")]
                found = findings(package(**{"Cargo_DOT_toml": b"\n".join(lines)}))
                self.assertTrue(any(field.decode() in f and "missing" in f for f in found), found)

    def test_more_than_five_keywords_are_refused(self):
        found = self.manifest_with(
            b'keywords = ["vcon"]', b'keywords = ["a", "b", "c", "d", "e", "f"]'
        )
        self.assertTrue(any("keywords" in f for f in found), found)

    def test_five_keywords_are_allowed(self):
        found = self.manifest_with(b'keywords = ["vcon"]', b'keywords = ["a", "b", "c", "d", "e"]')
        self.assertEqual(found, [])


class Readme(unittest.TestCase):
    def readme(self, text):
        return findings(package(**{"README_DOT_md": text}))

    def test_a_relative_link_is_refused(self):
        found = self.readme(b"See [contributing](CONTRIBUTING.md).\n")
        self.assertTrue(any("CONTRIBUTING.md" in f and "relative" in f for f in found), found)

    def test_a_relative_image_is_refused(self):
        found = self.readme(b"![logo](assets/brand/mark.svg)\n")
        self.assertTrue(any("assets/brand/mark.svg" in f for f in found), found)

    def test_a_relative_html_source_is_refused(self):
        found = self.readme(b'<img src="assets/brand/mark.svg" alt="">\n')
        self.assertTrue(any("assets/brand/mark.svg" in f for f in found), found)

    def test_a_relative_reference_link_is_refused(self):
        found = self.readme(b"See [docs].\n\n[docs]: docs/index.md\n")
        self.assertTrue(any("docs/index.md" in f for f in found), found)

    def test_absolute_links_are_allowed(self):
        found = self.readme(
            b"[a](https://vcon-server.com/) ![b](https://vcon-server.com/b.svg)\n\n[c]: https://docs.rs/vcon\n"
        )
        self.assertEqual(found, [])

    def test_a_link_inside_a_code_block_is_not_a_link(self):
        found = self.readme(b"```rust\nlet x = v[0](1);\n```\n")
        self.assertEqual(found, [])


class Scan(unittest.TestCase):
    def test_personal_data_in_a_packaged_file_is_refused(self):
        email = "someone" + "@" + "gmail.com"
        found = findings(package(**{"src__lib_DOT_rs": f"//! Mail {email}.\n".encode()}))
        self.assertTrue(any("src/lib.rs" in f and "email" in f for f in found), found)

    def test_ai_attribution_in_a_packaged_file_is_refused(self):
        name = "Cl" + "aude"
        found = findings(package(**{"README_DOT_md": f"# demo\n\nWritten by {name}.\n".encode()}))
        self.assertTrue(any("README.md" in f and "AI tool" in f for f in found), found)

    def test_the_owner_name_is_refused_in_a_crate_readme(self):
        name = "Br" + "andinger"
        found = findings(package(**{"README_DOT_md": f"# demo\n\nBy N. {name}.\n".encode()}))
        self.assertTrue(any("README.md" in f and "owner" in f for f in found), found)

    def test_a_private_document_id_in_the_manifest_is_refused(self):
        task = "P" + "11.7"
        found = findings(package(**{"Cargo_DOT_toml": MANIFEST + f"# {task}\n".encode()}))
        self.assertTrue(any("Cargo.toml" in f and "plan task id" in f for f in found), found)


if __name__ == "__main__":
    result = unittest.main(exit=False, verbosity=1).result
    sys.exit(0 if result.wasSuccessful() else 1)
