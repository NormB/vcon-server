+++
title = "Development"
description = "The code's layout, how it is tested, the gates every commit passes, and how to work on this site."
weight = 62

[extra]
group = "project"
+++

## Layout

```text
clients/            one example client per language (see Client examples)
crates/
├── vcon/           the vCon library: it reads and checks a vCon (form, gzip, uuid,
│                   critical) and writes one, with no I/O; the IETF examples are
│                   its fixtures
└── vcon-server/
    ├── console/           the console's page, script and styles, built into the binary
    └── src/
        ├── main.rs        the command line, a thin front end
        ├── lib.rs         the program itself, which main.rs runs and the tests drive
        ├── commands.rs    the commands other than serve
        ├── api.rs         HTTP routes, the request pipeline, problem documents
        ├── console.rs     serves the console's files with their headers
        ├── mcp.rs         the MCP server at /mcp; mcp/stdio.rs, the stdio command
        ├── tui.rs         the terminal UI, a client of a running server
        ├── reveal.rs      the reveal marker on a read, for the audit log
        ├── roles.rs       the roles a membership holds, and the scope each gives
        ├── access.rs      tokens, scopes, authentication
        ├── store.rs       one tenant's vCons: versions, deletes, purges, legal reads
        ├── backend.rs     where the data is kept: local files, an S3 bucket, PostgreSQL
        ├── export.rs      export parquet: a tenant's versions as Parquet files
        ├── tenants.rs     the registry: tenants, their keys, API keys, users, memberships, the admin log
        ├── envelope.rs    keys in locked memory; seal, wrap, HKDF
        ├── catalog.rs     SQLCipher catalogs
        ├── audit.rs       the audit log
        ├── retention.rs   calendar retention periods
        ├── config.rs      the configuration file
        ├── secret_file.rs  the one rule for every secret file the configuration names
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
scripts/nextest.sh install
scripts/nextest.sh run --workspace
cargo test --workspace --doc
```

The tests run under [cargo-nextest](https://nexte.st/), which runs each test
in a process of its own and ends a test still running at the deadline in
`.config/nextest.toml`: a test that runs for more than 60 seconds is
reported as `SLOW` by name, and after 3 minutes it is ended and fails as
`TIMEOUT`. `cargo test` has no such deadline, so one hung test would hang the
run. `scripts/nextest.sh` pins nextest's version; `scripts/nextest.sh install`
downloads that release for x86_64 or aarch64 Linux through
`scripts/fetch-pinned.sh` (see [pinned tool downloads](#pinned-tool-downloads)),
which checks it against its pinned SHA-256, and puts it in cargo's `bin`
directory, and the commit hook refuses to run another version. nextest does not run doc tests, so
`cargo test --doc` runs those. Arguments after `run` are nextest's, which
takes cargo's package and target options and a test-name filter, as
`cargo test` does.

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

The store's conformance suite runs on every backend. With the
`backend-tests` feature it runs against a real S3-compatible service as
well, with the S3 backend's own tests and `serve` keeping its objects in a
bucket; CI's "storage backends (RustFS)" job runs them against RustFS, and
"storage backends (MinIO)" against MinIO, whose binary it checks against a
pinned SHA-256 (MinIO publishes no container image any more). The tests
fail, never skip, when the service is not there. Start a RustFS that
listens on the loopback address only, and give the tests its address and
keys:

```sh
docker run -d --rm --name vcon-rustfs -p 127.0.0.1:9000:9000 \
  -e RUSTFS_ACCESS_KEY=vcontest -e RUSTFS_SECRET_KEY=vcon-local-only-secret \
  rustfs/rustfs@sha256:ed9210d6411d77ffceb70d94b964a3cb7d225983368bc25bebe01e995293dcc2
export VCON_TEST_S3_ENDPOINT=http://127.0.0.1:9000
export VCON_TEST_S3_ACCESS_KEY=vcontest VCON_TEST_S3_SECRET_KEY=vcon-local-only-secret
scripts/nextest.sh run -p vcon-server --features backend-tests --lib backend::
scripts/nextest.sh run -p vcon-server --features backend-tests --test serve s3::
docker stop vcon-rustfs
```

Each test makes a bucket of its own, so a run leaves buckets behind;
`docker stop` removes the container and them.

The PostgreSQL catalog's connector and pool (the `postgres` cargo feature, a
preview) are tested against real PostgreSQL servers as well. With both
`postgres` and `backend-tests`, the tests in `backend::postgres::` connect
to the servers `scripts/pg-test-server.sh` starts in containers: a primary
with TLS and one test role for each way of logging in, a hot standby of it,
and a server with ssl off. CI's "catalog backends (PostgreSQL N)" jobs run
them on PostgreSQL 16 and 18, each image pinned by digest, with RustFS
beside them for `serve`'s test, since a PostgreSQL catalog needs its objects
in a bucket. As with S3, the tests fail, never skip, when the servers are
not there. With the RustFS above still running:

```sh
image=postgres@sha256:f4610795587492cd4fb887c2dc3de301b813278bb6f616992061141b5f15ac36 # 16.15-alpine
scripts/pg-test-server.sh start "$image" "$PWD/target/pg-test"
set -a; . target/pg-test/env; set +a
scripts/nextest.sh run -p vcon-server --features postgres,backend-tests --lib backend::postgres::
scripts/nextest.sh run -p vcon-server --features postgres,backend-tests --test serve postgres::
scripts/pg-test-server.sh stop "$PWD/target/pg-test"
```

`stop` removes the containers and the network `start` made, by the ids
`start` recorded, and the directory.

## Linking

Test and development builds on aarch64 Linux link with the Rust
toolchain's own lld: `.cargo/config.toml` names `scripts/cc-lld.sh` as the
linker for `aarch64-unknown-linux-gnu`, and it runs `cc -fuse-ld=lld` with
the toolchain's `lib/rustlib/<target>/bin/gcc-ld` directory. Measured on
2026-11-02 on an aarch64 host, the 118 MB `serve` test binary linked in
2.1 to 2.6 seconds with lld and in 3.6 to 4.4 with GNU ld. A toolchain without that directory (a Linux distribution's own rustc,
for example) links with plain `cc` and GNU ld, so nothing needs installing.
On x86_64 Linux rustc links with its own lld already, and the config names
no linker there. Release builds (`scripts/reproducible-build.sh`) set
plain `cc` for aarch64, so the binaries released are linked as before.

## How the code is tested

- **Test first.** Each behavior's test is written before the code and seen to
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

- **pre-commit** refuses personal data and AI-tool attribution in staged
  changes (`scripts/scan_forbidden.py`), and checks that git ignores every
  export of the private design documents and every private spec or plan
  file (`*.local.md`), and that none is staged
  (`scripts/check-ignored-exports.sh`). On a copy of exactly what is staged,
  it checks that every action is pinned by commit
  (`scripts/check_action_pins.py`, see
  [adding a GitHub Action](#adding-a-github-action)) and runs
  `scripts/rustfmt.sh --check`, `scripts/check_style.py --walk`,
  `cargo clippy --workspace --all-targets --locked --quiet -- -D warnings`,
  the documentation with rustdoc's warnings denied
  (`cargo doc --workspace --no-deps --locked --quiet`),
  the tests with `scripts/nextest.sh run --workspace --locked` (each test
  under the deadline in `.config/nextest.toml`) and the doc tests with
  `cargo test --workspace --locked --quiet --doc`, then the same for the fuzz
  package (its format, clippy and its own tests). On aarch64 it links the
  test builds with the staged `scripts/cc-lld.sh` (see
  [Linking](#linking)). It also checks that every
  path `.bestpractices.json` links to exists
  (`scripts/test_check_repo_links.py`), and what each crate would publish to
  crates.io (`scripts/check_package.py`):
  it packages each published crate, refuses any file in a `.crate` other
  than the sources, the files they include, the README and the two
  licenses, and builds each crate from its `.crate` alone; a missing README or license; a dependency taken by git or by
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
the header. The search tests check that the search box is labeled, that
what it finds is announced as a status, and that a page fetches the search
library and index only once the box is used: the index is the site's
largest file, over a megabyte, so no page loads it up front. The sidebar
tests check that it lists the pages under their group headings, marks the
page being read, and matches the documentation's index. On a screen too
narrow for the sidebar to sit beside the page, they check that
`website/static/js/nav.js` folds its links behind one button, so the
page's heading is on the first screen; that the button opens and closes
them with the keyboard, and Escape closes them; and that without
JavaScript the links are shown and there is no button. They also check
that the edge of the search box and of that button has a contrast of 3 to
1 or more against the page, and that no box that clips its contents cuts
the focus ring of a sidebar link or of a code block on the home page. CI
runs it in the website job.

### Adding a documentation page

**Purpose:** a new page under `website/content/docs/` that the sidebar, the
indexes and the tests all know.

**Prerequisites:** Zola 0.23.6 for `scripts/check-site.sh`; nothing changes
on any other system until the page is deployed with the next push to `main`.

**Steps:**

1. Write the page with `title`, `description` and `weight` in its front
   matter, and its group under `[extra]`: `group = "start"`, `"how-to"`,
   `"concepts"`, `"reference"` or `"project"`. The groups, with the
   headings the sidebar shows, are `docs_groups` in `website/config.toml`.
2. Choose a `weight` no other page has, between the weights of the group's
   first and last pages: the sidebar lists the pages by weight.
3. Add a line for it to `docs/README.md`, with the page's title and
   description word for word, and to `website/static/llms.txt`.
4. A page in the `start` or `how-to` group is a guide: link it from
   [how-to guides](@/docs/how-to.md), and give it a heading that starts
   "Before you" (what to have first) and one that starts "When" (what to do
   when a step fails).

**Verify:** `scripts/nextest.sh run -p vcon-server --test docs_index` passes,
and `scripts/check-site.sh` prints `check-site: ok`.

**Troubleshoot:** the `docs_index` tests name the page and what it lacks: a
group the sidebar does not show, a weight another page has, a page of one
group sorted among another's, a line missing from `docs/README.md` or
`llms.txt`, or a guide without one of its two headings.

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
| `<!-- docs-test: skip <reason> -->` | never runs; the reason is required, and may not name a release |
| `<!-- docs-test: since 0.3.0 -->` | runs only when the page's `VERSION` is that release or a later one; the text before the block must name the release |
| `<!-- docs-test: exit 1 -->` | the block must exit 1 |
| `<!-- docs-test: background serving -->` | runs in the background; the run waits until it prints `serving` |
| `<!-- docs-test: file <path> -->` | a block, such as TOML or a systemd unit, that the page says to save as `<path>` |
| `<!-- docs-test: always -->` | runs at the end, even after a failure |
| `<!-- docs-test: terminal <word> -->` | a full-screen command: it runs in a terminal of 80 columns by 24 lines (util-linux `script`), in a bash of its own that sees the exported variables only; when it writes `<word>`, `q` is typed; the text block after it is the screen it had when it ended |

A step that works only from some release on, such as checking a signature
that older releases do not have, is marked `since`, not `skip`. Until that
release is published, the runner uses the newest published one, so the step
does not run and the runner says so; from then on it runs. `lint` refuses a
`skip` whose reason names a release, and a `since` block whose text does
not name its release, so a reader on an older release is told to skip it.

A scenario names the pages it runs, the system it runs them on, and its
tags. The scenarios include Debian 13 and Fedora 44 with the server in the
foreground, Debian 13 under systemd, each install path on each distribution,
an upgrade from the previous release to the server under test and back,
the storage pages with RustFS and MinIO, the PostgreSQL page with Debian's
own PostgreSQL, and the container page with Docker on the runner. The
how-to scenarios get the server built from the commit under test; a
scenario whose pages need a cargo feature the default build lacks names it
(`features`), and CI gives it a server built with that feature
(`docs_test.py artifact <scenario>` names the artifact). The release scenarios download the release that the install page
names. Until that release is published, they download the newest published
release and say so.

The runner keeps an outage elsewhere from failing a page whose commands are
right:

- A GitHub release asset a step downloads, such as a vcon-server release,
  cosign, RustFS, MinIO or DuckDB, comes through the runner's mirror. The
  mirror is an HTTP server on Docker's bridge address
  (`docker network inspect bridge`). The runner rewrites the step's
  `https://github.com/<owner>/<repo>/releases/download/` URL to the
  mirror's, and the mirror fetches that exact github.com URL with
  `scripts/fetch-pinned.sh`, checked against its sha256 in
  `scripts/docs-test-pins.txt`. A URL that the release does not have still
  fails the step, because the mirror answers `404` as github.com does.
  Before the first step the runner fetches every pinned asset the steps can
  name on this CPU, so the mirror answers each step at once: dnf gives up on
  a server that sends less than 1000 bytes a second for 30 seconds, and
  `fetch-pinned.sh` may wait out an outage for five minutes.
  `DOCS_TEST_PINNED` names the directory the mirror keeps the files in; CI
  restores it from the actions cache and saves it on `main`.
- Each image a scenario runs in, and each image a page builds `FROM`, is
  pinned by digest in `IMAGES` in `scripts/docs_test.py` and pulled with
  retries; an image a page builds from is then tagged with the name the page
  uses.
- In a container with apt, the runner sets `Acquire::Retries "3"` and
  `Acquire::http::Timeout "10"` before the first page, so apt retries each
  download and gives up one that stalls.
- A container with apt reads its packages from a list of mirrors, not from
  one host. Before the scenario starts, the runner asks each mirror in
  `scripts/apt-mirrors.txt` (a source a line: the address the image names,
  then its mirrors in the order apt tries them) for its index, with a time limit
  of 5 seconds, and writes those that answer, in order, to a list that
  apt's `mirror+file:` method reads. The log names them in a line that
  starts `docs-test: packages from`. Signatures are checked as before. The
  page's own `apt-get` commands are not changed. If no mirror of a source
  answers, the run ends at once with exit code 3 and the message `the
  package mirrors are unreachable from this machine; this is not a failure
  of the pages`, with each address it tried.

`lint` also fails on a page that downloads a release asset no line of
`scripts/docs-test-pins.txt` matches, and on a sha256 written on such a page
that is not one of the pins.

```sh
python3 -I scripts/test_docs_test.py                    # the runner's own tests
python3 -I scripts/docs_test.py lint                    # the rules every tested page keeps
python3 -I scripts/docs_test.py list                    # the scenarios
cargo build -p vcon-server
python3 -I scripts/docs_test.py run foreground --binary target/debug/vcon-server
```

It needs Docker. `lint` fails on a block that no scenario runs, on a guide
(a page in the sidebar's first two groups) that no scenario runs, on a
request to the server with no output shown after it, on a marker it does
not know, on a directory that one page makes for root alone and a page
writes under as the service account, and on an install step on the home
page whose code is not a run of lines in a block of the install page. A run prints one line per block, and for a failure, the block, what the
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
each downloaded tool is pinned by commit or by sha256; for actions, the scan
job and the commit hook check it (see
[adding a GitHub Action](#adding-a-github-action)), and for release assets,
`scripts/check_workflow_downloads.py` (see
[pinned tool downloads](#pinned-tool-downloads)).

The release build and the reproducible-build check run in an AlmaLinux 9
container, which has neither git nor a C compiler. Every `dnf install`
there goes through `scripts/dnf_install.py <package>...`. dnf asks the next
mirror of its list when a download fails, but it stops at a mirror whose
`repomd.xml` arrives and does not parse, and a second try of the same
command reads the same mirror. The script follows a failed try with one
that reads each enabled repository from the address its file names in a
commented `baseurl` line (the distribution's own server), and takes turns
between the two sources for at most five tries, with the metadata cleaned
and a wait of 5 to 40 seconds between them. It never changes `gpgcheck`.
Because the script is in the repository, those jobs check out before they
install anything: the build jobs fetch the files, run
`scripts/release-build-env.sh` (which installs git too), and check out
again to have `.git`. `scripts/check_workflow_downloads.py` fails a
`dnf install` anywhere else in a workflow, an action or a shell script,
and a `run:` step before the checkout of a job that runs in a container.
`scripts/test_dnf_install.py` tests the rule without a mirror.

The release's smoke job runs the built binary in debian:12 and
ubuntu:22.04 containers, where it installs curl and jq with `apt-get`.
Each image names one host for each package source, and apt's retries ask
that host again. So the job runs `bash scripts/apt-mirror-setup.sh` before
its first `apt-get`. The script needs bash and coreutils only, because the
images have no curl and no python3 yet. It reads `scripts/apt-mirrors.txt`,
the table the how-to runner reads, asks each mirror of each source the
system's files name for `<mirror>/dists/` with a time limit of 5 seconds,
and writes those that answer 200, in the table's order, to the list apt's
`mirror+file:` method reads: the files the how-to runner writes. It ends
with exit code 3 when no mirror of a source answers and with 4 when no
source of the system is in the table, and changes nothing in either case.
`scripts/check_workflow_downloads.py` fails an `apt-get update` or
`install` that comes before a run of the script: in a workflow, within a
job that runs in a container, and in a composite action or a shell script,
within the file. `scripts/test_apt_mirror_setup.py` tests the script
against a server on the same machine.

## Pinned tool downloads

CI installs cargo-nextest, DuckDB, MinIO, cargo-llvm-cov, cargo-deny, Zola,
resvg, cargo-fuzz and cargo-cyclonedx from GitHub release assets, and the
how-to scenarios download vcon-server releases, cosign, RustFS, MinIO, mc and
DuckDB. Each one is pinned by version and by sha256.

On 2026-11-08, and again on 2026-11-09 from about 05:15Z, github.com
answered release downloads with HTTP 504 for most of an hour. Each install
step was one `curl` with about 25 seconds of retries, so main's CI on
`2cb225a` failed at every job that fetched one of them, while the code was
green.

Every such download now goes through one rule:

- `scripts/fetch-pinned.sh` puts the pinned bytes of one file at a destination.
  - It hashes a file already at the destination first. On a match it uses
    that file and fetches nothing. On a mismatch it deletes the file with a
    warning, then fetches it again.
  - It retries HTTP 408, 429 and 5xx, timeouts, refused or reset connections
    and transfers cut short. The delay starts at 5 seconds and doubles up to
    60. It retries only while the time spent plus the next delay stays within
    300 seconds, and curl's time limit bounds each attempt.
  - Its exit status names the kind of failure: `1` is an outage, so a re-run
    can pass; `3` is bytes that do not match the pin, so a re-run cannot; `4`
    is a permanent refusal such as HTTP 404, so the URL or the version is
    wrong; `2` is wrong arguments. Each error names the URL and the last HTTP
    status or curl error.
- `.github/actions/pinned-download` runs that script between an `actions/cache` restore and save. The key is
  the tool's name, its version and its sha256. The script runs on every run,
  so it hashes a restored file before anything uses it. The save runs only
  after a miss, and only on `main`. The directory is the job's own
  `runner.temp`.
- `.github/actions/nextest` reads the version, sha256 and URL that
  `scripts/nextest.sh pin` prints, gets the archive through
  `pinned-download`, and runs `scripts/nextest.sh install`, which checks it
  again.
- `release.yml` runs the script for cargo-cyclonedx without the cache: a
  release runs on a tag, and a cache entry a tag run saves is readable only
  by that tag.
- `pages.yml` runs the script for Zola without the cache. It runs on
  `workflow_run`, in `main`'s context with `main`'s cache scope, so code it
  checks out must not write an entry that `main`'s jobs restore (CodeQL's
  `actions/cache-poisoning`).
- The how-to runner's mirror fetches each asset a page downloads with the
  same script (see [the how-to pages are tested](#the-how-to-pages-are-tested)).
- `scripts/check_workflow_downloads.py`, run by CI's scan job and the commit
  hook, fails a workflow or action step that names a `releases/download/`
  URL and does not go through the action or the script, a step that also
  runs `curl` or `wget` on one, and a script (other than a `scripts/test-*.sh`
  test) that names one and runs `curl` or `wget`. It also fails the combined
  `actions/cache` (which saves on every branch), an `actions/cache/save`
  whose `if:` is not main-only, any cache in a workflow run by
  `workflow_run`, a `pinned-download` action whose key or
  steps break the shape above, and a reading that finds fewer than 10
  release downloads through the rule.
- `scripts/test_fetch_pinned.py` drives the script against a local HTTP
  server (504s, stalls, cut-short transfers, redirects, substituted bytes)
  and `file://` fixtures.

To bump a tool in a workflow:

1. Change its `_VERSION` value. cargo-nextest's version and hashes are in
   `scripts/nextest.sh`; cargo-fuzz's are in
   `.github/actions/fuzz-setup/action.yml`.
2. Read the new asset's sha256 from GitHub, which computes one for every
   release asset:

   ```sh
   gh api repos/<owner>/<repo>/releases/tags/<tag> --jq '.assets[] | .name + " " + .digest'
   ```

   Where the release publishes a checksum file, compare the two.
3. Put the hash in the matching `_SHA256` value. The new version and hash
   are a new cache key, so the first run on `main` fetches and saves the
   asset.

To add a vcon-server release the how-to pages download, once it is
published (before then the scenarios use the newest published release), add
its four assets to `scripts/docs-test-pins.txt`: `SHA256SUMS.txt`,
`SHA256SUMS.txt.sigstore.json` and the x86_64 and aarch64 archives. Read the
hashes as in step 2, with `NormB/vcon-server` and the `v` tag, and check the
archives' against the release's `SHA256SUMS.txt`:

```sh
gh release download v<version> --repo NormB/vcon-server -p SHA256SUMS.txt -O -
```

A run for a release with no pins stops before its first page and names the
file. A third-party tool a page downloads takes the same steps, and the
page's own hash for it must be the pin's, or `lint` fails.

To bump an image the how-to scenarios use, read the digest of its
multi-platform index, the `Digest:` line, and put it in `IMAGES` in
`scripts/docs_test.py`:

```sh
docker buildx imagetools inspect debian:trixie-slim
```

During a download outage:

- A step that fails with exit 1 from `fetch-pinned.sh` failed on the
  network. Re-run it once a manual `curl` of the URL succeeds:

  ```sh
  curl -sS -o /dev/null -w '%{http_code}\n' -L "<the URL in the error>"
  ```

- A job whose asset is in the cache does not reach github.com, so an outage
  reaches only pins that `main` has not yet saved.
- An exit 3 means the bytes served differ from the pin. Do not re-run.
  Download the asset and compare its hash with the release's own checksum
  file before changing the pin.
- A warning that a cached file does not match its pin names the entry.
  Delete it with `gh cache delete <key>`, because an entry is never
  overwritten.

## Adding a GitHub Action

A tag or a branch is a name its owner can move to other code at any time; a
commit id is not. So every `uses:` in `.github/workflows/` and in the
composite actions under `.github/actions/` is one of two forms, and
`scripts/check_action_pins.py` refuses any other:

- `./.github/actions/<name>` or `./.github/workflows/<file>.yml`: an action
  or reusable workflow in this repository, with no `@ref`. The path must
  exist.
- `owner/repo@<40-hex commit> # v<version>` (or
  `owner/repo/<path>@<40-hex commit> # v<version>` for an action in a
  subdirectory, or a reusable workflow): the full commit id, in lower case,
  and the tag it was taken at as a comment. Dependabot updates the two
  together.

It refuses a tag, a branch, a short commit id, a `docker://` image (by tag
or by digest), a commit without the `# v<version>` comment, a YAML alias in
place of the value, a value not on the `uses:` line, and `uses:` in a flow
mapping (`{ ... }`). It does not read comment lines or the contents of a
block such as `run: |`.

To add an action:

1. Choose a release at least 14 days old (see
   [Contributing](#contributing)) and find the commit its tag names:

   ```sh
   gh api repos/actions/checkout/commits/v7 --jq .sha
   ```

   For an annotated tag this gives the commit the tag points to, which is
   the one to pin.
2. Write the step with that commit and the tag as a comment:

   ```yaml
   - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7
   ```

3. Check it, and run the check's own tests:

   ```sh
   python3 -I scripts/check_action_pins.py
   python3 -I scripts/test_check_action_pins.py
   ```

   The check prints `check_action_pins: ok (<n> uses in <m> files)`, or one
   line per refused `uses:` as `<file>:<line>: <reason>`, and exits 1.

If the check refuses a line: `@<ref> is not a commit` means a tag, a branch
or a short id (look up the full id as in step 1), or a `#` with no space
before it, or inside the quotes, which YAML reads as part of the value;
`no # v<version> comment` means the comment is missing or is not of the
form `v1`, `v1.2` or `v1.2.3` (a pre-release such as `v1.2.3-rc.1` is
accepted); `is not an action or workflow file in the repository` means a
local path that does not exist or leaves the repository.

## Contributing

- **A bug or a feature request:** open an issue at
  <https://github.com/NormB/vcon-server/issues>. For a bug, give the output
  of `vcon-server --version`, the commit you built, what you did, what you
  expected and what happened, and the log lines with every token removed.
- **A change:** send a pull request against `main`. Read
  [CONTRIBUTING.md](https://github.com/NormB/vcon-server/blob/main/CONTRIBUTING.md)
  first, and sign the [Contributor License Agreement](@/cla.md) before
  your first pull request merges.
- **A new dependency or version:** take it only once that version has been
  public for 14 days. Dependabot waits the same 14 days (`cooldown` in
  `.github/dependabot.yml`), and `scripts/check_dependabot.py` fails on an
  entry that waits less.
- **A security problem:** report it privately, never in an issue; see
  [reporting a vulnerability](@/docs/security.md#reporting-a-vulnerability).

## License

MIT OR Apache-2.0, at your option.
