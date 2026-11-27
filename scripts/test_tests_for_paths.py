"""Tests scripts/tests_for_paths.py: the tests to run in the quick slot for a
change, the whole-tree gates among them.

A gate that scans every file under a directory names no file, so `git grep
-l <touched path>` over the tests never finds it, and the commit hook is the
first to fail it after its wait in the build queue (sipnab, twice on
2026-11-26). These tests drive the rule on small trees held in memory, check
it on the real tree, and hold it to a second census that finds directory
walks another way.

Test names start with the kind of case: test_success_, test_failure_ (the
rule or the script refuses, or a check finds a fault) and test_edge_.

Run with: python3 -I scripts/test_tests_for_paths.py
"""

import contextlib
import importlib.util
import io
import json
import pathlib
import sys
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parent
spec = importlib.util.spec_from_file_location("tests_for_paths", HERE / "tests_for_paths.py")
tfp = importlib.util.module_from_spec(spec)
sys.modules["tests_for_paths"] = tfp
spec.loader.exec_module(tfp)

# The command name in two halves, as the scripts write it.
LISTING = "ls-" + "files"
SERVER = "crates/vcon-server/"

# scripts/test-hooks.sh runs this file in its fixture repository, whose first
# commit runs the whole hook: a stand-in crate named hook-fixture where the
# server's crate is (scripts/test_check_test_runner.py accepts it the same
# way). The fixture keeps none of the server's tests, so the facts RealTree
# checks are about a tree it is not; CI and the commit hook check them in the
# server's own tree. On 2026-11-27 its pre-push run refused the push for that.
FIXTURE_PACKAGE = "hook-fixture"


def package_of(manifest_text):
    """The [package] name of a Cargo.toml text."""
    import tomllib

    return tomllib.loads(manifest_text)["package"]["name"]


def tree_package(root=REPO):
    return package_of((root / SERVER / "Cargo.toml").read_text(encoding="utf-8"))


def is_server_tree(package):
    """True for the server's tree, False for the hook tests' fixture; any other
    name is an error, so a renamed crate cannot turn RealTree off unseen."""
    if package not in ("vcon-server", FIXTURE_PACKAGE):
        raise ValueError(f"{SERVER}Cargo.toml names {package!r}: neither vcon-server nor {FIXTURE_PACKAGE}")
    return package == "vcon-server"

SRC_WALK = """\
use std::path::Path;
fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        out.push(entry.unwrap().path());
    }
}
#[test]
fn no_print_in_src() {
    let mut files = Vec::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
}
"""

SITE_WALK = SRC_WALK.replace('join("src")', 'join("../../website/content")')

PLAIN = """\
#[test]
fn adds() {
    assert_eq!(1 + 1, 2, "arithmetic");
}
"""

LIB_WALK = """\
pub mod cli;
#[cfg(test)]
mod tests {
    #[test]
    fn no_banned_call_in_src() {
        for e in std::fs::read_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")) {}
    }
}
"""

PRODUCTION_WALK = """\
pub fn list(d: &std::path::Path) { for e in std::fs::read_dir(d) {} }
#[cfg(test)]
mod tests {
    #[test]
    fn adds() { assert_eq!(1 + 1, 2, "arithmetic"); }
}
"""

HOOK = """\
#!/usr/bin/env bash
python3 -I scripts/test_check_style.py
python3 -I scripts/check_style.py --walk
scripts/test-retry.sh
python3 -I scripts/clippy_features.py run --quiet
scripts/nextest.sh run --workspace --locked
"""

BASE = {
    "Cargo.toml": '[workspace]\nmembers = ["crates/vcon-server", "crates/vcon"]\n',
    SERVER + "Cargo.toml": '[package]\nname = "vcon-server"\n',
    "crates/vcon/Cargo.toml": '[package]\nname = "vcon"\n',
    ".githooks/pre-commit": HOOK,
}
BASE_TRACKED = [
    "README.md",
    "docs/install.md",
    "website/content/docs/a.md",
    "website/content/_index.md",
    SERVER + "src/main.rs",
    SERVER + "src/cli.rs",
    SERVER + "README.md",
    "crates/vcon/src/lib.rs",
    "scripts/check_style.py",
    "scripts/retry.sh",
]


def tree(files, tracked=()):
    every = {**BASE, **files}
    return tfp.Tree(read=every.get, tracked=sorted(set(every) | set(BASE_TRACKED) | set(tracked)))


def names(jobs):
    return {job.name for job in jobs}


def select(files, paths, **kw):
    kw.setdefault("scenarios_needed", lambda paths: [])
    return tfp.select(tree(files), paths, **kw)


def run_main(argv, **kw):
    """The command line, with relative paths read from the repository root:
    a test that left the directory to the process would read them from
    wherever the test was started."""
    out, err = io.StringIO(), io.StringIO()
    kw.setdefault("scenarios_needed", lambda paths: [])
    kw.setdefault("cwd", REPO)
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        try:
            rc = tfp.main(argv, **kw)
        except SystemExit as stop:
            rc = stop.code
    return rc, out.getvalue(), err.getvalue()


def t_of(files, path, tracked=()):
    t = tree(files, tracked)
    return t, tfp.target_of(t, path)


_REAL = None


def real():
    global _REAL
    if _REAL is None:
        _REAL = tfp.Tree.of_repo(REPO)
    return _REAL


class Rule(unittest.TestCase):
    def test_success_a_read_dir_walk_of_src_is_whole_tree(self):
        t, target = t_of({SERVER + "tests/a.rs": SRC_WALK}, SERVER + "tests/a.rs")
        self.assertEqual(tfp.marks(target, t), ["read_dir"])

    def test_success_a_walkdir_walk_is_whole_tree(self):
        text = 'fn f() { for e in walkdir::WalkDir::new(root.join("src")) {} }\n'
        t, target = t_of({SERVER + "tests/a.rs": text}, SERVER + "tests/a.rs")
        self.assertIn("WalkDir", tfp.marks(target, t))

    def test_success_a_listing_of_tracked_files_runs_everywhere(self):
        text = f'fn f() {{ git.args(["{LISTING}", "-z"]); r("src/"); }}\n'
        t, target = t_of({SERVER + "tests/a.rs": text}, SERVER + "tests/a.rs")
        self.assertTrue(tfp.scope(target, t).everywhere)

    def test_success_a_walk_in_an_included_support_module_counts(self):
        files = {
            SERVER + "tests/a.rs": '#[path = "support/walk.rs"]\nmod walk;\n',
            SERVER + "tests/support/walk.rs": 'pub fn w() { std::fs::read_dir(root.join("src")); }\n',
        }
        t, target = t_of(files, SERVER + "tests/a.rs")
        self.assertEqual(tfp.marks(target, t), ["read_dir"])

    def test_success_a_walk_in_an_include_macro_file_counts(self):
        files = {
            SERVER + "tests/a.rs": 'include!("support/walk.rs");\n',
            SERVER + "tests/support/walk.rs": 'fn w() { std::fs::read_dir(root.join("src")); }\n',
        }
        t, target = t_of(files, SERVER + "tests/a.rs")
        self.assertEqual(tfp.marks(target, t), ["read_dir"])

    def test_success_a_crate_root_mod_resolves_beside_it(self):
        files = {
            SERVER + "tests/a.rs": "mod markdown;\n",
            SERVER + "tests/markdown/mod.rs": 'fn w() { std::fs::read_dir(root.join("src")); }\n',
        }
        t, target = t_of(files, SERVER + "tests/a.rs")
        self.assertEqual(tfp.marks(target, t), ["read_dir"])

    def test_success_a_git_grep_runs_everywhere(self):
        text = 'fn f() { Command::new("git").args(["grep", "-l", "x"]); }\n'
        t, target = t_of({SERVER + "tests/a.rs": text}, SERVER + "tests/a.rs")
        self.assertEqual((tfp.marks(target, t), tfp.scope(target, t).everywhere), (["git grep"], True))

    def test_success_a_test_that_lists_the_changed_files_runs_everywhere(self):
        text = 'fn f() { git(&["diff", "--name-only", &tag]); r("src/"); }\n'
        t, target = t_of({SERVER + "tests/a.rs": text}, SERVER + "tests/a.rs")
        self.assertEqual((tfp.marks(target, t), tfp.scope(target, t).everywhere), (["git history"], True))

    def test_success_a_script_test_that_lists_the_changed_files_runs_everywhere(self):
        files = {"scripts/test_check_style.py": "run(['git', 'diff', '--name-only', 'origin/main'])\n"}
        t, target = t_of(files, "scripts/test_check_style.py")
        self.assertEqual((tfp.marks(target, t), tfp.scope(target, t).everywhere), (["git history"], True))

    def test_failure_a_diff_of_two_strings_is_no_history_read(self):
        t, target = t_of({SERVER + "tests/a.rs": 'fn f() { assert_eq!(k, "diff", "a word"); }\n'}, SERVER + "tests/a.rs")
        self.assertEqual(tfp.marks(target, t), [])

    def test_failure_a_test_with_no_walk_is_not_whole_tree(self):
        t, target = t_of({SERVER + "tests/a.rs": PLAIN}, SERVER + "tests/a.rs")
        self.assertEqual(tfp.marks(target, t), [])

    def test_failure_a_walk_of_proc_is_not_a_tree_walk(self):
        text = 'fn f() { std::fs::read_dir(format!("/proc/{pid}/fd")); std::fs::read_dir("/proc"); }\n'
        t, target = t_of({SERVER + "tests/a.rs": text}, SERVER + "tests/a.rs")
        self.assertEqual(tfp.marks(target, t), [])

    def test_failure_a_comment_that_names_read_dir_is_not_a_walk(self):
        t, target = t_of({SERVER + "tests/a.rs": "// read_dir(src) is not used\n" + PLAIN}, SERVER + "tests/a.rs")
        self.assertEqual(tfp.marks(target, t), [])

    def test_edge_an_include_cycle_ends(self):
        files = {
            SERVER + "tests/a.rs": '#[path = "support/x.rs"]\nmod x;\n',
            SERVER + "tests/support/x.rs": 'include!("y.rs");\n',
            SERVER + "tests/support/y.rs": 'include!("x.rs");\nfn w() { std::fs::read_dir(root.join("src")); }\n',
        }
        t, target = t_of(files, SERVER + "tests/a.rs")
        self.assertEqual(tfp.marks(target, t), ["read_dir"])

    def test_edge_a_missing_include_is_skipped(self):
        t, target = t_of({SERVER + "tests/a.rs": '#[path = "support/gone.rs"]\nmod gone;\n'}, SERVER + "tests/a.rs")
        self.assertEqual(tfp.marks(target, t), [])

    def test_edge_crlf_line_endings_are_read(self):
        t, target = t_of({SERVER + "tests/a.rs": SRC_WALK.replace("\n", "\r\n")}, SERVER + "tests/a.rs")
        self.assertEqual(tfp.marks(target, t), ["read_dir"])

    def test_edge_the_split_listing_name_is_found(self):
        text = 'fn f() { git.args([concat!("ls-", "files")]); }\n'
        t, target = t_of({SERVER + "tests/a.rs": text}, SERVER + "tests/a.rs")
        self.assertIn("git " + LISTING, tfp.marks(target, t))

    def test_edge_a_member_outside_the_workspace_has_no_target(self):
        t = tree({"fuzz/Cargo.toml": '[package]\nname = "fuzz"\n', "fuzz/tests/a.rs": SRC_WALK})
        self.assertNotIn("fuzz/tests/a.rs", {x.file for x in tfp.targets(t)})


class Scope(unittest.TestCase):
    def test_success_a_crate_literal_names_the_crates_own_directory(self):
        t, target = t_of({SERVER + "tests/a.rs": SRC_WALK}, SERVER + "tests/a.rs")
        self.assertEqual(tfp.scope(target, t).roots, frozenset({SERVER + "src"}))

    def test_success_a_climbing_literal_names_a_repository_directory(self):
        t, target = t_of({SERVER + "tests/a.rs": SITE_WALK}, SERVER + "tests/a.rs")
        self.assertEqual(tfp.scope(target, t).roots, frozenset({"website/content"}))

    def test_edge_a_climb_to_a_sibling_crate_names_that_crate(self):
        text = 'fn f() { for e in std::fs::read_dir(root.join("../vcon/src")) {} }\n'
        t, target = t_of({SERVER + "tests/a.rs": text}, SERVER + "tests/a.rs")
        self.assertEqual(tfp.scope(target, t).roots, frozenset({"crates/vcon/src"}))

    def test_edge_a_format_string_root_is_read_from_its_components(self):
        text = 'fn f() { for e in std::fs::read_dir(format!("{}/src/{}", r, d)) {} }\n'
        t, target = t_of({SERVER + "tests/a.rs": text}, SERVER + "tests/a.rs")
        self.assertEqual(tfp.scope(target, t).roots, frozenset({SERVER + "src"}))

    def test_edge_a_literal_of_several_words_gives_a_root_per_word(self):
        text = 'fn f() { for e in std::fs::read_dir(d) {} let m = "see `docs/install.md`: and (src/cli.rs);"; }\n'
        t, target = t_of({SERVER + "tests/a.rs": text}, SERVER + "tests/a.rs")
        self.assertEqual(tfp.scope(target, t).roots, frozenset({"docs/install.md", SERVER + "src/cli.rs"}))

    def test_edge_a_walker_that_names_no_tracked_directory_runs_everywhere(self):
        text = "fn f(tmp: &Path) { for e in std::fs::read_dir(tmp) {} }\n"
        t, target = t_of({SERVER + "tests/a.rs": text}, SERVER + "tests/a.rs")
        self.assertTrue(tfp.scope(target, t).everywhere)

    def test_edge_a_spaced_borrow_of_the_repo_root_is_a_root_walk(self):
        text = 'fn f() { for e in std::fs::read_dir( &repo() ) {} let x = "src"; }\n'
        t, target = t_of({SERVER + "tests/a.rs": text}, SERVER + "tests/a.rs")
        self.assertTrue(tfp.scope(target, t).everywhere)

    def test_edge_a_longer_name_is_not_its_prefix_directory(self):
        text = 'fn f() { for e in std::fs::read_dir(r.join("srcx")) {} let y = "../../docs"; }\n'
        t, target = t_of({SERVER + "tests/a.rs": text}, SERVER + "tests/a.rs")
        self.assertEqual(tfp.scope(target, t).roots, frozenset({"docs"}))


class Lib(unittest.TestCase):
    def test_success_a_unit_test_that_walks_src_makes_the_lib_whole_tree(self):
        files = {"crates/vcon/src/lib.rs": LIB_WALK, "crates/vcon/src/cli.rs": ""}
        t, lib = t_of(files, "crates/vcon/src/lib.rs")
        self.assertEqual((lib.name, tfp.marks(lib, t)), ("vcon lib", ["read_dir"]))

    def test_success_the_lib_runs_for_a_file_it_compiles(self):
        files = {"crates/vcon/src/lib.rs": "pub mod cli;\n", "crates/vcon/src/cli.rs": "pub fn f() {}\n"}
        jobs = select(files, ["crates/vcon/src/cli.rs"])
        self.assertEqual(
            [(j.command, j.reasons) for j in jobs if j.name == "vcon lib"],
            [("heavy-run --quick -- scripts/nextest.sh run -p vcon --locked --lib --bins", ["compiles crates/vcon/src/cli.rs"])],
        )

    def test_success_a_quick_line_runs_a_binary_under_the_hooks_own_runner(self):
        # The hook runs the suite through scripts/nextest.sh: each test in a process of its own.
        hook = (REPO / ".githooks" / "pre-commit").read_text(encoding="utf-8")
        self.assertIn("scripts/nextest.sh run --workspace --locked", hook)
        target = tfp.Target(SERVER + "tests/a.rs", "a", "vcon-server", SERVER)
        self.assertEqual(tfp.commands(target, tree({})), ["heavy-run --quick -- scripts/nextest.sh run -p vcon-server --locked --test a"])

    def test_success_a_binary_with_postgres_code_also_runs_with_the_postgres_feature(self):
        hook = (REPO / ".githooks" / "pre-commit").read_text(encoding="utf-8")
        self.assertIn(f"scripts/nextest.sh run -p {tree_package()} --locked --features postgres", hook)
        t, target = t_of({SERVER + "tests/a.rs": '#[cfg(feature = "postgres")]\nmod pg;\n'}, SERVER + "tests/a.rs")
        self.assertEqual(tfp.commands(target, t), [
            "heavy-run --quick -- scripts/nextest.sh run -p vcon-server --locked --test a",
            "heavy-run --quick -- scripts/nextest.sh run -p vcon-server --locked --features postgres --test a",
        ])

    def test_success_a_change_the_lib_compiles_also_runs_its_doctests_as_the_hook_does(self):
        hook = (REPO / ".githooks" / "pre-commit").read_text(encoding="utf-8")
        self.assertIn("cargo test --workspace --locked --quiet --doc", hook)
        files = {"crates/vcon/src/lib.rs": "pub mod cli;\n", "crates/vcon/src/cli.rs": "pub fn f() {}\n"}
        jobs = select(files, ["crates/vcon/src/cli.rs"])
        self.assertEqual([j.command for j in jobs if j.name == "vcon doctests"],
                         ["heavy-run --quick -- cargo test -p vcon --locked --quiet --doc"])

    def test_failure_a_binary_without_postgres_code_runs_once(self):
        t, target = t_of({SERVER + "tests/a.rs": PLAIN}, SERVER + "tests/a.rs")
        self.assertEqual(len(tfp.commands(target, t)), 1)

    def test_failure_a_production_read_dir_does_not_make_the_lib_whole_tree(self):
        t, lib = t_of({"crates/vcon/src/lib.rs": PRODUCTION_WALK}, "crates/vcon/src/lib.rs")
        self.assertEqual(tfp.marks(lib, t), [])

    def test_failure_a_lib_that_walks_src_does_not_run_for_a_docs_change(self):
        files = {"crates/vcon/src/lib.rs": LIB_WALK, "crates/vcon/src/cli.rs": ""}
        self.assertNotIn("vcon lib", names(select(files, ["docs/install.md"])))

    def test_edge_a_test_module_in_its_own_file_is_test_code_throughout(self):
        files = {
            "crates/vcon/src/lib.rs": "#[cfg(test)]\nmod walk_tests;\n",
            "crates/vcon/src/walk_tests.rs": 'fn l() { std::fs::read_dir(root.join("../../docs")); }\n',
        }
        t, lib = t_of(files, "crates/vcon/src/lib.rs")
        self.assertEqual(tfp.marks(lib, t), ["read_dir"])

    def test_edge_a_commented_cfg_test_does_not_start_the_test_code(self):
        text = "// #[cfg(test)] would start the tests here\n" + PRODUCTION_WALK.split("#[cfg(test)]")[0]
        t, lib = t_of({"crates/vcon/src/lib.rs": text}, "crates/vcon/src/lib.rs")
        self.assertEqual(tfp.marks(lib, t), [])


class Selection(unittest.TestCase):
    def test_success_a_src_walker_is_selected_for_a_src_change(self):
        self.assertIn("a", names(select({SERVER + "tests/a.rs": SRC_WALK}, [SERVER + "src/cli.rs"])))

    def test_success_a_site_walker_is_selected_for_a_page_change(self):
        self.assertIn("a", names(select({SERVER + "tests/a.rs": SITE_WALK}, ["website/content/docs/a.md"])))

    def test_success_a_test_that_names_the_path_is_selected(self):
        text = 'fn f() { std::fs::read_to_string(root.join("../../docs/install.md")); }\n'
        jobs = select({SERVER + "tests/a.rs": text}, ["docs/install.md"])
        self.assertEqual([j.reasons for j in jobs if j.name == "a"], [["names docs/install.md"]])

    def test_success_a_touched_support_file_selects_the_binaries_that_include_it(self):
        files = {
            SERVER + "tests/a.rs": '#[path = "support/run.rs"]\nmod run;\n',
            SERVER + "tests/b.rs": PLAIN,
            SERVER + "tests/support/run.rs": "pub fn run() {}\n",
        }
        got = names(select(files, [SERVER + "tests/support/run.rs"]))
        self.assertEqual(("a" in got, "b" in got), (True, False))

    def test_success_a_touched_test_file_selects_itself(self):
        jobs = select({SERVER + "tests/a.rs": PLAIN}, [SERVER + "tests/a.rs"])
        self.assertEqual([j.reasons for j in jobs if j.name == "a"], [["is the test binary"]])

    def test_success_the_command_form_is_one_binary_per_quick_job(self):
        jobs = select({SERVER + "tests/a.rs": SRC_WALK}, [SERVER + "src/cli.rs"])
        self.assertEqual([j.command for j in jobs if j.name == "a"],
                         ["heavy-run --quick -- scripts/nextest.sh run -p vcon-server --locked --test a"])

    def test_success_a_script_test_runs_for_the_script_it_loads(self):
        files = {"scripts/test_check_style.py": 'spec_from_file_location("s", HERE / "check_style.py")\n'}
        jobs = select(files, ["scripts/check_style.py"])
        self.assertEqual([(j.command, j.reasons) for j in jobs if j.name == "scripts/test_check_style.py"],
                         [("heavy-run --quick -- python3 -I scripts/test_check_style.py",
                           ["names scripts/check_style.py"])])

    def test_success_a_shell_test_of_the_hook_runs_for_the_script_it_names(self):
        files = {"scripts/test-retry.sh": 'run "$here/retry.sh" 3 false\n'}
        jobs = select(files, ["scripts/retry.sh"])
        self.assertEqual([j.command for j in jobs if j.name == "scripts/test-retry.sh"],
                         ["heavy-run --quick -- scripts/test-retry.sh"])

    def test_success_a_script_test_that_walks_the_site_runs_for_a_page_change(self):
        files = {"scripts/test_check_style.py": 'for p in (HERE.parent / "website" / "content").rglob("*.md"): pass\n'}
        self.assertIn("scripts/test_check_style.py", names(select(files, ["website/content/docs/a.md"])))

    def test_success_a_script_test_that_imports_a_walking_module_walks(self):
        files = {
            "scripts/test_check_style.py": "from style_walk import run\nrun()\n",
            "scripts/style_walk.py": "import os\ndef run():\n    for d, _, fs in os.walk(ROOT_DIR): pass\n",
        }
        t, target = t_of(files, "scripts/test_check_style.py", tracked=["scripts/style_walk.py"])
        self.assertEqual(tfp.marks(target, t), ["os.walk"])

    def test_success_a_script_loaded_by_its_bare_name_is_followed(self):
        files = {
            "scripts/test_check_style.py": 'check = load("style_walk")\n',
            "scripts/style_walk.py": "import os\ndef run():\n    for d, _, fs in os.walk(ROOT_DIR): pass\n",
        }
        t, target = t_of(files, "scripts/test_check_style.py", tracked=["scripts/style_walk.py"])
        self.assertEqual(tfp.marks(target, t), ["os.walk"])

    def test_edge_a_script_that_a_loaded_script_loads_is_followed(self):
        files = {
            "scripts/test_check_style.py": 'spec_from_file_location("a", HERE / "check_style.py")\n',
            "scripts/check_style.py": 'spec_from_file_location("b", HERE / "style_walk.py")\n',
            "scripts/style_walk.py": "import os\ndef run():\n    for d, _, fs in os.walk(ROOT_DIR): pass\n",
        }
        t, target = t_of(files, "scripts/test_check_style.py", tracked=["scripts/style_walk.py"])
        self.assertEqual(tfp.marks(target, t), ["os.walk"])

    def test_success_needed_docs_scenarios_print_the_main_slot_push_run(self):
        jobs = select({}, ["docs/install.md"], scenarios_needed=lambda paths: ["foreground", "systemd"])
        self.assertEqual(
            [(j.kind, j.command, j.reasons) for j in jobs if j.kind == "docs-scenarios"],
            [("docs-scenarios",
              'heavy-run -- python3 -I scripts/docs_test.py push "$(git rev-parse HEAD)" --since origin/main',
              ["the push runs these how-to scenarios: foreground, systemd (main slot, before git push)"])],
        )

    def test_success_two_paths_give_one_job_with_both_reasons(self):
        text = 'fn f() { r("../../docs/install.md"); r("../../README.md"); }\n'
        jobs = select({SERVER + "tests/a.rs": text}, ["docs/install.md", "README.md"])
        self.assertEqual([j.reasons for j in jobs if j.name == "a"], [["names docs/install.md", "names README.md"]])

    def test_failure_a_src_walker_is_not_selected_for_a_page_change(self):
        self.assertNotIn("a", names(select({SERVER + "tests/a.rs": SRC_WALK}, ["website/content/docs/a.md"])))

    def test_failure_a_script_test_the_hook_does_not_run_is_not_selected(self):
        files = {"scripts/test_unhooked.py": 'spec_from_file_location("s", HERE / "check_style.py")\n'}
        self.assertNotIn("scripts/test_unhooked.py", names(select(files, ["scripts/check_style.py"])))

    def test_failure_no_scenario_needed_prints_no_push_run(self):
        self.assertEqual([j for j in select({}, ["README.md"]) if j.kind == "docs-scenarios"], [])

    def test_edge_a_touched_directory_selects_the_walker_of_it(self):
        self.assertIn("a", names(select({SERVER + "tests/a.rs": SRC_WALK}, [SERVER + "src"])))

    def test_edge_a_deleted_path_still_selects(self):
        self.assertIn("a", names(select({SERVER + "tests/a.rs": SRC_WALK}, [SERVER + "src/removed.rs"])))

    def test_edge_everywhere_lists_every_whole_tree_binary(self):
        files = {SERVER + "tests/a.rs": SRC_WALK, SERVER + "tests/b.rs": PLAIN}
        got = names(select(files, ["README.md"], everywhere=True))
        self.assertEqual(("a" in got, "b" in got), (True, False))

    def test_edge_the_crate_relative_name_of_a_touched_file_is_a_name_match(self):
        text = 'fn f() { std::fs::read_to_string(root.join("README.md")); }\n'
        jobs = select({SERVER + "tests/a.rs": text}, [SERVER + "README.md"])
        self.assertEqual([j.reasons for j in jobs if j.name == "a"], [[f"names {SERVER}README.md"]])


ROOT_WALK_TEXT = "fn f() { for e in std::fs::read_dir(repo()) {} }\n"
NAMES_CLI = 'fn f() { r("src/cli.rs"); }\n'


class Order(unittest.TestCase):
    def test_success_whole_tree_gates_that_run_everywhere_come_first(self):
        files = {SERVER + "tests/a_named.rs": NAMES_CLI, SERVER + "tests/b_src.rs": SRC_WALK,
                 SERVER + "tests/z_root.rs": ROOT_WALK_TEXT}
        jobs = [j for j in select(files, [SERVER + "src/cli.rs"]) if j.kind == "cargo"]
        self.assertEqual([j.name for j in jobs], ["z_root", "b_src", "a_named"])

    def test_success_the_docs_scenarios_come_last(self):
        files = {SERVER + "tests/a.rs": SRC_WALK}
        jobs = select(files, [SERVER + "src/cli.rs"], scenarios_needed=lambda p: ["foreground"])
        self.assertEqual((jobs[0].name, jobs[-1].kind), ("a", "docs-scenarios"))

    def test_success_chain_format_stops_at_the_first_failure(self):
        files = {SERVER + "tests/a.rs": SRC_WALK, SERVER + "tests/z.rs": ROOT_WALK_TEXT}
        _, out, _ = run_main(["--format", "chain", SERVER + "src/cli.rs"], tree=tree(files))
        self.assertEqual(out.splitlines(), [
            "heavy-run --quick -- scripts/nextest.sh run -p vcon-server --locked --test z && \\",
            "heavy-run --quick -- scripts/nextest.sh run -p vcon-server --locked --test a",
        ])

    def test_success_json_holds_each_jobs_phase(self):
        files = {SERVER + "tests/a.rs": SRC_WALK, SERVER + "tests/z.rs": ROOT_WALK_TEXT}
        _, out, _ = run_main(["--format", "json", SERVER + "src/cli.rs"], tree=tree(files))
        self.assertEqual([(d["name"], d["phase"]) for d in json.loads(out)], [("z", 1), ("a", 2)])

    def test_failure_chain_format_with_no_job_prints_nothing(self):
        rc, out, _ = run_main(["--format", "chain", "docs/install.md"], tree=tree({SERVER + "tests/a.rs": SRC_WALK}))
        self.assertEqual((rc, out), (0, ""))

    def test_edge_a_job_both_everywhere_and_named_is_in_the_first_phase(self):
        files = {SERVER + "tests/a.rs": ROOT_WALK_TEXT.replace("{}", '{ r("src/cli.rs"); }'),
                 SERVER + "tests/b.rs": SRC_WALK}
        jobs = [j for j in select(files, [SERVER + "src/cli.rs"]) if j.kind == "cargo"]
        self.assertEqual([(j.name, j.phase) for j in jobs], [("a", 1), ("b", 2)])


class Census(unittest.TestCase):
    def test_failure_check_finds_a_walk_by_a_crate_the_rule_does_not_know(self):
        text = 'fn f() { ignore::WalkBuilder::new(root.join("src")).build(); }\n'
        problems = tfp.check(tree({SERVER + "tests/a.rs": text}), exempt={})
        self.assertTrue(len(problems) == 1 and "ignore::" in problems[0], problems)

    def test_failure_check_finds_a_walk_reached_by_an_include_the_rule_cannot_follow(self):
        files = {
            SERVER + "tests/a.rs": 'include!(concat!("support/", "walk.rs"));\nfn f() { walk::go(); }\n',
            SERVER + "tests/support/walk.rs": 'pub fn go() { std::fs::read_dir(root.join("src")); }\n',
        }
        problems = tfp.check(tree(files), exempt={})
        self.assertTrue(len(problems) == 1 and SERVER + "tests/a.rs" in problems[0], problems)

    def test_failure_check_finds_a_unit_test_walk_the_rule_does_not_know(self):
        text = PRODUCTION_WALK.replace("fn adds()", 'fn walks() { jwalk::Walker::new(root); }\n    fn adds()')
        problems = tfp.check(tree({"crates/vcon/src/lib.rs": text}), exempt={})
        self.assertTrue(len(problems) == 1 and problems[0].startswith("crates/vcon/src/lib.rs"), problems)

    def test_failure_check_finds_a_script_test_walk_through_an_import(self):
        files = {
            # A name the rule cannot follow: no literal, no import statement.
            "scripts/test_check_style.py": "spec.loader.exec_module(style_walk)\nstyle_walk.run()\n",
            "scripts/style_walk.py": "import os\ndef run():\n    for d, _, fs in os.walk(ROOT_DIR): pass\n",
        }
        problems = tfp.check(tree(files), exempt={})
        self.assertTrue(len(problems) == 1 and "scripts/test_check_style.py" in problems[0], problems)

    def test_failure_check_refuses_an_exemption_the_census_does_not_need(self):
        problems = tfp.check(tree({SERVER + "tests/a.rs": PLAIN}), exempt={SERVER + "tests/a.rs": "reason"})
        self.assertTrue(len(problems) == 1 and "remove" in problems[0], problems)

    def test_failure_check_refuses_an_exemption_of_a_binary_the_rule_finds(self):
        problems = tfp.check(tree({SERVER + "tests/a.rs": SRC_WALK}), exempt={SERVER + "tests/a.rs": "reason"})
        self.assertTrue(len(problems) == 1 and "the rule finds it" in problems[0], problems)

    def test_failure_check_refuses_an_exemption_of_no_test_binary(self):
        problems = tfp.check(tree({}), exempt={SERVER + "tests/gone.rs": "reason"})
        self.assertTrue(len(problems) == 1 and "no test binary" in problems[0], problems)


class CommandLine(unittest.TestCase):
    def test_success_names_format_prints_one_name_a_line(self):
        rc, out, _ = run_main(["--format", "names", SERVER + "src/cli.rs"], tree=tree({SERVER + "tests/a.rs": SRC_WALK}))
        self.assertEqual((rc, "a" in out.splitlines()), (0, True))

    def test_success_commands_format_prints_bare_commands(self):
        rc, out, _ = run_main(["--format", "commands", SERVER + "src/cli.rs"],
                              tree=tree({SERVER + "tests/a.rs": SRC_WALK}))
        self.assertIn("heavy-run --quick -- scripts/nextest.sh run -p vcon-server --locked --test a", out.splitlines())

    def test_success_text_format_adds_the_reason_as_a_shell_comment(self):
        _, out, _ = run_main([SERVER + "src/cli.rs"], tree=tree({SERVER + "tests/a.rs": SRC_WALK}))
        self.assertIn(
            f"heavy-run --quick -- scripts/nextest.sh run -p vcon-server --locked --test a  # walks {SERVER}src (read_dir)",
            out.splitlines(),
        )

    def test_success_json_format_is_a_list_of_jobs(self):
        _, out, _ = run_main(["--format", "json", SERVER + "src/cli.rs"], tree=tree({SERVER + "tests/a.rs": SRC_WALK}))
        self.assertEqual([d for d in json.loads(out) if d["name"] == "a"], [{
            "kind": "cargo", "name": "a", "file": SERVER + "tests/a.rs",
            "command": "heavy-run --quick -- scripts/nextest.sh run -p vcon-server --locked --test a",
            "reasons": [f"walks {SERVER}src (read_dir)"], "phase": 2,
        }])

    def test_failure_no_paths_is_a_usage_error(self):
        self.assertEqual(run_main([], tree=tree({}))[0], 2)

    def test_failure_a_path_outside_the_repository_is_refused(self):
        rc, _, err = run_main(["/etc/passwd"], tree=tree({}))
        self.assertEqual((rc, "outside the repository" in err), (2, True))

    def test_failure_an_unknown_format_is_a_usage_error(self):
        self.assertEqual(run_main(["--format", "yaml", "README.md"], tree=tree({}))[0], 2)

    def test_failure_check_exits_one_and_names_the_binary(self):
        text = 'fn f() { jwalk::Walker::new(root.join("src")); }\n'
        rc, _, err = run_main(["--check"], tree=tree({SERVER + "tests/a.rs": text}), exempt={})
        self.assertEqual((rc, SERVER + "tests/a.rs" in err), (1, True))

    def test_edge_an_absolute_path_inside_the_repository_is_made_relative(self):
        rc, out, _ = run_main(["--format", "names", str(REPO / SERVER / "src" / "cli.rs")],
                              tree=tree({SERVER + "tests/a.rs": SRC_WALK}))
        self.assertEqual((rc, "a" in out.splitlines()), (0, True))

    def test_edge_a_relative_path_is_read_from_the_working_directory(self):
        files = {SERVER + "tests/a.rs": SRC_WALK}
        _, from_src, _ = run_main(["--format", "names", "src/cli.rs"], tree=tree(files), cwd=REPO / SERVER)
        _, from_scripts, _ = run_main(["--format", "names", SERVER + "src/cli.rs"], tree=tree(files),
                                      cwd=REPO / "scripts")
        self.assertEqual((from_src.splitlines(), from_scripts), (["a"], ""))

    def test_edge_a_dot_slash_path_is_normalized(self):
        self.assertEqual(tfp.normalize("./README.md", REPO, REPO), "README.md")

    def test_edge_no_job_at_all_prints_nothing(self):
        rc, out, _ = run_main(["--format", "names", "docs/install.md"], tree=tree({SERVER + "tests/a.rs": SRC_WALK}))
        self.assertEqual((rc, out), (0, ""))


STEP_HOOK = """\
#!/usr/bin/env bash
# python3 -I scripts/commented.py is no step
install -m 0755 scripts/cc-lld.sh "$CARGO_TARGET_DIR/cc-lld.sh"
python3 -I scripts/check_style.py --walk
python3 -I scripts/clippy_features.py check
python3 -I scripts/clippy_features.py run --quiet
scripts/rustfmt.sh --check
scripts/nextest.sh run --workspace --locked
python3 -I scripts/check_plain.py
"""
STYLE = 'import tree_files\nroot, paths = tree_files.listed(walk)\nrs = [p for p in paths if p.endswith(".rs")]\n'
RUSTFMT = "#!/bin/sh\ngit ls-files -z -- '*.rs' | xargs -0 rustfmt --check\n"


def step_tree(extra=None):
    files = {".githooks/pre-commit": STEP_HOOK, "scripts/check_style.py": STYLE, "scripts/rustfmt.sh": RUSTFMT,
             "scripts/clippy_features.py": "print('features')\n", "scripts/check_plain.py": "print('x')\n",
             "scripts/tree_files.py": "def listed(walk): pass\n", **(extra or {})}
    return tree(files)


def step_jobs(paths, extra=None):
    return [j for j in tfp.select(step_tree(extra), paths, scenarios_needed=lambda p: []) if j.kind == "hook-step"]


class HookSteps(unittest.TestCase):
    def test_success_a_changed_rs_file_runs_the_style_and_format_steps(self):
        got = {j.name: j.command for j in step_jobs([SERVER + "src/cli.rs"])}
        self.assertEqual((got.get("scripts/check_style.py"), got.get("scripts/rustfmt.sh")),
                         ("heavy-run --quick -- python3 -I scripts/check_style.py",
                          "heavy-run --quick -- scripts/rustfmt.sh --check"))

    def test_success_a_touched_step_script_runs_its_step(self):
        got = [j for j in step_jobs(["scripts/check_plain.py"]) if j.name == "scripts/check_plain.py"]
        self.assertEqual([j.reasons for j in got], [["is the hook step"]])

    def test_success_a_subcommand_is_part_of_the_step(self):
        commands = [s.command for s in tfp.hook_steps(step_tree())]
        self.assertIn("python3 -I scripts/clippy_features.py check", commands)

    def test_failure_a_runner_of_the_hook_is_no_step(self):
        commands = [s.command for s in tfp.hook_steps(step_tree())]
        self.assertFalse([c for c in commands if "nextest.sh" in c or "clippy_features.py run" in c], commands)

    def test_failure_check_refuses_a_runner_entry_the_hook_no_longer_runs(self):
        problems = tfp.check(step_tree({".githooks/pre-commit": "python3 -I scripts/check_plain.py\n"}), exempt={})
        self.assertTrue(any("RUNNERS names scripts/nextest.sh" in p for p in problems), problems)

    def test_failure_a_step_with_no_walk_does_not_run_for_an_unrelated_change(self):
        self.assertNotIn("scripts/check_plain.py", {j.name for j in step_jobs([SERVER + "src/cli.rs"])})

    def test_success_a_cargo_fmt_step_runs_for_a_changed_rs_file_only(self):
        extra = {"scripts/rustfmt.sh": '#!/bin/sh\ncd "$root"\ncargo "+$toolchain" fmt --all ${1:+"$1"}\n'}
        rs = {j.name: j.reasons for j in step_jobs(["crates/vcon/src/lib.rs"], extra)}
        md = {j.name for j in step_jobs(["docs/install.md"], extra)}
        self.assertEqual((rs.get("scripts/rustfmt.sh"), "scripts/rustfmt.sh" in md),
                         (["formats every .rs file (cargo fmt)"], False))

    def test_edge_a_script_test_the_hook_runs_is_a_test_not_a_step(self):
        self.assertNotIn("scripts/test_check_style.py", {s.script for s in tfp.hook_steps(tree({}))})

    def test_failure_check_finds_a_hook_step_walk_the_rule_does_not_know(self):
        extra = {"scripts/check_plain.py": 'import pathlib\nfor d, _, fs in pathlib.Path("src").walk():\n    pass\n'}
        problems = tfp.check(step_tree(extra), exempt={})
        self.assertTrue(any(p.startswith("scripts/check_plain.py: the census finds") for p in problems), problems)

    def test_edge_an_argument_that_names_a_script_is_no_step(self):
        self.assertNotIn("scripts/cc-lld.sh", {s.script for s in tfp.hook_steps(step_tree())})

    def test_edge_the_walk_flag_is_dropped_in_a_checkout(self):
        style = [s for s in tfp.hook_steps(step_tree()) if s.script == "scripts/check_style.py"]
        self.assertEqual([s.command for s in style], ["python3 -I scripts/check_style.py"])


class HookCopy(unittest.TestCase):
    def test_edge_a_copy_with_no_git_directory_is_read_by_walking_it(self):
        # The commit hook runs the script tests in `git checkout-index` output, which has no .git.
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            (root / "crates" / "vcon" / "tests").mkdir(parents=True)
            (root / "crates" / "vcon" / "tests" / "a.rs").write_text(SRC_WALK, encoding="utf-8")
            (root / "target").mkdir()
            (root / "target" / "junk.rs").write_text("", encoding="utf-8")
            t = tfp.Tree.of_repo(root)
            self.assertEqual((sorted(t.tracked), t.read("crates/vcon/tests/a.rs")),
                             (["crates/vcon/tests/a.rs"], SRC_WALK))


class TreeKind(unittest.TestCase):
    def test_success_the_server_manifest_is_the_server_tree(self):
        self.assertTrue(is_server_tree(package_of('[package]\nname = "vcon-server"\nversion = "1.0.0"\n')))

    def test_success_the_fixture_manifest_is_not_the_server_tree(self):
        self.assertFalse(is_server_tree(package_of('[package]\nname = "hook-fixture"\n')))

    def test_failure_another_package_name_is_an_error(self):
        with self.assertRaisesRegex(ValueError, "names 'vcon-srv': neither vcon-server nor hook-fixture"):
            is_server_tree("vcon-srv")

    def test_edge_the_tree_this_file_is_in_names_one_of_the_two(self):
        self.assertIn(tree_package(), ("vcon-server", FIXTURE_PACKAGE))

    def test_edge_a_tree_with_the_servers_tests_is_the_server_tree(self):
        # RealTree is skipped only where the server's tests are absent.
        if (REPO / SERVER / "tests" / "stderr_rule.rs").is_file():
            self.assertTrue(is_server_tree(tree_package()), "the server's tree reads as the fixture")


@unittest.skipUnless(is_server_tree(tree_package()),
                     "the hook tests' fixture (scripts/test-hooks.sh) is not the server's tree")
class RealTree(unittest.TestCase):
    def test_success_each_listed_walker_is_whole_tree(self):
        for name in ("stderr_rule", "docs_pages", "licence_files"):
            target = tfp.target_of(real(), f"{SERVER}tests/{name}.rs")
            self.assertTrue(tfp.marks(target, real()), f"{name} is not found by the rule")

    def test_success_a_server_source_change_selects_its_lib(self):
        self.assertIn("vcon-server lib", names(tfp.select(real(), [SERVER + "src/config.rs"],
                                                         scenarios_needed=lambda p: [])))

    def test_success_a_server_source_change_runs_the_style_and_format_steps(self):
        jobs = tfp.select(real(), [SERVER + "src/config.rs"], scenarios_needed=lambda p: [])
        steps = {j.name for j in jobs if j.kind == "hook-step"}
        self.assertTrue({"scripts/check_style.py", "scripts/rustfmt.sh"} <= steps, steps)

    def test_success_the_census_of_the_real_tree_agrees_with_the_rule(self):
        self.assertEqual(tfp.check(real()), [])

    def test_success_every_script_test_the_hook_runs_exists(self):
        self.assertTrue(all(real().read(p) is not None for p in tfp.hook_script_tests(real())))


if __name__ == "__main__":
    unittest.main()
