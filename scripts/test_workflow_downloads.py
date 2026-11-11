"""Tests scripts/check_workflow_downloads.py: every step in the workflows,
the composite actions and the shell scripts that downloads a toolchain or a
tool does so with a bounded retry.

Run with: python3 -I scripts/test_workflow_downloads.py
"""

import importlib.util
import pathlib
import sys
import tempfile
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("check_workflow_downloads", HERE / "check_workflow_downloads.py")
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


def found(text, path=".github/workflows/x.yml"):
    return check.unretried(path, text)


class Rustup(unittest.TestCase):
    def test_a_bare_toolchain_install_is_found(self):
        self.assertEqual(found("        run: rustup toolchain install\n"), [".github/workflows/x.yml:1: rustup without scripts/retry.sh"])

    def test_component_and_target_adds_are_found(self):
        text = "          rustup component add llvm-tools\n          rustup target add x86_64-unknown-linux-musl\n"
        self.assertEqual(len(found(text)), 2)

    def test_one_through_the_retry_script_passes(self):
        self.assertEqual(found("        run: scripts/retry.sh rustup toolchain install\n"), [])
        self.assertEqual(found("        (cd fuzz && ../scripts/retry.sh rustup toolchain install)\n"), [])
        self.assertEqual(found('  exec "$root/scripts/retry.sh" rustup toolchain install "$t"\n', "scripts/x.sh"), [])

    def test_rustup_run_downloads_nothing(self):
        self.assertEqual(found('if ! rustup run "$toolchain" rustfmt --version; then\n', "scripts/x.sh"), [])


class Npm(unittest.TestCase):
    def test_npm_ci_without_retry_is_found(self):
        self.assertEqual(found("          npm ci\n"), [".github/workflows/x.yml:1: npm without scripts/retry.sh"])

    def test_npm_ci_through_the_retry_script_passes(self):
        self.assertEqual(found("          (cd a && ../scripts/retry.sh npm ci --ignore-scripts)\n"), [])


class Playwright(unittest.TestCase):
    def test_a_browser_install_without_retry_is_found(self):
        self.assertEqual(found("          npx playwright install --with-deps --only-shell chromium\n"), [".github/workflows/x.yml:1: playwright install without scripts/retry.sh"])

    def test_a_browser_install_through_the_retry_script_passes(self):
        self.assertEqual(found("          ../scripts/retry.sh npx playwright install --with-deps chromium\n"), [])


class Dnf(unittest.TestCase):
    def test_a_bare_dnf_install_is_found(self):
        self.assertEqual(
            found("        run: dnf install -y git-core\n"),
            [".github/workflows/x.yml:1: dnf install outside scripts/dnf_install.py, which tries a second source"],
        )
        self.assertEqual(len(found("            yum install -y jq\n            microdnf install -y jq\n")), 2)

    def test_dnf_through_the_rule_passes(self):
        self.assertEqual(found("        run: python3 -I scripts/dnf_install.py git-core\n"), [])
        self.assertEqual(found("python3 -I scripts/dnf_install.py gcc make perl-core >&2\n", "scripts/x.sh"), [])

    def test_dnf_through_the_retry_script_cleaning_metadata_is_found(self):
        # The rule until 2026-11-11, when five such tries read the same
        # mirror's bad repomd.xml: cleaning does not change the mirror.
        self.assertEqual(len(found("          RETRY_BETWEEN='dnf clean metadata' scripts/retry.sh dnf install -y jq\n")), 1)
        self.assertEqual(len(found("RETRY_BETWEEN='dnf clean metadata' scripts/retry.sh \\\n  dnf install -y jq\n", "scripts/x.sh")), 1)

    def test_dnf_through_the_retry_script_without_cleaning_is_found(self):
        self.assertEqual(len(found("          scripts/retry.sh dnf install -y jq\n")), 1)

    def test_cleaning_once_without_a_retry_is_found(self):
        self.assertEqual(len(found("          dnf clean metadata && dnf install -y jq\n")), 1)

    def test_an_inline_loop_that_cleans_metadata_is_found(self):
        # The step that failed: before the checkout, where no script exists.
        line = "        run: for i in 1 2 3 4 5; do dnf install -y git-core && break; [ \"$i\" = 5 ] && exit 1; dnf clean metadata; sleep $((i * 10)); done\n"
        self.assertEqual(len(found(line)), 1)

    def test_an_install_with_options_before_the_word_is_found(self):
        self.assertEqual(len(found("          dnf -y --setopt=install_weak_deps=False install jq\n")), 1)

    def test_dnf_commands_that_install_nothing_pass(self):
        self.assertEqual(found("          dnf clean metadata\n          dnf --version\n          rpm -q dnf\n"), [])

    def test_a_step_that_names_the_rule_and_also_runs_dnf_is_found(self):
        self.assertEqual(len(found("          python3 -I scripts/dnf_install.py jq || dnf install -y jq\n")), 1)

    def test_the_rule_is_counted_where_it_is_called(self):
        text = "      - run: python3 -I scripts/dnf_install.py git-core\n      # python3 -I scripts/dnf_install.py git-core\n      - name: python3 -I scripts/dnf_install.py jq\n"
        self.assertEqual(check.dnf_rule_calls(text), 1)
        self.assertEqual(check.dnf_rule_calls("      - run: echo dnf_install\n"), 0)

    def test_a_tree_with_fewer_calls_of_the_rule_than_the_floor_fails(self):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            (root / ".github/workflows").mkdir(parents=True)
            (root / "scripts").mkdir()
            (root / ".github/workflows/release.yml").write_text("    steps:\n      - run: python3 -I scripts/dnf_install.py git-core\n", encoding="utf-8")
            problems = check.check(root)
            (root / "scripts/x.sh").write_text("python3 -I scripts/dnf_install.py gcc\n", encoding="utf-8")
            enough = check.check(root)
        self.assertTrue(any("found 1 installs through scripts/dnf_install.py; expected at least 2" in p for p in problems), problems)
        self.assertFalse(any("scripts/dnf_install.py" in p for p in enough), enough)

    def test_the_real_tree_calls_the_rule_at_each_known_place(self):
        root = HERE.parent
        calls = {p.relative_to(root).as_posix(): check.dnf_rule_calls(p.read_text(encoding="utf-8")) for p in check.files(root)}
        self.assertEqual(
            {k: v for k, v in calls.items() if v},
            {".github/workflows/release.yml": 1, "scripts/release-build-env.sh": 1},
        )
        # Both build jobs prepare their container with that script.
        for name in ("release.yml", "reproducible.yml"):
            self.assertIn("run: bash scripts/release-build-env.sh\n", (root / ".github/workflows" / name).read_text(encoding="utf-8"), name)
        self.assertTrue((root / "scripts/dnf_install.py").is_file())

    def test_the_whole_check_reports_a_run_step_before_a_container_job_s_checkout(self):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            (root / ".github/workflows").mkdir(parents=True)
            text = "jobs:\n  build:\n" + self.IN_CONTAINER + "    steps:\n      - run: echo hello\n" + self.CHECKOUT
            (root / ".github/workflows/release.yml").write_text(text, encoding="utf-8")
            problems = check.check(root)
        self.assertIn(".github/workflows/release.yml:6: a run step before the job's checkout: no repository script exists there", problems)

    def test_no_job_runs_a_command_before_its_checkout_in_an_almalinux_container(self):
        # A step before the checkout cannot use a repository script, so it
        # could only install from one source.
        root = HERE.parent
        for name in ("release.yml", "reproducible.yml"):
            text = (root / ".github/workflows" / name).read_text(encoding="utf-8")
            self.assertEqual(check.runs_before_checkout(name, text), [])

    CHECKOUT = "      - uses: actions/checkout@0000000000000000000000000000000000000000 # v7\n"
    IN_CONTAINER = "    container:\n      image: almalinux:9\n"

    def test_a_run_step_before_the_checkout_is_found(self):
        text = "jobs:\n  build:\n" + self.IN_CONTAINER + "    steps:\n      - name: Install git\n        run: echo hello\n" + self.CHECKOUT + "      - run: echo after\n"
        self.assertEqual(check.runs_before_checkout("x.yml", text), ["x.yml:6: a run step before the job's checkout: no repository script exists there"])

    def test_a_container_named_on_one_line_or_below_the_steps_counts(self):
        early = "    steps:\n      - run: echo hello\n" + self.CHECKOUT
        self.assertEqual(len(check.runs_before_checkout("x.yml", "jobs:\n  build:\n    container: almalinux:9\n" + early)), 1)
        self.assertEqual(len(check.runs_before_checkout("x.yml", "jobs:\n  build:\n" + early + self.IN_CONTAINER)), 1)
        # A comment at the margin between the two keys ends nothing.
        self.assertEqual(len(check.runs_before_checkout("x.yml", "jobs:\n  build:\n    container: almalinux:9\n# why\n" + early)), 1)

    def test_a_job_on_the_runner_s_own_system_may_run_commands_first(self):
        text = "jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hello\n" + self.CHECKOUT
        self.assertEqual(check.runs_before_checkout("x.yml", text), [])

    def test_another_job_s_container_does_not_count(self):
        text = "jobs:\n  a:\n" + self.IN_CONTAINER + "    steps:\n" + self.CHECKOUT + "  b:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo b\n" + self.CHECKOUT
        self.assertEqual(check.runs_before_checkout("x.yml", text), [])

    def test_a_container_named_in_a_comment_or_under_services_does_not_count(self):
        text = "jobs:\n  b:\n    # container: almalinux:9\n    services:\n      db:\n        container: x\n    steps:\n      - run: echo b\n" + self.CHECKOUT
        self.assertEqual(check.runs_before_checkout("x.yml", text), [])

    def test_a_job_with_no_checkout_may_run_commands(self):
        text = "jobs:\n  build:\n" + self.IN_CONTAINER + "    steps:\n      - run: echo hello\n"
        self.assertEqual(check.runs_before_checkout("x.yml", text), [])

    def test_a_uses_step_before_the_checkout_passes(self):
        text = "jobs:\n  build:\n" + self.IN_CONTAINER + "    steps:\n      - uses: actions/setup-node@0000000000000000000000000000000000000000 # v6\n" + self.CHECKOUT + "      - run: echo after\n"
        self.assertEqual(check.runs_before_checkout("x.yml", text), [])

    def test_a_run_step_between_two_checkouts_passes(self):
        # The build jobs: files first, then git through the rule, then the
        # checkout that keeps .git.
        text = "jobs:\n  build:\n" + self.IN_CONTAINER + "    steps:\n" + self.CHECKOUT + "      - run: python3 -I scripts/dnf_install.py git-core\n" + self.CHECKOUT
        self.assertEqual(check.runs_before_checkout("x.yml", text), [])

    def test_each_job_s_steps_are_judged_on_their_own(self):
        text = (
            "jobs:\n  a:\n" + self.IN_CONTAINER + "    steps:\n" + self.CHECKOUT + "      - run: echo a\n"
            "  b:\n" + self.IN_CONTAINER + "    steps:\n      - run: echo b\n      - run: echo c\n" + self.CHECKOUT
        )
        self.assertEqual([p.split(":")[1] for p in check.runs_before_checkout("x.yml", text)], ["12", "13"])


class Apt(unittest.TestCase):
    def test_apt_get_without_retries_is_found(self):
        self.assertEqual(len(found("            apt-get update -qq\n            apt-get install -y curl\n")), 2)

    def test_apt_get_with_acquire_retries_passes(self):
        self.assertEqual(found("            apt-get -o Acquire::Retries=5 update -qq\n"), [])


class Curl(unittest.TestCase):
    def test_curl_without_retry_is_found(self):
        self.assertEqual(found("          curl -fsSL -o x.tgz https://example.com/x.tgz\n"), [".github/workflows/x.yml:1: curl without --retry"])

    def test_curl_with_retry_passes(self):
        self.assertEqual(found("          curl -fsSL --retry 5 --retry-delay 5 -o x.tgz https://example.com/x\n"), [])

    def test_a_continued_curl_command_is_read_whole(self):
        text = "          status=\"$(curl -sS -o /dev/null \\\n            --retry 5 https://example.com)\"\n"
        self.assertEqual(found(text), [])
        text = "          status=\"$(curl -sS -o /dev/null \\\n            https://example.com)\"\n"
        self.assertEqual(found(text), [".github/workflows/x.yml:1: curl without --retry"])

    def test_a_probe_of_a_local_server_is_not_a_download(self):
        self.assertEqual(found('curl -ksf -o /dev/null "$url/console/" || exit 1\n', "scripts/x.sh"), [])
        self.assertEqual(found("          curl -sf http://127.0.0.1:8080/ || true\n"), [])
        self.assertEqual(found("          curl -sf https://localhost:8443/ || true\n"), [])

    def test_curl_named_in_prose_is_not_a_command(self):
        self.assertEqual(found("      - name: Install what the curl client needs\n"), [])
        self.assertEqual(found("          RUN_CLIENTS=curl bash scripts/run-clients.sh x\n"), [])


class Comments(unittest.TestCase):
    def test_a_comment_line_is_not_a_step(self):
        self.assertEqual(found("      # rustup toolchain install, then npm ci and curl -o x\n"), [])


URL = "https://github.com/o/r/releases/download/v1/t.tar.gz"


def release(text, path=".github/workflows/x.yml"):
    """The release-asset problems in a workflow or action text."""
    return check.release_problems(path, text)[0]


def step(*lines, indent=6):
    """One step of a job's `steps:` list, its lines indented as in ci.yml."""
    pad = " " * indent
    return "    steps:\n" + "".join(f"{pad}{l}\n" for l in lines)


class ReleaseAssets(unittest.TestCase):
    """Every release asset goes through .github/actions/pinned-download or
    scripts/fetch-pinned.sh: cached, retried for up to five minutes, and
    checked against its sha256 on every run."""

    def test_curl_of_a_release_asset_is_found(self):
        text = step("- name: Install", "  run: |", f"    curl -fsSL --retry 5 -o t.tar.gz {URL}")
        self.assertEqual(release(text), [".github/workflows/x.yml:2: a release asset fetched with curl or wget, not through .github/actions/pinned-download or scripts/fetch-pinned.sh"])

    def test_wget_of_a_release_asset_is_found(self):
        self.assertEqual(len(release(step("- name: Install", f"  run: wget -q {URL}"))), 1)

    def test_a_url_in_a_variable_fetched_by_curl_is_found(self):
        text = step("- name: Install", "  env:", f"    U: {URL}", '  run: curl -fsSL --retry 5 -O "$U"')
        self.assertEqual(len(release(text)), 1)

    def test_a_release_asset_named_by_another_command_is_found(self):
        text = step("- name: Install", f"  run: python3 fetch.py {URL}")
        self.assertEqual(release(text), [".github/workflows/x.yml:2: a release asset named outside .github/actions/pinned-download and scripts/fetch-pinned.sh"])

    def test_a_pinned_download_step_passes(self):
        text = step("- name: Get it", "  uses: ./.github/actions/pinned-download", "  with:", "    name: t", f"    url: {URL}")
        self.assertEqual(check.release_problems(".github/workflows/x.yml", text), ([], 1))

    def test_a_run_step_through_fetch_pinned_passes(self):
        text = step("- name: Get it", "  run: |", f'    scripts/fetch-pinned.sh --url "{URL}" --sha256 "$S" --dest t.tar.gz')
        self.assertEqual(check.release_problems(".github/workflows/x.yml", text), ([], 1))

    def test_fetch_pinned_named_by_its_workspace_path_passes(self):
        text = step("- name: Get it", f'  run: bash "$GITHUB_WORKSPACE/scripts/fetch-pinned.sh" --url {URL} --sha256 "$S" --dest t')
        self.assertEqual(release(text), [])

    def test_a_pinned_download_step_that_also_curls_the_asset_is_found(self):
        text = step("- name: Get it", "  uses: ./.github/actions/pinned-download", "  with:", f"    url: {URL}",
                    "- name: Again", f"  run: curl -fsSL --retry 5 -O {URL}")
        self.assertEqual(release(text), [".github/workflows/x.yml:6: a release asset fetched with curl or wget, not through .github/actions/pinned-download or scripts/fetch-pinned.sh"])

    def test_a_commented_release_url_is_not_a_download(self):
        self.assertEqual(release(step("- name: Build", f"  # was: curl {URL}", "  run: cargo build")), [])

    def test_a_step_with_no_release_url_is_out_of_scope(self):
        self.assertEqual(check.release_problems(".github/workflows/x.yml", step("- name: Build", "  run: curl -s http://127.0.0.1:9000/")), ([], 0))

    def test_a_link_to_the_releases_page_is_not_a_download(self):
        self.assertEqual(release(step("- name: Note", "  run: echo https://github.com/o/r/releases/latest")), [])

    def test_steps_of_a_composite_action_are_read_too(self):
        text = "runs:\n  using: composite\n  steps:\n    - name: Install\n      shell: bash\n" + f"      run: curl -fsSL --retry 5 -O {URL}\n"
        self.assertEqual(release(text, ".github/actions/a/action.yml"), [".github/actions/a/action.yml:4: a release asset fetched with curl or wget, not through .github/actions/pinned-download or scripts/fetch-pinned.sh"])

    def test_a_list_nested_in_a_step_does_not_start_a_new_step(self):
        text = step("- name: Get it", "  uses: ./.github/actions/pinned-download", "  with:", "    extra:", "      - one", f"    url: {URL}")
        self.assertEqual(check.release_problems(".github/workflows/x.yml", text), ([], 1))

    def test_each_step_is_judged_on_its_own(self):
        text = step("- name: One", f"  run: scripts/fetch-pinned.sh --url {URL} --sha256 x --dest t",
                    "- name: Two", f"  run: curl -fsSL --retry 5 -O {URL}")
        self.assertEqual(len(release(text)), 1)


def script_release(text, path="scripts/x.sh"):
    return check.script_release_problems(path, text)


class ScriptReleaseAssets(unittest.TestCase):
    def test_a_script_that_curls_a_release_asset_is_found(self):
        text = f'base="{URL}"\ncurl -fsSL --retry 5 -o t "$base"\nscripts/fetch-pinned.sh --url "$base"\n'
        self.assertEqual(script_release(text), ["scripts/x.sh:2: curl or wget in a script that names a release asset; fetch it with scripts/fetch-pinned.sh"])

    def test_a_script_that_names_a_release_asset_and_never_calls_fetch_pinned_is_found(self):
        self.assertEqual(script_release(f'base="{URL}"\necho "$base"\n'), ["scripts/x.sh: names a release asset and never calls scripts/fetch-pinned.sh"])

    def test_a_script_that_hands_the_asset_to_fetch_pinned_passes(self):
        text = f'download="${{NEXTEST_DOWNLOAD:-{URL}}}"\nbash "$root/scripts/fetch-pinned.sh" --url "$download/x" --sha256 "$sha" --dest "$d"\n'
        self.assertEqual(script_release(text), [])

    def test_a_release_base_with_no_trailing_slash_counts(self):
        text = 'download="${NEXTEST_DOWNLOAD:-https://github.com/o/r/releases/download}"\ncurl -fsSL --retry 5 -o t "$download/x"\n'
        self.assertEqual(len(script_release(text)), 2)

    def test_curl_named_in_a_grep_pattern_is_not_a_fetch(self):
        text = f'want="url={URL}"\nif grep -qE "\\b(curl|wget)\\b" x.sh; then echo; fi\nbash scripts/fetch-pinned.sh --url "$u"\n'
        self.assertEqual(script_release(text), [])

    def test_curl_after_a_shell_keyword_is_a_fetch(self):
        text = f'u="{URL}"\nif true; then curl -fsSL -o t "$u"; fi\nbash scripts/fetch-pinned.sh --url "$u"\n'
        self.assertEqual(len(script_release(text)), 1)

    def test_a_script_with_no_release_url_may_run_curl(self):
        self.assertEqual(script_release('curl -fsS --retry 3 "$url"\n'), [])


def cache(text, path=".github/workflows/x.yml"):
    return check.cache_problems(path, text)


SAVE = "actions/cache/save@0057852bfaa89a56745cba8c7296529d2fc39830 # v4.3.0"
RESTORE = "actions/cache/restore@0057852bfaa89a56745cba8c7296529d2fc39830 # v4.3.0"


class CachePolicy(unittest.TestCase):
    """A cache entry is saved only on main: one a pull request saves is
    readable only by that pull request, yet counts against the repository's
    10 GB."""

    def test_the_combined_cache_action_is_found(self):
        text = step("- uses: actions/cache@0057852bfaa89a56745cba8c7296529d2fc39830 # v4.3.0", "  with:", "    path: x")
        self.assertEqual(cache(text), [".github/workflows/x.yml:2: actions/cache saves on every branch; restore with actions/cache/restore, and save with actions/cache/save only on main"])

    def test_a_save_with_no_condition_is_found(self):
        self.assertEqual(cache(step(f"- uses: {SAVE}", "  with:", "    path: x")), [".github/workflows/x.yml:2: actions/cache/save without `if: ... github.ref == 'refs/heads/main'`"])

    def test_a_save_on_a_miss_on_every_branch_is_found(self):
        self.assertEqual(len(cache(step("- name: Save", "  if: steps.c.outputs.cache-hit != 'true'", f"  uses: {SAVE}"))), 1)

    def test_a_save_only_on_main_passes(self):
        text = step("- name: Save", "  if: steps.c.outputs.cache-hit != 'true' && github.ref == 'refs/heads/main'", f"  uses: {SAVE}")
        self.assertEqual(cache(text), [])

    def test_a_restore_on_any_branch_passes(self):
        self.assertEqual(cache(step(f"- uses: {RESTORE}", "  with:", "    path: x")), [])


WORKFLOW_RUN = "on:\n  workflow_run:\n    workflows: [CI]\n    types: [completed]\njobs:\n  build:\n"


class PrivilegedCache(unittest.TestCase):
    """A workflow_run workflow runs in main's context with main's cache
    scope; code it checks out must not write an entry main's jobs restore
    (CodeQL actions/cache-poisoning, pages.yml, 2026-11-09)."""

    def test_pinned_download_in_a_workflow_run_workflow_is_found(self):
        text = WORKFLOW_RUN + step("- uses: ./.github/actions/pinned-download", "  with:", f"    url: {URL}")
        self.assertEqual(cache(text, ".github/workflows/pages.yml"), [".github/workflows/pages.yml:8: a workflow run by workflow_run shares main's cache; fetch with scripts/fetch-pinned.sh, without the cache"])

    def test_a_cache_restore_in_a_workflow_run_workflow_is_found(self):
        self.assertEqual(len(cache(WORKFLOW_RUN + step(f"- uses: {RESTORE}", "  with:", "    path: x"))), 1)

    def test_fetch_pinned_in_a_workflow_run_workflow_passes(self):
        text = WORKFLOW_RUN + step("- name: Zola", f"  run: scripts/fetch-pinned.sh --url {URL} --sha256 x --dest z")
        self.assertEqual(cache(text), [])

    def test_workflow_run_named_in_a_comment_does_not_count(self):
        text = "on:\n  # workflow_run:\n  push:\njobs:\n  b:\n" + step("- uses: ./.github/actions/pinned-download")
        self.assertEqual(cache(text), [])


ACTION = """runs:
  using: composite
  steps:
    - name: Restore
      id: cache
      uses: {restore}
      with:
        path: ${{{{ runner.temp }}}}/vcon-pinned/${{{{ inputs.name }}}}
        key: {key}
    - name: Fetch
      id: fetch
{fetch_if}      shell: bash
      env:
        SHA256: ${{{{ inputs.sha256 }}}}
      run: bash "$GITHUB_WORKSPACE/scripts/fetch-pinned.sh" --url "$URL" --sha256 "$SHA256" --dest "$DEST"
    - name: Save
      if: {save_if}
      uses: {save}
      with:
        path: ${{{{ runner.temp }}}}/vcon-pinned/${{{{ inputs.name }}}}
        key: {save_key}
"""
KEY = "pinned-${{ inputs.name }}-${{ inputs.version }}-${{ inputs.sha256 }}"
SAVE_IF = "steps.cache.outputs.cache-hit != 'true' && github.ref == 'refs/heads/main'"


def action(**kw):
    values = dict(restore=RESTORE, save=SAVE, key=KEY, save_key=KEY, fetch_if="", save_if=SAVE_IF)
    values.update(kw)
    return check.pinned_action_problems(ACTION.format(**values))


class PinnedDownloadAction(unittest.TestCase):
    def test_the_one_rule_shape_passes(self):
        self.assertEqual(action(), [])

    def test_a_key_without_the_sha256_is_found(self):
        k = "pinned-${{ inputs.name }}-${{ inputs.version }}"
        self.assertEqual(action(key=k, save_key=k), ["the cache key does not name inputs.name, inputs.version and inputs.sha256"])

    def test_restore_and_save_keys_that_differ_are_found(self):
        self.assertEqual(action(save_key=KEY + "-x"), ["the restore and save steps use different keys"])

    def test_a_fetch_step_that_can_be_skipped_is_found(self):
        self.assertEqual(action(fetch_if="      if: steps.cache.outputs.cache-hit != 'true'\n"), ["the fetch-pinned.sh step has an if:, so a restored file could go unchecked"])

    def test_a_save_after_a_hit_is_found(self):
        self.assertEqual(action(save_if="github.ref == 'refs/heads/main'"), ["the save runs after a cache hit too; an entry is never overwritten"])

    def test_an_action_with_no_fetch_step_is_found(self):
        text = ACTION.format(restore=RESTORE, save=SAVE, key=KEY, save_key=KEY, fetch_if="", save_if=SAVE_IF).replace("fetch-pinned.sh", "fetch.sh")
        self.assertIn("no step runs scripts/fetch-pinned.sh", check.pinned_action_problems(text))


class ReleaseDownloadFloor(unittest.TestCase):
    def test_a_test_script_that_names_a_release_url_is_out_of_scope(self):
        # The tests drive the scripts against file:// fixtures; they assert
        # on a release URL and never fetch one.
        with tempfile.TemporaryDirectory() as d:
            (pathlib.Path(d) / "scripts").mkdir()
            (pathlib.Path(d) / "scripts/test-x.sh").write_text(f'grep -qx "url={URL}" out\n')
            (pathlib.Path(d) / "scripts/x.sh").write_text(f'echo "{URL}"\n')
            problems = check.check(d)
        self.assertNotIn("scripts/test-x.sh: names a release asset and never calls scripts/fetch-pinned.sh", problems)
        self.assertIn("scripts/x.sh: names a release asset and never calls scripts/fetch-pinned.sh", problems)

    def test_a_tree_with_fewer_release_downloads_than_the_floor_fails(self):
        with tempfile.TemporaryDirectory() as d:
            (pathlib.Path(d) / ".github/workflows").mkdir(parents=True)
            (pathlib.Path(d) / ".github/workflows/ci.yml").write_text(step("- name: Build", "  run: cargo build"))
            problems = check.check(d)
        self.assertIn(f"found 0 release downloads through the one rule; expected at least {check.MIN_RELEASE_DOWNLOADS}. A reading that lost the steps cannot pass", problems)


class TheRealTree(unittest.TestCase):
    def test_every_download_in_the_tree_retries(self):
        root = HERE.parent
        self.assertEqual(check.check(root), [])

    def test_every_known_release_download_goes_through_the_one_rule(self):
        root = HERE.parent
        self.assertGreaterEqual(check.release_downloads(root), check.MIN_RELEASE_DOWNLOADS)

    def test_the_real_pinned_download_action_has_the_one_rule_shape(self):
        text = (HERE.parent / ".github/actions/pinned-download/action.yml").read_text(encoding="utf-8")
        self.assertEqual(check.pinned_action_problems(text), [])


if __name__ == "__main__":
    unittest.main()
