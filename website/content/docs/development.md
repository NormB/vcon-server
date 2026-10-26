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
  [getting started](@/docs/getting-started.md#build)).
- **When it goes:** the test `crates_io_rusqlite_refuses_sqlcipher_names`
  fails once the workspace's rusqlite accepts SQLCipher's names, as in the
  dependency update that brings such a release. Then set the settings
  through the pragma helpers in `catalog.rs`, and remove the `rusqlite_fork`
  dev-dependency, the `[patch]` section, both tests and this section.

## Gates

Enable the repository's hooks once per clone:

```sh
git config core.hooksPath .githooks
```

- **pre-commit** runs `cargo fmt --check`, `cargo clippy -D warnings` and the
  tests on exactly what is staged, then the same for the fuzz package (its
  format, clippy and its own tests), and refuses personal data and AI-tool
  attribution in staged changes. It also checks that every path
  `.bestpractices.json` links to exists (`scripts/check_repo_links.py`), and
  what each crate would publish to crates.io (`scripts/check_package.py`):
  it packages and builds each crate, then refuses any file in a `.crate`
  other than the sources, the files they include, the README and the two
  licences; a missing README or licence; a dependency taken by git or by
  path, in the manifest or the lock file; a relative link in the README; and
  anything the scan above refuses.
- **commit-msg** refuses the same in the message.
- **pre-push** refuses any outgoing commit that is not signed by an allowed
  signer, or whose author and committer are not the pusher's configured
  identity.

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
[configuration](@/docs/configuration.md) page disagree with the code. A push
to `main` that touches the site deploys it to GitHub Pages; Cloudflare fronts
it.

The logo and the favicons are drawn from `assets/brand/mark.svg` by
`scripts/icons.py`; see `assets/brand/README.md`. The check above also
refuses an SVG that holds a script, a style, a link or an embedded object.

## Continuous integration

Every push to `main` and every pull request runs `.github/workflows/ci.yml`:
format, clippy and the tests as the commit hook runs them; the console's
browser tests; the package check; the scan for AI attribution and personal
data over every tracked file; the `.bestpractices.json` link check; `scripts/check-site.sh`;
and `scripts/icons.py --check`. A coverage job runs the same tests under
`cargo llvm-cov` and sends the result to Codecov. One fuzz job per target
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
