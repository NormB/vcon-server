+++
title = "Development"
description = "The code's layout, how it is tested, the gates every commit passes, and how to work on this site."
weight = 90
+++

## Layout

```text
clients/            one example client per language (see Client examples)
crates/
├── vcon/           the vCon library; today it reads and checks a vCon: form,
│                   gzip, uuid, critical (no I/O; the IETF examples are its fixtures)
└── vcon-server/
    ├── console/           the console's page, script and styles, built into the binary
    └── src/
        ├── main.rs        the command line, a thin front end
        ├── api.rs         HTTP routes, the request pipeline, problem documents
        ├── console.rs     serves the console's files with their headers
        ├── mcp.rs         the MCP server at /mcp; mcp/stdio.rs, the stdio command
        ├── access.rs      tokens, scopes, authentication
        ├── store.rs       one tenant's vCons: versions, deletes, purges, legal reads
        ├── tenants.rs     the registry: tenants, their keys, API keys, the admin log
        ├── envelope.rs    keys in locked memory; seal, wrap, HKDF
        ├── catalog.rs     SQLCipher catalogs
        ├── audit.rs       the audit log
        ├── retention.rs   calendar retention periods
        ├── config.rs      the configuration file
        ├── datadir.rs     who may use the data directory at once
        ├── serve.rs       the server and its purge sweep
        ├── fs.rs, clock.rs  the filesystem and the clock, behind interfaces for tests
        ├── ids.rs         tenant, version and key ids
        └── query.rs       query strings, read by one rule for every route
fuzz/               cargo fuzz targets for everything the server reads from outside
```

`crates/vcon-server/openapi.json` describes the HTTP API. It is written by
hand; `website/static/openapi.json` must be the same bytes. The unit tests
fail when a route, a status a test produces or a problem type is missing
from it.

The binary does almost nothing itself: everything it runs is in the library,
so tests drive the same code the binary does.

## Build and test

```sh
cargo build
cargo test --workspace
```

The tests that run the real binary (`crates/vcon-server/tests/`) start a
server on a free port, with a temporary data directory and certificate, and
check what a client and an operator see: stores and reads, memory locked,
no core dumps, a second server refused, SIGTERM.

The console's browser tests, in `console-tests/`, drive headless Chromium
with [Playwright](https://playwright.dev/) against the real server.
`scripts/test-console.sh` builds the binary, starts it with a temporary data
directory, certificate and operator key, runs the tests and removes
everything it made. Install the pinned Playwright and its browser once:

```sh
npm ci --prefix console-tests
(cd console-tests && npx playwright install --only-shell chromium)
scripts/test-console.sh
```

Each test makes its own tenant and keys through the admin API, and fails on
any content-security violation in the page.

## How the code is tested

- **Test first.** Each behaviour's test is written before the code and seen to
  fail for the right reason.
- **Faults injected.** After a change passes, its code is broken on purpose,
  one fault at a time (a condition inverted, a step skipped, a limit moved),
  and the tests must fail for each. A fault that survives means a missing
  test, which is then written.
- **Crashes simulated.** `fs.rs` has a test filesystem that can cut power at
  any step: what was not synced is lost, and the store must reopen whole.
- **Races forced.** Test hooks pause a request between its steps so another
  can run in the gap: two writers on one uuid, a purge during a restore, a
  removal during a request.
- **Time controlled.** Retention and the sweep run on a clock the tests set.
- **Fuzzed.** Every parser of outside input has a fuzz target; see below.

## Fuzzing

Each target in `fuzz/fuzz_targets/` feeds random input, grown from its seeds,
to one thing the server reads from outside:

| Target | What it reads |
| --- | --- |
| `inspect` | any body as a vCon, with no media type; the same body gzipped must give the same answer |
| `inspect_gzip` | any body sent as `application/vcon+gzip`: the inflater and its limit |
| `inspect_signed` | any bytes as a JWS payload: base64url, a gzipped payload, nested signatures up to the layer limit, header uuids; it must give what the payload gives on its own |
| `inspect_encrypted` | any JSON as a JWE's `unprotected` member |
| `parse_uuid` | any text as a uuid in a path |
| `token` | any text as an API token |
| `ids` | any text as a key, tenant or version id |
| `retention` | any text as a retention period, added to any time |
| `config` | any text as the configuration file |
| `query` | any text as a query string, as `GET /vcons` reads it (names, percent decoding, limit, cursor, time bounds) and as the audit routes read it |
| `sealed` | any bytes as a sealed vCon or a wrapped key read from disk; none may open |
| `http_request` | any request to the API, served in-process: token, path, media type, `Content-Length`, `If-Match`, the body and inflation limits, every handler; no answer is 500 or more, and a body over a limit is answered 413 |

A target fails on a panic, on a failed check of what it reads, and on a
sanitizer error. Fuzzing needs the nightly toolchain named in
`fuzz/rust-toolchain.toml` and [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz)
0.13.2:

```sh
cargo install cargo-fuzz --version 0.13.2 --locked
scripts/fuzz.sh list                  # the targets
scripts/fuzz.sh run inspect 60        # run one target for 60 seconds
scripts/fuzz.sh plant inspect         # plant a panic, expect the target to find it
```

`scripts/fuzz.sh run` reads the seeds in `fuzz/seeds/<target>/` and, for
`inspect` and `inspect_gzip`, the IETF examples. What libFuzzer adds goes to
`fuzz/corpus/<target>/`, and a crash input to `fuzz/artifacts/<target>/`;
git ignores both. A crash is a defect: fix it with a regression test written
first, in the crate the crash is in.

The seeds hold no personal data. The vCons among them are three of the
synthetic vCons from [vcon_faker](https://github.com/vcon-dev/vcon_faker),
with every name, email address, phone number and URL replaced by a reserved
value and every free text by neutral words.

`scripts/fuzz.sh plant` checks the targets themselves. For each line of
`fuzz/plants.txt` it puts `panic!("planted fault");` before one line of the
code a target reads, runs that target (300 seconds at most, set by
`PLANT_SECONDS`), and expects the crash. It then puts the file back and
compares it with its copy, byte for byte. A target that misses its plant
fails the check.

## Test dependency from a fork

The tests use a fork of rusqlite beside rusqlite itself:
`https://github.com/NormB/rusqlite` at commit
`a2936ca6190f54e7ab54375385877cc96eebebe5`, version `0.41.0-sqlcipher.1`,
the dev-dependency `rusqlite_fork` of vcon-server.

- **Why:** rusqlite 0.41.0's pragma helpers (`pragma_update`,
  `pragma_query_value` and the others) refuse SQLCipher's own pragmas, such
  as `cipher_log_level`, with "Invalid pragma". They accept only names in
  SQLite's pragma list, and SQLCipher's are not in it. So `catalog.rs` sets
  SQLCipher's settings as plain SQL. The fork also accepts SQLCipher's names.
  The test `sqlcipher_settings_read_the_same_through_the_pragma_helpers`
  reads the settings back through the fork's helpers and checks that they
  equal what the plain SQL reads. The bug is not reported upstream yet.
- **What uses it:** only the tests. The server, from `cargo build`, a release
  or crates.io, uses crates.io rusqlite 0.41.0. A `[patch]` in the workspace
  `Cargo.toml` gives the fork crates.io's libsqlite3-sys 0.39.1, so both
  share one SQLite. A build fetches the fork all the same (see
  [building without network access](@/docs/install.md#building-without-network-access)).
- **When it goes:** the test `crates_io_rusqlite_refuses_sqlcipher_names`
  fails once the workspace's rusqlite accepts SQLCipher's names, as in the
  dependency update that brings such a release. Then set the settings
  through the pragma helpers in `catalog.rs`, and remove the `rusqlite_fork`
  dev-dependency, the `[patch]` section, both tests and this section.

## Code style

The Rust code follows the
[Google Rust Style Guide](https://google.github.io/styleguide/rustguide.html),
the [Rust Style Guide](https://doc.rust-lang.org/nightly/style-guide/) and,
for the published crates' public API, the
[Rust API Guidelines](https://rust-lang.github.io/api-guidelines/).

- **Format** with `scripts/rustfmt.sh`, not `cargo fmt`. `rustfmt.toml`
  sets options only a nightly rustfmt reads, so the script runs the nightly
  it pins, for formatting only; `scripts/rustfmt.sh install` installs it.
  Builds, clippy and tests use the stable toolchain.
- **`scripts/check_style.py`** checks what rustfmt and clippy do not: glob
  imports (allowed: `use super::*` in an inline module, `use Enum::*` in a
  function, `pub use sub::*` in a non-leaf module), a reason above each
  `#[rustfmt::skip]`, and a message on every assertion, tests included,
  saying why it holds. Until every assertion has one,
  `style/assert-message-baseline.txt` holds how many each file may still
  lack; a new file may lack none, and a file that gets better lowers its
  count with `scripts/check_style.py --write-baseline`, which never raises
  one.
- **Clippy and rustc lints** for the rest are set at deny in
  `[workspace.lints]` in `Cargo.toml`. Every package takes them, and
  `check_style.py` fails if one does not. A module that does not meet one
  yet carries `#[expect(lint, reason = "...")]` on its `mod` line in
  `lib.rs`; the expectation fails once the module is fixed, and is then
  removed. `expect()` is allowed: the Google guide prefers it to `unwrap()`,
  with the reason.

## Gates

Enable the repository's hooks once per clone:

```sh
git config core.hooksPath .githooks
```

- **pre-commit** runs `scripts/rustfmt.sh --check`, `scripts/check_style.py`,
  `cargo clippy -D warnings` and the tests on exactly what is staged, then the same for the fuzz package (its
  format, clippy and its own tests), and refuses personal data and AI-tool
  attribution in staged changes. It also checks that every path
  `.bestpractices.json` links to exists (`scripts/check_repo_links.py`), and
  what each crate would publish to crates.io (`scripts/check_package.py`):
  it packages each published crate, refuses any file in a `.crate` other
  than the sources, the files they include, the README and the two
  licences, and builds each crate from its `.crate` alone; a missing README or licence; a dependency taken by git or by
  path, in the manifest or the lock file; a relative link in the README; and
  anything the scan above refuses.
- **commit-msg** refuses the same in the message, every line of it,
  including lines that start with `#`, which `git commit -m` keeps.
- **pre-push** refuses any outgoing commit that is not signed by an allowed
  signer, whose author and committer are not the pusher's configured
  identity, or whose whole message the same scan refuses
  (`scripts/scan-commit-messages.sh`), so a commit made with `--no-verify`
  is still caught. CI scans every commit message of a push or pull
  request the same way.

`scripts/test-hooks.sh` and `scripts/test-pre-push.sh` test the hooks
themselves. Every commit on `main` is signed and shows as verified on GitHub.

Test data uses reserved values only: `example.com` addresses, 555-01xx
numbers, RFC 5737 and RFC 3849 addresses. The one exception is the IETF
draft's own examples, kept unchanged under
`crates/vcon/tests/fixtures/ietf/`.

## This website

The site is [Zola](https://www.getzola.org/) 0.23.6, in `website/`. Preview it
with `zola serve` from that directory.

```sh
scripts/check-site.sh   # builds the site and checks it, as the deploy does
```

The check fails on a broken internal link, on any inline script, inline style
or event-handler attribute (the pages' content-security policy forbids them),
and when the [problem types](@/problems/_index.md) or the
[configuration](@/docs/configuration.md) page disagree with the code. Once
CI passes on a push to `main`, `.github/workflows/pages.yml` deploys the site
of that commit to GitHub Pages; Cloudflare fronts it.

```sh
npm ci --prefix site-tests
npx --prefix site-tests playwright install --only-shell chromium
scripts/test-site.sh    # the built site in a browser
```

`scripts/test-site.sh` builds the site, serves it locally and runs the
Playwright tests in `site-tests/`. They check every page with axe-core
against WCAG 2.2 level AA, contrast and landmarks included, in light and dark
at 360 and 1440 pixels wide. They also check the home page at 768 pixels, that
it never scrolls sideways, that the browser logs no error (a refusal by the
content-security policy is one), and that it shows the sections it is built
from. On every page they check three WCAG 2.2 AA criteria axe does not:
nothing scrolls sideways at 320 pixels (reflow), no keyboard focus stop is
hidden under the pinned header, and a link to a place on a page shows it below
the header. The search tests check that the search box is labelled and that
what it finds is announced as a status. CI runs it in the website job.

The home page states the number of automated tests and the share of lines
they cover. Nobody types these numbers. The deploy downloads the test output
and the coverage summary from CI's run of the commit it deploys, derives the
numbers with `scripts/published_metrics.py`, and writes them to
`website/data/metrics.toml`, which git ignores. The page states exactly what
that file says, or no number when it is absent; the site check fails
otherwise, and also builds the site with a made-up file and requires the
page to state its numbers.

The logo and the favicons are drawn from `assets/brand/mark.svg` by
`scripts/icons.py`; see `assets/brand/README.md`. The check above also
refuses an SVG that holds a script, a style, a link or an embedded object.

## The how-to pages are tested

The [install](@/docs/install.md), [getting started](@/docs/getting-started.md)
and how-to pages are run, not only read. `scripts/docs_test.py` reads a page
from top to bottom, runs each `sh` block in one bash shell in a fresh
container, and compares what each block prints with the `text` block that
follows it on the page. In that `text` block, `<words>` stands for any text
on its line, such as a token or a date, and a line holding only `…` stands
for any number of lines. A block must exit 0, unless its marker says
otherwise.

An HTML comment on the line before a block changes how it runs:

| Marker | Effect |
|---|---|
| `<!-- docs-test: only apt release -->` | runs only in scenarios that have every tag named |
| `<!-- docs-test: skip <reason> -->` | never runs; the reason is required |
| `<!-- docs-test: exit 1 -->` | the block must exit 1 |
| `<!-- docs-test: background serving -->` | runs in the background; the run waits until it prints `serving` |
| `<!-- docs-test: file <path> -->` | a block, such as TOML or a systemd unit, that the page says to save as `<path>` |
| `<!-- docs-test: always -->` | runs at the end, even after a failure |

A scenario names the pages it runs, the system it runs them on, and its
tags. The scenarios include Debian 13 and Fedora 44 with the server in the
foreground, Debian 13 under systemd, each install path on each distribution,
an upgrade from the previous release to the server under test and back, and
the container page with Docker on the runner. The how-to scenarios get the server built from the commit
under test. The release scenarios download the release that the install page
names. Until that release is published, they download the newest published
release and say so.

```sh
python3 -I scripts/test_docs_test.py                    # the runner's own tests
python3 -I scripts/docs_test.py lint                    # the rules every tested page keeps
python3 -I scripts/docs_test.py list                    # the scenarios
cargo build -p vcon-server
python3 -I scripts/docs_test.py run foreground --binary target/debug/vcon-server
```

It needs Docker. `lint` fails on a block that no scenario runs, on a request
to the server with no output shown after it, and on a marker it does not
know. A run prints one line per block, and for a failure, the block, what the
page says it prints, and what it printed.

## Continuous integration

Every push to `main` and every pull request runs `.github/workflows/ci.yml`:
format, clippy and the tests as the commit hook runs them (on a push to
`main` it keeps the test output and the coverage summary for the deploy,
and fails if the test output cannot be counted); the console's
browser tests; the package check; the scan for AI attribution and personal
data over every tracked file; the `.bestpractices.json` link check; `scripts/check-site.sh`;
and `scripts/icons.py --check`. A coverage job runs the same tests under
`cargo llvm-cov` and sends the result to Codecov. The how-to jobs run `scripts/docs_test.py`
for each scenario it lists, against the server built from the commit (see
[the how-to pages are tested](#the-how-to-pages-are-tested)). One fuzz job per target
runs it for 60 seconds, then `scripts/fuzz.sh plant` for it; the targets are
read from `fuzz/Cargo.toml`, so a new one joins with no list to edit. A
crash input is kept as a workflow artifact for 7 days.
[ClusterFuzzLite](https://google.github.io/clusterfuzzlite/) builds the same
targets in its own image (`.clusterfuzzlite/`) and runs them when a pull
request or a push to `main` changes `crates/` or `fuzz/` (five and ten
minutes), and for an hour in all each Monday
(`.github/workflows/clusterfuzzlite.yml`). Each action and
each downloaded tool is pinned by commit or by sha256.

## Contributing

- **A bug or a feature request:** open an issue at
  <https://github.com/NormB/vcon-server/issues>. For a bug, give the output
  of `vcon-server --version`, the commit you built, what you did, what you
  expected and what happened, and the log lines with every token removed.
- **A change:** send a pull request against `main`. Read
  [CONTRIBUTING.md](https://github.com/NormB/vcon-server/blob/main/CONTRIBUTING.md)
  first, and sign the [Contributor License Agreement](@/cla.md) before
  your first pull request merges.
- **A security problem:** report it privately, never in an issue; see
  [reporting a vulnerability](@/docs/security.md#reporting-a-vulnerability).

## Licence

MIT OR Apache-2.0, at your option.
