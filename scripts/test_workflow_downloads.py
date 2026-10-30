"""Tests scripts/check_workflow_downloads.py: every step in the workflows,
the composite actions and the shell scripts that downloads a toolchain or a
tool does so with a bounded retry.

Run with: python3 -I scripts/test_workflow_downloads.py
"""

import importlib.util
import pathlib
import sys
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
        self.assertEqual(found("        run: dnf install -y git-core\n"), [".github/workflows/x.yml:1: dnf without a retry that cleans its metadata"])
        self.assertEqual(len(found("            yum install -y jq\n            microdnf install -y jq\n")), 2)

    def test_dnf_through_the_retry_script_cleaning_metadata_passes(self):
        self.assertEqual(found("          RETRY_BETWEEN='dnf clean metadata' scripts/retry.sh dnf install -y jq\n"), [])

    def test_dnf_through_the_retry_script_without_cleaning_is_found(self):
        # An empty repomd.xml is a parse error, not a failed download, so a
        # second try must start from fresh metadata.
        self.assertEqual(len(found("          scripts/retry.sh dnf install -y jq\n")), 1)

    def test_cleaning_once_without_a_retry_is_found(self):
        self.assertEqual(len(found("          dnf clean metadata && dnf install -y jq\n")), 1)

    def test_an_inline_loop_that_cleans_metadata_passes(self):
        # Before the checkout no repository script exists yet.
        line = "        run: for i in 1 2 3 4 5; do dnf install -y git-core && break; [ \"$i\" = 5 ] && exit 1; dnf clean metadata; sleep $((i * 10)); done\n"
        self.assertEqual(found(line), [])


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


class TheRealTree(unittest.TestCase):
    def test_every_download_in_the_tree_retries(self):
        root = HERE.parent
        self.assertEqual(check.check(root), [])


if __name__ == "__main__":
    unittest.main()
