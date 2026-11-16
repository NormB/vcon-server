# Changelog

All notable changes to this project are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- **The site and the READMEs say this project is not the vCon Conserver.**
  - Effect: A note under the home page's lead, in both READMEs and on the
    Producers page links to the Conserver at
    [github.com/vcon-dev/vcon-server](https://github.com/vcon-dev/vcon-server).
  - Docs: [Producers](https://vcon-server.com/docs/producers/)

## [0.4.7] - 2026-11-16

### Changed

- **The upgrade from 0.4.6 is the binary alone.**
  - Effect: No stored data, configuration or schema changes; 0.4.6 opens a
    data directory that 0.4.7 has served.
  - Docs: [From 0.4.6 to
    0.4.7](https://vcon-server.com/docs/upgrading/#from-0-4-6-to-0-4-7)
- **The audit log has a new action, `read_masked`.**
  - Effect: Each masked view read is recorded as `read_masked` in the
    tenant's log; a reader that matches a fixed list of actions needs it.
  - Do: Add `read_masked` to any alert, report or filter that lists actions.
  - Docs: [Audit log
    records](https://vcon-server.com/docs/audit-log-records/#every-action)
- **The library's `audit::Action` has a new variant, `ReadMasked`.**
  - Effect: Code that matches every `audit::Action` variant no longer
    compiles until it handles `ReadMasked`.
  - Do: Add an arm for `Action::ReadMasked`.

### Added

- **A vCon's masked view: `GET /vcons/{uuid}?view=masked`.**
  - Effect: The server answers a vCon's form, counts and masked parties as
    JSON, with no name, subject, full number, address or content.
  - Limit: The legal routes refuse `view`; it needs the `read` scope.
  - Docs: [Masked view](https://vcon-server.com/docs/http-api/#masked-view)
- **The terminal UI shows a vCon's masked view and reveals one part on a key.**
  - Effect: Enter shows the masked view; a reveal key shows the parties,
    subject, dialog, analysis, links or the whole vCon.
  - Limit: Each reveal is recorded in the audit log; the server must offer
    the masked view.
  - Docs: [Reveal a part of a
    vCon](https://vcon-server.com/docs/tui/#reveal-a-part-of-a-vcon)

### Fixed

- **The `serving` line always comes before the first purge sweep's line.**
  - Was: The first sweep, pack run or rotation warning could be logged
    before `serving`.
  - Effect: A start logs `serving` first, then the background jobs' lines.
  - Docs: [Logs](https://vcon-server.com/docs/operations/#logs)

## [0.4.6] - 2026-11-14

### Changed

- **Upgrading from 0.4.5: a data-directory deployment replaces the binary
  only.**
  - Effect: The catalogs stay at schema version 9, and a downgrade is
    possible.
  - Do: With `catalog.backend = "postgres"` set on 0.4.5, run
    `vcon-server migrate` before the first start.
  - Limit: That database is then at schema version 13 and holds no tenants.
  - Docs: [From 0.4.5 to
    0.4.6](https://vcon-server.com/docs/upgrading/#from-0-4-5-to-0-4-6)
- **The release binaries are about 2 MB larger with the PostgreSQL catalog.**

  | Target | 0.4.5, as published | Built with the catalog |
  |---|---|---|
  | `x86_64-unknown-linux-gnu` | 50,457,584 bytes | 52,499,224 bytes |
  | `aarch64-unknown-linux-gnu` | 47,734,568 bytes | 49,603,432 bytes |
- **`cargo install` gets the PostgreSQL catalog with `--features postgres`.**
  - Effect: `cargo install vcon-server` and a build from source build the
    default features, as before, without the preview.
  - Do: Add `--features postgres` to either command to build it in.
  - Docs: [install with
    cargo](https://vcon-server.com/docs/install/#install-with-cargo)
- **The terminal UI page lists the keys once.**
  - Effect: One table names every key and the views it works in; the
    command is unchanged.
  - Docs: [Keys](https://vcon-server.com/docs/tui/#keys)

### Added

- **The catalogs can be kept in PostgreSQL, a preview the release binaries
  include.**
  - Effect: With `catalog.backend = "postgres"`, a server keeps its registry
    and every tenant's catalog in a PostgreSQL database.
  - Limit: One server serves a database, and a database starts with no
    tenant: no command moves a data directory's tenants.
  - Docs: [the limits of the
    preview](https://vcon-server.com/docs/postgres-catalog/#the-limits-of-the-preview)

### Fixed

- **`vcon-server migrate` frees the database before it returns.**
  - Was: A server started the moment `migrate` returned could be refused by
    the migration's locks.
  - Effect: `migrate` gives up both locks before it returns, also when it
    fails.
  - Limit: Only the PostgreSQL preview is affected.
  - Docs: [make the
    schema](https://vcon-server.com/docs/postgres-catalog/#make-the-schema)
- **The PostgreSQL preview's pages no longer overstate row-level security.**
  - Was: The pages said a query with no tenant set is refused; a cached
    statement that examines no row returns nothing, with no error.
  - Effect: The pages say what holds for every plan: with no tenant set,
    no row is returned or changed.
  - Limit: The schema is unchanged; no row was readable without a tenant
    before or after.
  - Docs: [keep the catalogs in
    PostgreSQL](https://vcon-server.com/docs/postgres-catalog/)

## [0.4.5] - 2026-11-12

### Changed

- **Upgrading from 0.4.4 replaces the binary only, and a downgrade is
  possible.**
  - Effect: No stored data and no schema changes: the catalogs stay at
    schema version 9, the PostgreSQL preview's schema at version 6.
  - Do: Keep the 0.4.4 configuration: it is read as it was.
  - Limit: Three things look different after the upgrade; each is an entry
    of this section.
  - Docs: [From 0.4.4 to
    0.4.5](https://vcon-server.com/docs/upgrading/#from-0-4-4-to-0-4-5)
- **A new key's expiry is a whole second.**
  - Effect: `expires_at` of a key made by 0.4.5 has no fraction of a second,
    such as `2027-02-10T12:00:00Z`.
  - Do: Keep reading an `expires_at` with a fraction: keys made before keep
    the expiry they have.
  - Limit: A key lives up to one second less than the days asked, and never
    longer.
- **The terminal UI page shows every screen.**
  - Effect: The page has a table of the 25 screens the command draws and a
    section for each.
  - Docs: [terminal UI](https://vcon-server.com/docs/tui/)
- **A GitHub release's text is its changelog section.**
  - Effect: A release's text is the section of its version, then the line
    that says how to verify a download.

### Fixed

- **The terminal UI's last line names only the keys that act.**
  - Was: The connecting screen, an empty tenant and every failure showed
    the list's keys, which did nothing there.
  - Effect: On each screen of `vcon-server tui` the last line lists the keys
    that do something in the state shown.
  - Docs: [terminal UI](https://vcon-server.com/docs/tui/)
- **Two failure screens of the terminal UI say what can be done.**
  - Was: For an operator key, and for a key without the `read` scope, the
    screen said `Press r to try again, or q to quit.`.
  - Effect: The screen says `Press q to quit.`, since `r` cannot read
    another key.
  - Limit: While a message stands in place of the list, the keys that
    select and open do nothing.

## [0.4.4] - 2026-11-12

### Changed

- **No downgrade from 0.4.4: copy the data directory before the upgrade.**
  - Effect: 0.4.4's first open of a data directory brings the catalogs from
    schema version 4 to 9; a 0.4.3 configuration is read as it was.
  - Do: Copy the data directory, with the server stopped, before the
    upgrade.
  - Limit: Release 0.4.3 and earlier refuse a catalog at version 9: the copy
    is the only way back, and what is stored after it is lost.
  - Docs: [From 0.4.3 to
    0.4.4](https://vcon-server.com/docs/upgrading/#from-0-4-3-to-0-4-4)
- **The PostgreSQL catalog preview needs `vcon-server migrate` before a
  server starts.**
  - Effect: The database's schema goes from version 1 to 6, and only
    `vcon-server migrate` changes it.
  - Do: Run `vcon-server migrate` after the new binary is installed and
    before a server starts.
  - Docs: [Schema version 9, and no way back without the
    copy](https://vcon-server.com/docs/upgrading/#schema-version-9-and-no-way-back-without-the-copy)
- **A new key expires 90 days after it is made, unless its request says
  otherwise.**
  - Effect: This holds for `key create`, `POST /admin/tenants/{id}/keys`,
    `POST /tenant/keys` and the MCP tool `admin_key_create`.
  - Do: A script whose key is never replaced stops with 401 `key-expired`
    after 90 days: replace its key on a schedule, or use `--no-expiry`.
  - Limit: Keys made before the upgrade are not changed: they have no
    expiry and keep working.
  - Docs: [Keys made before the upgrade have no
    expiry](https://vcon-server.com/docs/upgrading/#keys-made-before-the-upgrade-have-no-expiry)
- **A request with both a key and the session cookie is refused.**
  - Effect: Every route answers 400 `key-and-session` to a request with an
    `Authorization` header and a cookie named `__Host-vcs_session`.
  - Limit: No release set that cookie, so a client is affected only if it
    sends a cookie of that name itself.
  - Docs: [From 0.4.3 to
    0.4.4](https://vcon-server.com/docs/upgrading/#from-0-4-3-to-0-4-4)
- **A reader of the audit log meets thirteen new actions and a new actor.**
  - Effect: The new actions are `user_add`, `user_list`, `user_deactivate`,
    `membership_grant`, `membership_revoke`, `role_cap_set`, `role_cap_read`,
    `settings_read`, `settings_set`, `membership_list`, `session_start`,
    `session_read` and `session_end`.
  - Do: Add them to a reader that knows a fixed list of actions, and read
    an `actor` of `user:` and a user id.
  - Limit: An `auth_failed` record may have the status 400 or 403.
  - Docs: [Audit log
    records](https://vcon-server.com/docs/audit-log-records/)
- **Code that uses the `vcon-server` library sets new members and matches
  new variants.**
  - Effect: `config::Config` gains `public_origin` and `keys_no_expiry`,
    `config::Operator` `public_origin`, `api::App` `origin`;
    `catalog::SCHEMA_VERSION` is 9.
  - Do: Set them in a struct literal, and take `audit::Actor::User` and the
    thirteen new `audit::Action` variants in a `match` that lists every
    variant.
- **`vcon-server tui` says a change to its files takes effect at the next
  start.**
  - Effect: When a request fails, the screen adds `A change to the client
    file or the key file takes effect at the next start.`
  - Limit: The command reads both files once, at start, so `r` never reads
    a corrected file.
  - Docs: [Terminal UI](https://vcon-server.com/docs/tui/)
- **The terminal UI page is rewritten, and shows sixteen screens.**
  - Effect: It has a quick start, a section and a screen for each task,
    reference tables, and each message with its cause and what to do.
  - Docs: [Terminal UI](https://vcon-server.com/docs/tui/)

### Security

- **The bundled SQLite moves to 3.54.1 (CVE-2026-119375,
  RUSTSEC-2026-0459).**
  - Was: SQLite before 3.54.0, bundled by 0.4.3 and earlier, can return
    uninitialized page-cache memory beside a `-wal` file of another page
    size.
  - Effect: rusqlite goes from 0.41.1 to 0.42.0, SQLCipher from 4.19.0 to
    4.20.0, SQLite from 3.53.4 to 3.54.1; the file format does not change.
  - Limit: The vector is local: it needs someone who can write a crafted
    pair of files into the data directory.
  - Do: Code that uses the library builds with rusqlite 0.42.0 or later,
    which the crate now requires.

### Added

- **Users and memberships, made and changed by five new commands on the
  host.**
  - Effect: `user add`, `user list`, `user deactivate`, `membership grant`
    and `membership revoke` run beside a running server.
  - Limit: Nobody can sign in as a user: no route reads them and no sign-in
    method exists yet; API keys work as before.
  - Docs: [Add users and
    memberships](https://vcon-server.com/docs/users-and-memberships/)
- **Roles give scopes, and a tenant's role cap limits the roles it grants.**
  - Effect: Each role gives one scope, `tenant-admin` the new scope
    `tenant_admin`, and `user list` prints each membership's `scopes`
    beside its `roles`.
  - Do: Set the cap with `tenant set-role-cap` and print it with
    `tenant role-cap`, on the host beside a running server.
  - Limit: A grant outside the cap is refused, and a role granted before
    the cap stops counting until the cap allows it again.
  - Docs: [Roles, scopes and a tenant's role
    cap](https://vcon-server.com/docs/roles-and-scopes/)
- **A tenant key with `tenant_admin` administers the tenant's settings and
  keys.**
  - Effect: It reaches `GET` and `PUT /tenant/settings`, `POST` and
    `GET /tenant/keys`, `DELETE /tenant/keys/{id}` and
    `GET /tenant/memberships`, in its own tenant.
  - Do: Give a key the scope with `key create --scope` or
    `POST /admin/tenants/{id}/keys`; the console's key form offers it.
  - Limit: An operator key is refused, and a key made at
    `POST /tenant/keys` stays within the role cap, never `tenant_admin` or
    `admin`.
  - Docs: [Administer a tenant with a
    key](https://vcon-server.com/docs/tenant-administration/)
- **Sessions and their CSRF protection, the fourth part of sign-in.**
  - Effect: `GET /session` and `POST /session/logout` take the cookie
    `__Host-vcs_session` and no key; nothing outside the tests starts a
    session yet.
  - Do: Set `public_origin`, and `operator.public_origin` for the operator
    listener, with different hosts; a listener without it serves no
    session.
  - Limit: The routes that take a key take no session yet.
  - Docs: [Set a listener's public origin for
    sessions](https://vcon-server.com/docs/session-origin/)
- **Keys show their expiry and last-used time; an operator can require an
  expiry.**
  - Effect: `key list`, `GET /admin/tenants/{id}/keys`, `GET /tenant/keys`,
    `admin_key_list` and the console's key table show `expires_at` and
    `last_used_at`.
  - Do: Choose 1 to 365 days with `key create --expires-days`,
    `expires_in_days` or the console's key form; set `keys.no_expiry` to
    `false` to refuse keys with no expiry.
  - Limit: The last-used time is the start of the minute, written at most
    once a minute; a failure to write it refuses no request.
  - Docs: [Set a key's expiry and see when it was last
    used](https://vcon-server.com/docs/tenant-administration/#set-a-key-s-expiry-and-see-when-it-was-last-used)

### Fixed

- **The terminal UI page quotes the 503 message the UI shows.**
  - Was: The page said a refused list shows `the server answered 503
    (unavailable)`.
  - Effect: It quotes the problem's title, as the UI shows it:
    `the server answered 503 (The service cannot answer now)`.
  - Docs: [Terminal UI](https://vcon-server.com/docs/tui/)
- **`vcon-server tui` keeps `? help` and `q quit` on a narrow terminal's
  last line.**
  - Was: In a terminal narrower than its last line (59 columns in the list,
    49 in the detail view), the key hints were cut.
  - Effect: The last line always ends with `? help` and `q quit`, and leaves
    out, whole, the hints that do not fit.
- **`vcon-server tui` wraps the line above the key hints.**
  - Was: A failed next page showed `The next page was not read: the server
    did not answer: error sending request for`, cut, with no cause.
  - Effect: The line wraps, up to half of the screen, where it was one line
    cut at the terminal's edge.
- **`vcon-server tui` says `not connected` after its key is revoked.**
  - Was: A key revoked while the command ran left the first line saying
    `connected`, with the tenant, above `the server refused the key (401)`.
  - Effect: The first line says `not connected`, as at start, and `r` asks
    for the key again.

## [0.4.3] - 2026-11-11

### Changed

- **An upgrade from 0.4.2 is the binary alone.**
  - Effect: The catalogs keep their schema version, and 0.4.2 opens a data
    directory that 0.4.3 has served.
  - Do: Keep the 0.4.2 configuration: it is read as it was.
  - Limit: Five changes can break something around a 0.4.2 deployment, and
    one breaks code that uses the library; each is an entry of this section.
  - Docs: [From 0.4.2 to
    0.4.3](https://vcon-server.com/docs/upgrading/#from-0-4-2-to-0-4-3)
- **A reader of the audit log meets seventeen new actions.**
  - Effect: The fourteen reveal actions, `reveal_<part>` and
    `legal_reveal_<part>`, and `wrong_listener`, `listener_full` and
    `csp_report` are new.
  - Do: Before installing the binary, add them to any rule, parser or report
    that lists actions.
  - Limit: A `listener_full` record has `null` as its `status`, and a server
    without an `[operator]` table never writes `wrong_listener`.
  - Docs: [Audit log
    records](https://vcon-server.com/docs/audit-log-records/)
- **The console's Reveals are recorded as reveals, where 0.4.2 wrote reads.**
  - Effect: The tenant log holds `reveal_<part>` where 0.4.2 wrote `read`,
    and `legal_reveal_<part>` where it wrote `legal_read`.
  - Do: To count every read of content, count the `reveal_` actions with
    `read`, and the `legal_reveal_` actions with `legal_read`.
  - Limit: Opening a vCon and **Download** are still `read`.
  - Docs: [Reveals](https://vcon-server.com/docs/audit-logs/#reveals)
- **The console has three new paths, and its policies a reporting
  directive.**
  - Effect: `/console/player.html`, `/console/player.js` and
    `POST /console/csp-report` are new; both policies end with
    `report-uri /console/csp-report`.
  - Do: In a proxy that passes the console's paths by name, pass all three,
    and pass each `Content-Security-Policy` header unchanged.
  - Limit: A check that holds either policy byte for byte needs the new
    value; a proxy that passes all of `/console/` needs no change.
  - Docs: [From 0.4.2 to
    0.4.3](https://vcon-server.com/docs/upgrading/#from-0-4-2-to-0-4-3)
- **`/openapi.json` lists the answers of a server with an operator
  listener.**
  - Effect: It names the problem type `wrong-listener` and lists 403 for
    `GET /health`, `GET /key` and `GET /openapi.json`.
  - Limit: Those three routes answer 403 only on a server with an operator
    listener.
  - Docs: [Operator listener](https://vcon-server.com/docs/operator-listener/)
- **Code that builds `config::Config` or `api::App` sets their new
  members.**
  - Effect: `config::Config` has `operator`, `connection` and `reports`;
    `api::App` has `plane` and `reports`, an `api::ReportGate` made from
    `api::ReportLimits`.
  - Do: Set them in a struct literal; `None` for `operator` and
    `access::Plane::Both` for `plane` keep the behavior as it was.
- **Code that matches `audit::Action` or names the type of
  `console::FILES` changes.**
  - Effect: `audit::Action` has five new variants, `Reveal`, `LegalReveal`,
    `WrongListener`, `ListenerFull` and `CspReport`; `console::FILES` is
    `[File; 6]`, where it was `[File; 4]`.
  - Do: Take the five variants in a `match` that lists every variant.
  - Limit: `api::serve` takes any `api::Accept` (`accept`, and `local` with
    a default), which `tokio::net::TcpListener` implements; no public item
    is removed.
- **The documentation's sidebar folds behind one button on a narrow
  screen.**
  - Effect: A page's own content comes first; without JavaScript the
    sidebar is shown as before.
- **Three documentation pages each keep to one kind of writing.**
  - Effect: The audit log's record members and actions, and the steps for
    reading a log, are on two new pages.
  - Limit: Every heading the three pages had is still there, so a link to
    one still lands on it.
  - Docs: [Audit log records](https://vcon-server.com/docs/audit-log-records/),
    [Read the audit logs](https://vcon-server.com/docs/read-audit-logs/),
    [Connect an AI agent over MCP](https://vcon-server.com/docs/connect-mcp/)
- **The `parquet` crate goes from 60.0.0 to 60.1.0.**
  - Effect: It writes `export parquet` files and Parquet storage's pack
    files; the export round trip, DuckDB reads and Parquet storage on S3
    pass unchanged.
- **A build from source fetches from crates.io alone.**
  - Effect: rusqlite goes from 0.41.0 to 0.41.1, and the fork of rusqlite
    from github.com is gone.
  - Do: With a mirror of crates.io or `cargo vendor`, a build from source,
    tests included, needs no other host.
  - Limit: The catalogs are opened with the same settings as before.

### Added

- **An operator listener serves operator keys on an address of their own.**
  - Effect: With an `[operator]` table, `serve` also listens on
    `operator.listen`, for operator keys only, and `listen` serves tenant
    keys only.
  - Limit: A valid key at the other listener is refused with 403
    `wrong-listener`, on every route and on `/mcp`; without the table
    nothing changes.
  - Docs: [Operator listener](https://vcon-server.com/docs/operator-listener/)
- **The console plays a recording that a vCon holds inside a dialog.**
  - Effect: Each dialog whose `body` is `base64url` or `base64` and whose
    `mediatype` is `audio/wav`, `audio/mpeg`, `audio/ogg` or `audio/opus`
    has a **Play** button.
  - Limit: A recording is played up to 104,857,600 bytes; one at a link is
    never loaded, and `application/ogg` is not played.
  - Docs: [Play a
    recording](https://vcon-server.com/docs/console/#play-a-recording)
- **A how-to gets a Let's Encrypt certificate in place of the self-signed
  one.**
  - Effect: certbot gets it by HTTP-01 or DNS-01, and a deploy hook installs
    each new certificate and restarts the server; clients need no CA file.
  - Docs: [Get a Let's Encrypt
    certificate](https://vcon-server.com/docs/lets-encrypt/)
- **The terminal UI's page and the home page show its list and detail
  screens.**
  - Effect: The screens are drawn by the UI's own code.
  - Docs: [Terminal UI](https://vcon-server.com/docs/tui/)
- **A read of a vCon can be recorded as a reveal of one part.**
  - Effect: `GET /vcons/{uuid}?reveal=<part>`, also on
    `GET /legal/vcons/{uuid}` and
    `GET /legal/vcons/{uuid}/versions/{version}`, records `reveal_<part>` or
    `legal_reveal_<part>`.
  - Do: Name the part: `parties`, `subject`, `dialog`, `analysis`, `links`,
    `all` or `audio`.
  - Limit: The answer and scope are those of the read without it; a
    `reveal` given twice, beside another parameter or with no part is
    `bad-query` (400).
  - Docs: [Reveals](https://vcon-server.com/docs/audit-logs/#reveals)
- **A browser reports a broken console policy to `POST /console/csp-report`.**
  - Effect: The route takes no key, stores and logs nothing of a report, and
    answers 204; each request is counted as `csp_report`.
  - Do: Bound a listener's reports a minute with `limits.csp_reports`
    (default 120) and, from one source address,
    `limits.csp_reports_per_address` (default 20).
  - Limit: A request past either is answered 429 with a `Retry-After`
    header and no write of its own.
  - Docs: [Reports of a broken
    policy](https://vcon-server.com/docs/console/#reports-of-a-broken-policy)
- **The `vcon-server` library has new items for connection limits, reports
  and the terminal.**
  - Effect: `api::ConnectionLimits` (`max`, `handshake`, `first_head`,
    `head`, `idle`, `goodbye`, `accept_pause`), `api::serve_within`,
    `api::serve_observed`, `console::REPORT_PATH`, `audit::count_many` and
    `tui::with_terminal` are new.
  - Do: Call `api::serve_within` for `api::serve` with limits, and
    `api::serve_observed` to have a function called each time a listener
    fills.
  - Limit: `console::router()` answers a report at `REPORT_PATH` with 204
    and counts nothing.

### Fixed

- **The library page's example passes `application/vcon` to
  `vcon::read::inspect`.**
  - Was: The `vcon` crate's README and the library page passed
    `application/vcon+json`, which the server refuses with
    `unsupported-media-type`.
  - Effect: The example passes `application/vcon`; the vCon draft registers
    `application/vcon` and `application/vcon+gzip`.
  - Docs: [The vcon library](https://vcon-server.com/docs/library/)
- **The site's pages no longer fetch the search index before a search.**
  - Was: Each page fetched the search index, a file of over a megabyte,
    before the search box was used.
  - Effect: A page fetches the index once its search box is used.
- **The documentation lists its pages under five group headings.**
  - Was: Five pairs of pages shared a place in the order.
  - Effect: The sidebar and index start with install, getting started and
    the first vCon, and no two pages share a place.
- **Each how-to guide says what to do when a step fails.**
  - Effect: `llms.txt` links every page once, and between releases the
    status page lists what the newest release does not hold yet.
- **The site uses US spelling, and lists `GET /key` among any key's
  routes.**
  - Was: Seven words were not in US spelling, and the tenants-and-keys page
    left `GET /key` out of the routes any valid key may ask.
  - Effect: The seven words are in US spelling, and the page lists
    `GET /key`.
- **The PostgreSQL preview's stop message says which tables to remove.**
  - Was: It said to set `catalog.backend = "local"`, and a configuration
    with that and the `[catalog.postgres]` table is refused.
  - Effect: It says to remove the `[catalog]` and `[catalog.postgres]`
    tables.
  - Limit: Only a build with the `postgres` feature prints it.
- **The upgrade and operations pages put root's copies in
  `/backup/stopped`.**
  - Was: They made `/backup` a directory for root alone
    (`install -d -m 0700 /backup`), so
    `vcon-server backup /backup/vcon-server/...` run as `vcon` was refused.
  - Effect: The copies root makes go in `/backup/stopped`.
  - Docs: [Back up and
    restore](https://vcon-server.com/docs/backup-and-restore/)
- **Three pages are corrected: `check`'s members, a held tenant's removal,
  the source layout.**
  - Was: The tenants-and-keys page said a tenant under a legal hold is
    refused with `under-hold` whatever the query.
  - Effect: It says that without `force=true` the answer is `holds-vcons`;
    the operations and development pages have the four members and seven
    files they left out.
- **The site's search box edge and focus rings are fixed.**
  - Was: The search box's edge had a contrast of 1.2 to 1 against the page;
    WCAG 1.4.11 asks 3 to 1.
  - Effect: The focus ring of the sidebar's links and of the home page's
    code blocks is no longer cut off by the box around them.
- **A listener holds at most `limits.connections` connections at once.**
  - Was: A listener took every connection it was offered, so connections
    alone could use up the process's file descriptors.
  - Effect: At 512, or the number set from 1 to 65536, a new connection
    waits in the system's queue until one ends.
  - Do: Keep the number below the process's limit on open files.
  - Limit: 0.4.2 refuses a configuration that has one of this release's five
    new `limits` keys.
  - Docs: [Connection
    limits](https://vcon-server.com/docs/configuration/#connection-limits)
- **A connection that waits for its next request is closed after
  `limits.timeouts.idle` seconds.**
  - Was: An HTTP/2 connection that had sent one request was kept for as
    long as the other side kept it.
  - Effect: The limit is 120 seconds unless set from 1 to 3600, for HTTP/2
    with no stream open and HTTP/1.1 between requests.
  - Do: Set `idle = 30` in `[limits.timeouts]` to close an idle connection
    after 30 seconds.
  - Limit: No limit runs while a request is being answered; a client finds
    the connection closed and opens another.
  - Docs: [Connection
    limits](https://vcon-server.com/docs/configuration/#connection-limits)
- **A connection whose TLS handshake or request head stalls is closed.**
  - Was: A stalled connection was kept for as long as the other side kept
    it, with its file descriptor.
  - Effect: The limits are 10 seconds for the handshake, 30 for the first
    request head and 30 for an HTTP/1.1 request head.
  - Do: Change them with `limits.timeouts.handshake`,
    `limits.timeouts.first_head` and `limits.timeouts.head`, each from 1 to
    3600 seconds.
  - Docs: [Connection
    limits](https://vcon-server.com/docs/configuration/#connection-limits)
- **The server no longer stops when the system cannot hand it a
  connection.**
  - Was: One failed accept, such as one a process out of file descriptors
    gets, ended `serve` with `serving failed`.
  - Effect: The listener goes on, at once after a connection's own failure
    and after one second otherwise, with a warning in the log.
- **A stop signal right after the `serving` line stops the server in
  order.**
  - Was: The server took SIGTERM and SIGINT only after that line, so a
    supervisor that stopped it at once could find it killed by the signal.
  - Effect: It stops with exit status 0: `serve` takes both signals before
    it opens the data directory.
  - Limit: A server that cannot take them does not start:
    `the stop signals cannot be taken`.

### Security

- **rustix goes from 1.1.5 to 1.1.6 (RUSTSEC-2026-0486).**
  - Was: Its extended-attribute functions, given a zero-length buffer,
    marked unwritten bytes as initialized: undefined behavior from safe
    code.
  - Effect: The affected version leaves the build; vcon-server takes rustix
    through crossterm, for the terminal UI, and tempfile, in tests.
  - Limit: No source file of vcon-server, crossterm 0.29.0 or tempfile
    3.27.0 names an extended-attribute function.

## [0.4.2] - 2026-11-10

### Changed

- **An upgrade from 0.4.1 is the binary alone.**
  - Effect: Nothing in this release breaks a 0.4.1 deployment; the catalogs
    keep their schema version, and 0.4.1 opens a data directory 0.4.2 has
    served.
  - Docs: [From 0.4.1 to
    0.4.2](https://vcon-server.com/docs/upgrading/#from-0-4-1-to-0-4-2)
- **The console opens a vCon masked, and each reveal reads it again.**
  - Effect: A summary shows its form, counts and each party as a number's
    last four digits or an address's domain; a reveal shows the text.
  - Do: In a proxy that passes the console's paths by name, also pass its
    fourth file, `/console/format.js`.
  - Limit: The HTTP API is as it was: `GET /vcons/{uuid}` and the legal
    reads answer as in 0.4.1.
  - Docs: [Console](https://vcon-server.com/docs/console/)
- **The `vcon-server` crate's default features now include `tui`.**
  - Effect: A build with default features compiles ratatui 0.30.2 and,
    through it, crossterm 0.29.0.
  - Do: Depend on the crate with `default-features = false` to build it as
    before, without them.
  - Limit: The library's public items are all still there; `tui`,
    `commands::migrate`, `console::FILES`, `console::File` and
    `console::path_of` are new.
- **The PostgreSQL catalog preview needs `vcon-server migrate` before it
  connects.**
  - Effect: With `catalog.backend = "postgres"`, a connection refuses a
    database without the schema, where 0.4.1 connected and stopped.
  - Limit: `migrate` refuses a configuration whose `catalog.backend` is
    `"local"`, and neither release serves from PostgreSQL.
  - Docs: [Make the
    schema](https://vcon-server.com/docs/postgres-catalog/#make-the-schema)

### Added

- **`vcon-server tui` lists the vCons of one key's tenant in a terminal.**
  - Effect: It shows each one's uuid, version and times, as a client of a
    running server's HTTP API with a `read` key from a client file.
  - Do: Use the release binaries, `cargo install` or a build from source:
    the new cargo feature `tui` is on by default.
  - Limit: It reads only, and never a vCon's content; a build made with
    `--no-default-features` says it has no terminal UI.
  - Docs: [Terminal UI](https://vcon-server.com/docs/tui/)
- **`vcon-server migrate` makes the PostgreSQL catalog preview's schema.**
  - Effect: It makes the schema as `vcon_owner`, one transaction per schema
    version, alone: it takes the locks every server and command shares.
  - Limit: A connection refuses any schema version but the binary's; a
    server refuses a role that owns the catalog or can act as its owner.
  - Docs: [Make the
    schema](https://vcon-server.com/docs/postgres-catalog/#make-the-schema)
- **The library has a column codec for the PostgreSQL catalog.**
  - Effect: `envelope::column` seals a column under a per-row key derived
    from the catalog key, with AES-256-GCM, and finds values by
    HMAC-SHA256.

### Fixed

- **`vcon-server tui` keeps the connection whole in a narrow terminal.**
  - Was: In a terminal narrower than its first line, the server's address
    was the first part shortened.
  - Effect: The address loses `https://`, then the tenant's id is
    shortened, then the tenant is left out, and only then is the address
    shortened.
- **`vcon-server tui` without a terminal says `a terminal is required`.**
  - Was: It said `the terminal: No such device or address (os error 6)`.
  - Effect: It exits with code 1, says which of standard input and standard
    output is not a terminal, and checks before it reads the key file.
- **`vcon-server tui` says what is wrong with a server certificate it
  refuses.**
  - Was: It showed the TLS library's text.
  - Effect: It names a certificate authority's certificate (`CA:TRUE`), an
    untrusted signer, another name, an expired certificate or one not valid
    yet, and what to do.
  - Limit: Each is still refused before any request is sent.
  - Docs: [Terminal UI](https://vcon-server.com/docs/tui/)
- **The library's documentation has no link to a private item.**
  - Was: Two links from public documentation in `mcp/fence.rs` led to a
    private item.
  - Effect: The documentation builds with rustdoc's warnings denied.

## [0.4.1] - 2026-11-09

### Changed

- **Breaking: every secret file is held to one rule, and `serve` stops on
  a refusal.**
  - Effect: `master_key`, `tls.key`, the S3 and PostgreSQL secret files,
    the `rekey master --new-master` file and the MCP `key_file` are all
    checked.
  - Do: Before installing, run `stat -c '%a %U %n'` on each secret file,
    and `chmod 600` (or `chown` to the server's account) any that fails.
  - Limit: A file passes as a regular file owned by the process's user or
    root, with no bit of `0027`: `0600`, `0400` and `0640`.
  - Docs: [From 0.4.0 to 0.4.1: the secret-file
    rule](https://vcon-server.com/docs/upgrading/#from-0-4-0-to-0-4-1-the-secret-file-rule)
- **Code that matches the library's `KeyError` or `TlsError` takes a new
  arm.**
  - Effect: `KeyError` gains `Refused(String)` and `TlsError` gains
    `KeyFile(String)`; neither enum is `#[non_exhaustive]`.
  - Limit: A directory given as the master key is now `KeyError::Refused`,
    not `KeyError::Read`.

### Added

- **A PostgreSQL catalog preview, behind the `postgres` cargo feature, not
  on by default.**
  - Effect: `backend = "postgres"` in `[catalog]`, with a
    `[catalog.postgres]` table and `storage.objects = "s3"`, checks every
    key and the database at start.
  - Do: Build with `--features postgres`, which also turns on `s3`.
  - Limit: Then `serve` and every command stop with exit code 1: the
    catalogs are not kept in PostgreSQL yet.
  - Docs: [Keep the catalogs in
    PostgreSQL](https://vcon-server.com/docs/postgres-catalog/)

### Fixed

- **The S3 how-to waits until RustFS or MinIO answers `403`.**
  - Was: It waited for any answer, then expected `403`; a service still
    starting answers `503`, which ended the wait and failed the next step.
  - Effect: Each wait now ends on the `403`.
  - Docs: [Keep vCons in an S3
    bucket](https://vcon-server.com/docs/s3-storage/)

### Security

- **rmcp goes from 3.8.0 to 3.9.0 (RUSTSEC-2026-0462,
  GHSA-c9f9-cp8h-jcj5).**
  - Was: The client `vcon-server mcp` uses read an answer whole, so a
    server at its `url` could make the bridge buffer it out of memory.
  - Effect: 3.9.0 stops at 16 MiB for a JSON answer and 64 KiB for an error
    answer, and the bridge then stops with an error.
  - Do: Code that uses the library builds with rmcp 3.9.0 or later, which
    the crate now requires.

## [0.4.0] - 2026-11-06

### Changed

- **Every catalog moves to schema version 4, which release 0.3.0 does not
  open.**
  - Effect: A server brings an older catalog up to date; the new places for
    metadata levels and Parquet pack files stay empty until versions are
    packed.
  - Do: Copy the data directory before the upgrade: 0.3.0 opens neither
    version 3 nor 4, so the copy is the way back.
  - Docs: [From 0.3.0 to
    0.4.0](https://vcon-server.com/docs/upgrading/#from-0-3-0-to-0-4-0)
- **`serve` refuses to start without `[storage.parquet]` once a version is
  packed.**
  - Effect: The refusal names the table; without it the server packed no
    more and no longer refused a bucket with Object Lock or versioning.
  - Limit: `vcon-server check` gives each tenant's `packed` versions and
    reports them without the table as a problem; other commands run as
    before.
  - Docs: [Keep vCons as Parquet files in a
    bucket](https://vcon-server.com/docs/parquet-storage/)

### Added

- **Sealed vCon objects can be kept in a bucket of an S3-compatible
  service.**
  - Effect: With `storage.objects = "s3"` and `[storage.s3]`, RustFS, MinIO
    or AWS S3 holds each object; the catalogs and the master key stay in
    `data_dir`.
  - Do: Set `allow_versioning` for a bucket with versioning on, which is
    refused without it.
  - Limit: The first data directory to start on an empty bucket and prefix
    claims it (`<prefix>vcon-server.claim`), and any other is refused there.
  - Docs: [Keep vCons in an S3
    bucket](https://vcon-server.com/docs/s3-storage/)
- **`vcon-server storage move-objects` moves the sealed objects from the
  data directory to the bucket.**
  - Effect: Each object is written, read back and compared byte for byte
    before its file is removed, so a run that stops loses nothing.
  - Do: Run it while no server runs; a server that kept its objects
    locally then moves to Parquet storage with `[storage.parquet]`.
  - Limit: It prints, per tenant, how many moved and how many versions
    `check` then finds without an object.
  - Docs: [Move objects stored in the data
    directory](https://vcon-server.com/docs/s3-storage/#move-objects-stored-in-the-data-directory)
- **Parquet storage packs each tenant's objects in the bucket into Parquet
  files.**
  - Effect: With `[storage.parquet]`, a pack run at start and every
    `pack_interval` seconds (300 unless set) packs one store date at a time.
  - Do: Use `export parquet` for queryable columns: the files hold a
    version's id, store time and sealed bytes only.
  - Limit: An object over `pack_limit` (16 MiB unless set) stays an object;
    a bucket with versioning or Object Lock is refused at start.
  - Docs: [Keep vCons as Parquet files in a
    bucket](https://vcon-server.com/docs/parquet-storage/)
- **`vcon-server export parquet` copies a tenant's kept versions as Parquet
  files.**
  - Effect: `vcon-server export parquet --tenant <id> --to <dir |
    s3://bucket/prefix> [--since <time>] [--until <time>]` writes the files,
    then a manifest.
  - Do: Reach a bucket with the new `[export.s3]` table, or with
    `[storage.s3]`'s service and keys.
  - Limit: It runs beside a serving server and changes nothing in the store
    but one `export` record in the tenant's audit log.
  - Docs: [Export to
    Parquet](https://vcon-server.com/docs/export-parquet/)
- **Each tenant has a metadata level for its Parquet exports.**
  - Effect: `none`, the default, holds the version id, store time and sealed
    body; `counts` adds sizes, times and counts; `full` the uuid, subject
    and parties.
  - Do: Set it with `vcon-server tenant set-metadata` or
    `PUT /admin/tenants/{id}/metadata`; the tenant list shows it as
    `metadata`.
  - Limit: Statistics are written only for the time and count columns, and
    no column has a dictionary page.
  - Docs: [Export to
    Parquet](https://vcon-server.com/docs/export-parquet/)
- **The `vcon` crate reads the members an export copies, in
  `vcon::read::fields`.**
  - Effect: `created_at`, `updated_at`, `subject`, each party's `tel`,
    `mailto` and `name`, and the number of dialogs, also inside a signed
    vCon.
- **Start-up recovery logs how many objects it removed.**
  - Effect: `start-up recovery removed objects that no catalog row names
    removed=<n>`, for local files and for objects in a bucket.
  - Was: It removed them without a word.
- **The documentation covers MinIO and AWS S3, and more client examples.**
  - Effect: The S3 how-to has their setup, checks and troubleshooting, the
    services' required support for conditional writes, and every start-up
    refusal.
  - Do: Read the client examples for gzip, `If-Match`, paging, a legal
    hold, an audit read, an admin call and MCP.
  - Docs: The audit-logs page lists every action, the command line page has
    `serve`, and the library page shows `vcon::write`.

### Fixed

- **A purge sweep goes on when a version's object file is already gone.**
  - Was: It stopped with an I/O error after deleting the row, so the rest of
    the sweep and the WAL checkpoint after it did not run.
  - Effect: A missing object now counts as removed.
- **A read the storage fails to give answers `500 internal`, not
  `500 damaged`.**
  - Was: An unreadable object file, a key page not locked or an S3 service
    not reached answered `500 damaged`, and `check` listed the version as
    damaged.
  - Effect: The server logs `a stored version's object could not be read`
    with the failure's `kind`, never its text, a key or an id.
  - Limit: `damaged` is now only a missing object whose row is kept, or one
    that does not decrypt and authenticate.
- **`vcon-server check` says a tenant's objects cannot be read, not its
  catalog.**
  - Was: It reported `its catalog cannot be opened: Io`, though the catalog
    had opened.
  - Effect: It reports `its objects cannot be read:` with the failure's kind
    and error number, such as `permission denied (os error 13)`.
  - Limit: Any other failure of the examination is `it could not be
    examined:` with its error.
- **The crate and OpenAPI descriptions and the site use US spelling.**
  - Was: They spelled "authorised", "behaviour", "licence" and
    "honour(ed)".
  - Effect: Each is in US spelling, and the development page's "Licence"
    heading is now "License".

## [0.3.0] - 2026-11-04

### Added

- **An MCP server lets AI agents use the API with one key.**
  - Effect: `POST /mcp` on the TLS listener, and `vcon-server mcp --config
    <client file>` on stdin and stdout, offer 19 tools, each counted as
    action `mcp`.
  - Limit: Each call passes the key's rate, a free slot, a known tool, its
    scope and fitting arguments; a result holds at most `mcp.response_bytes`.
  - Docs: [MCP](https://vcon-server.com/docs/mcp/)
- **`vcon::write` builds vCons of the core draft, unsigned and gzipped.**
  - Effect: The output is compact JSON with members in the draft's order,
    and `read::inspect` reads it back with the same form, gzip flag and uuid.
  - Do: Use `Vcon::amend` and `Vcon::redact` to copy a vCon under a new uuid
    that links the prior one, and `uuid_v8` for the draft's uuid.
  - Limit: It refuses what core-04 does not allow and names the JSON path;
    `WriteError` implements `std::error::Error`.
  - Docs: [The vcon library](https://vcon-server.com/docs/library/)
- **`vcon::write` signs and encrypts vCons with `sign`, `sign_again` and
  `encrypt`.**
  - Effect: `sign` writes a General JWS JSON Serialization, `sign_again` up
    to `read::MAX_SIGNED_LAYERS` layers, and `encrypt` a General JWE of a
    signed vCon.
  - Do: Bring the cryptography through the `Signer`, `ContentEncrypter` and
    `KeyEncrypter` traits; a payload can be gzipped first (`Compression`).
  - Limit: The input is read under the caller's `read::Limits`, and every
    output reads back with `read::inspect`.
- **Each release holds a cosign signature and its build provenance.**
  - Effect: `SHA256SUMS.txt.sigstore.json` is a keyless signature over
    `SHA256SUMS.txt`, and `vcon-server-<version>.intoto.jsonl` the build
    provenance.
  - Limit: 0.2.0 and earlier have neither file.
  - Docs: [Check the signed
    checksums](https://vcon-server.com/docs/install/#check-the-signed-checksums)

### Changed

- **The release binaries run on Debian 12, Ubuntu 22.04 and AlmaLinux 9.**
  - Was: The 0.2.0 binaries needed glibc 2.38 and did not start there.
  - Effect: They are built on glibc 2.34 (AlmaLinux 9), run on all three
    before a release, and built reproducibly.
- **A failure to open `server.sqlite` at start says which step failed.**
  - Was: It said only `Io`.
  - Effect: It names the step, the file and the error underneath: a wrong
    key, an unfinished recovery, or the system's error.

### Fixed

- **The files the server writes are mode `0600` and its directories `0700`,
  whatever the umask.**
  - Was: Under umask 0002, `backup` wrote its files `0664` and its `tenants`
    directory `0775`, and the catalogs were made `0644`.
  - Effect: Every file in the data directory or a backup is `0600`, and
    every directory `0700`.
- **A commit SQLite may have kept no longer loses its object file.**
  - Was: Storing, replacing or a new tenant removed the new file on a commit
    error, so a kept commit left a row without its object.
  - Effect: The file stays when SQLite cannot say whether a failed COMMIT
    was kept, and start-up recovery removes it only if no row names it.

## [0.2.0] - 2026-10-27

### Added

- **The `not-a-vcon` problem (422).**
  - Effect: It answers a refusal from the vcon library that this server
    does not name with a problem of its own.
- **`vcon-server backup <dir>` copies the data directory while the server
  runs.**
  - Effect: Each catalog is copied with SQLite's online backup API, then its
    object files; the copy opens under the same master key.
  - Limit: A purge or tenant removal during the copy makes it try again, at
    most 5 times; the admin log records each run as `backup`.
  - Docs: [Back up and
    restore](https://vcon-server.com/docs/backup-and-restore/)
- **The `vcon` and `vcon-server` crates are published on crates.io with each
  release.**
  - Effect: After the GitHub release, each crate whose version crates.io
    does not have yet is published.
  - Limit: 0.1.0 was published by hand.

### Changed

- **vcon 0.2.0 moves the reading API into `vcon::read`.**
  - Effect: `inspect`, `Inspection`, `Limits`, `Rejection`,
    `MAX_SIGNED_LAYERS` and `MAX_JSON_DEPTH` are there; `Form` and
    `parse_uuid` stay at the crate root.
  - Do: Make limits with `Limits::new`: `Inspection`, `Limits` and
    `Rejection` are `#[non_exhaustive]`.
  - Limit: `CRATE_NAME` is gone.

## [0.1.0] - 2026-10-26

### Added

- **The first release: the `vcon` library and the `vcon-server` binary.**
  - Effect: One Cargo workspace holds both, with `vcon-server --version`, a
    README and this changelog.
  - Limit: The license is MIT OR Apache-2.0; `LICENSES/` holds both texts
    by SPDX identifier (`Apache-2.0.txt`, `MIT.txt`).
- **`vcon::inspect` reads a received vCon without changing it.**
  - Effect: It reports its form (unsigned, signed or encrypted), whether it
    is gzipped, and its uuid, or why it cannot be stored.
- **`GET /vcons` lists a tenant's live vCons, newest first, a page at a
  time.**
  - Effect: The order is by when each was first stored, with a cursor, and
    `since` and `until` bound the list.
  - Limit: A tenant catalog made by an earlier build gains the first-store
    time when it is next opened.
- **`GET /audit` and `GET /admin/audit` read the audit logs a page at a
  time.**
  - Effect: A tenant key with the new `audit` scope reads its tenant's log,
    an operator key the admin log, as the lines `audit export` writes.
  - Limit: Each read is recorded as `audit_read`.
- **`GET /key` answers the calling key's own id, tenant and scopes.**
  - Effect: Each call is recorded as `key_info`.
- **An OpenAPI 3.1 description of the HTTP API.**
  - Effect: It is served at `GET /openapi.json` to any valid key and
    published at <https://vcon-server.com/openapi.json>.
- **The console, a web page at `/console/`, signs in with an API key.**
  - Effect: The key is held in page memory only, and the page is served with
    a strict content-security policy and no framing.
  - Do: List, read as text (gzip inflated), store from a file and delete
    vCons; with a legal key, see versions, hold, release and restore.
  - Limit: Each view shows only to a key whose scopes allow it, and each
    action leaves the same audit record as the API.
- **The console shows the audit logs, and tenants and keys to an operator
  key.**
  - Effect: The audit log view shows the tenant log or the admin log, a page
    at a time.
- **Both crates are ready for crates.io.**
  - Effect: Each has its crates.io README, page metadata and the two license
    texts, and packages only its sources.
- **Client examples in `clients/` for curl, Python, TypeScript, Go and
  Rust.**
  - Effect: Each stores, lists, reads and deletes a vCon.

### Changed

- **A build of the repository fetches a fork of rusqlite from github.com.**
  - Effect: The tests use `github.com/NormB/rusqlite` as a dev-dependency,
    since rusqlite 0.41.0 refuses SQLCipher's pragma names; the server builds
    with crates.io rusqlite.
  - Do: Use `cargo vendor` to copy it for offline builds.

### Security

- **A signed vCon's nested layers and deep JSON are read within limits.**
  - Was: Each layer took a stack frame, so a crafted body under a high
    `limits.inflated`, or in a debug build, could overflow the stack.
  - Effect: At most 64 layers and 64 levels of arrays and objects are read;
    a deeper body is refused with `too-deep` (422).
  - Limit: `vcon::MAX_SIGNED_LAYERS`, `vcon::MAX_JSON_DEPTH` and
    `vcon::Rejection::TooDeep` name the limits.

[Unreleased]: https://github.com/NormB/vcon-server/compare/v0.4.7...HEAD
[0.4.7]: https://github.com/NormB/vcon-server/releases/tag/v0.4.7
[0.4.6]: https://github.com/NormB/vcon-server/releases/tag/v0.4.6
[0.4.5]: https://github.com/NormB/vcon-server/releases/tag/v0.4.5
[0.4.4]: https://github.com/NormB/vcon-server/releases/tag/v0.4.4
[0.4.3]: https://github.com/NormB/vcon-server/releases/tag/v0.4.3
[0.4.2]: https://github.com/NormB/vcon-server/releases/tag/v0.4.2
[0.4.1]: https://github.com/NormB/vcon-server/releases/tag/v0.4.1
[0.4.0]: https://github.com/NormB/vcon-server/releases/tag/v0.4.0
[0.3.0]: https://github.com/NormB/vcon-server/releases/tag/v0.3.0
[0.2.0]: https://github.com/NormB/vcon-server/releases/tag/v0.2.0
[0.1.0]: https://github.com/NormB/vcon-server/releases/tag/v0.1.0
