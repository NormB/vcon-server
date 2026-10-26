+++
title = "Development"
description = "The code's layout, how it is tested, the gates every commit passes, and how to work on this site."
weight = 90
+++

## Layout

```text
crates/
├── vcon-form/      reads a vCon without changing it: form, gzip, uuid, critical
│                   (no I/O; returns no bytes; the IETF examples are its fixtures)
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

## Gates

Enable the repository's hooks once per clone:

```sh
git config core.hooksPath .githooks
```

- **pre-commit** runs `cargo fmt --check`, `cargo clippy -D warnings` and the
  tests on exactly what is staged, and refuses personal data and AI-tool
  attribution in staged changes.
- **commit-msg** refuses the same in the message.
- **pre-push** refuses any outgoing commit that is not signed by an allowed
  signer, or whose author and committer are not the pusher's configured
  identity.

`scripts/test-hooks.sh` and `scripts/test-pre-push.sh` test the hooks
themselves. Every commit on `main` is signed and shows as verified on GitHub.

Test data uses reserved values only: `example.com` addresses, 555-01xx
numbers, RFC 5737 and RFC 3849 addresses. The one exception is the IETF
draft's own examples, kept unchanged under
`crates/vcon-form/tests/fixtures/ietf/`.

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
browser tests; the scan for AI
attribution and personal data over every tracked file; `scripts/check-site.sh`;
and `scripts/icons.py --check`. A coverage job runs the same tests under
`cargo llvm-cov` and sends the result to Codecov. Each action and each
downloaded tool is pinned by commit or by sha256.

## Licence

MIT OR Apache-2.0, at your option.
