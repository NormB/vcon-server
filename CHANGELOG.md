# Changelog

All notable changes to this project are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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

Nothing in this release breaks a 0.4.1 deployment, and an upgrade from
0.4.1 is the binary alone: see the Upgrade page's "From 0.4.1 to 0.4.2".

### Added

- The console opens a vCon masked, in the vCons and Legal views: a summary
  with its form, counts and each party as the last four digits of a number
  or the domain of an address, and the whole text only after a reveal,
  which reads the vCon again so the audit log holds a record of it. A
  reveal shows one part (parties, subject, dialog, analysis or links) or
  the whole text, cut at 2,097,152 characters. A link in a vCon is shown
  as text and never loaded, and **Download** saves the vCon as
  `<uuid>.json`. Every
  list has a loading and an empty state, a request that gets no answer is
  reported, a refusal says what to do next, releasing a hold asks first,
  and times are shown in UTC with the offset stated. The page has a skip
  link, a labeled navigation, a focus ring and scrolling table regions,
  and its browser tests run axe-core (WCAG 2.2 AA) on every view. The
  console's files are served from one list that a test holds to the
  directory and to the published package.
- `vcon-server tui`: a terminal UI that lists the vCons of one key's tenant
  and shows each one's uuid, version and times. It is a client of a running
  server's HTTP API with a `read` key from a client file, so the server's
  scopes and audit log apply; it reads only, and never a vCon's content. It
  is in the release binaries, `cargo install` and a build from source: the
  new cargo feature `tui` is on by default. A build made with
  `--no-default-features` has the command and says that it has no terminal
  UI.
- `vcon-server migrate`, for the PostgreSQL catalog preview: it makes the
  catalog's schema as `vcon_owner`, one transaction per schema version,
  alone (it takes the advisory locks every server and command shares). A
  connection refuses a schema of any other version than the binary's, and a
  server refuses a role that owns the catalog or can act as its owner.
  Every table is under `FORCE ROW LEVEL SECURITY` with a policy that raises
  when no tenant is set, so a query that forgets the tenant is refused. The
  column codec the catalog will use is in `envelope::column`: AES-256-GCM
  under a per-row key derived from the catalog key, with associated data
  binding the tenant, table, column and row, and HMAC-SHA256 under an index
  key for the values SQL must find.

### Changed

- The console no longer shows a vCon's whole text when it is opened. It
  shows the masked summary, and each reveal is one more read of the vCon in
  the audit log. The HTTP API is as it was: `GET /vcons/{uuid}` and the
  legal reads answer as in 0.4.1. The console's page now loads a fourth
  file, `/console/format.js`; a proxy in front of the server that
  passes the console's paths by name must pass that one too.
- The `vcon-server` crate's default features now include `tui`, so a build
  with default features compiles ratatui 0.30.2 and, through it, crossterm
  0.29.0. A dependency on the crate with `default-features = false` is built
  as before, without them. The library's public items are all still there:
  `tui`, `commands::migrate`, the column codec's names in `envelope`, and
  `console::FILES`, `console::File` and `console::path_of` are new.
- The catalogs in `data_dir` keep their schema version, so a server on the
  default catalogs has nothing to migrate, and 0.4.1 opens a data directory
  0.4.2 has served. `vcon-server migrate` is for the PostgreSQL catalog
  preview only and refuses a configuration whose `catalog.backend` is
  `"local"`. A build with the `postgres` feature and
  `catalog.backend = "postgres"` now needs `vcon-server migrate` run first:
  a connection refuses a database without the schema, where 0.4.1 connected
  and stopped. Neither release serves from PostgreSQL.

### Fixed

- `vcon-server tui`: in a terminal narrower than its first line, the
  server's address is no longer the first part shortened. The connection is
  always whole; the address loses `https://`, then the tenant's id is
  shortened, then the tenant is left out, and only then is the address
  shortened.
- `vcon-server tui` without a terminal exits with code 1 and
  `a terminal is required`, with which of standard input and standard
  output is not one, in place of
  `the terminal: No such device or address (os error 6)`. It checks before
  it reads the key file.
- `vcon-server tui` says what is wrong with a server certificate it
  refuses, and what to do, in place of the TLS library's text: a
  certificate marked as a certificate authority (`CA:TRUE`), one no
  trusted certificate signed, one for another name, one that has expired
  and one not valid yet. Each is still refused before any request is sent.
- The documentation builds with rustdoc's warnings denied: two links from
  public documentation in `mcp/fence.rs` to a private item are gone, and
  the commit hook and CI now build it that way.
- The scenario runner that runs the pages' commands in CI gives a
  container with apt a list of package mirrors that answer, in place of
  one host; an outage of `archive.ubuntu.com` on port 80 had failed a
  required check.

## [0.4.1] - 2026-11-09

### Security

- rmcp goes from 3.8.0 to 3.9.0 (RUSTSEC-2026-0462, GHSA-c9f9-cp8h-jcj5).
  Before 3.9.0, rmcp's Streamable HTTP client read a JSON answer, and an
  error answer, whole and with no size limit. `vcon-server mcp`, the stdio
  bridge, uses that client, so a malicious or broken server at its `url`
  could answer the handshake with a body that never ends, and the bridge
  would buffer it until its host ran out of memory. 3.9.0 stops at 16 MiB
  for a JSON answer and 64 KiB for an error answer, and the bridge then
  stops with an error. The crate now requires rmcp 3.9.0 or later. 3.9.0
  was published on 2026-11-05, less than the 14 days CONTRIBUTING.md asks
  of a new version; it is taken now because it is a security update, which
  Dependabot also takes with no cooldown. A test serves the bridge an
  endless JSON answer and checks that it gives up near the limit.
### Changed

- **Breaking: every secret file is held to one rule.** `master_key`,
  `tls.key`, `storage.s3.secret_access_key_file`,
  `export.s3.secret_access_key_file`, `catalog.postgres.password_file` and
  `catalog.postgres.client_key_file`, the file `rekey master --new-master`
  names, and the MCP stdio configuration's `key_file` must each be a regular
  file, owned by the user the process runs as or by root, with no bit of
  `0027` in its mode: no access for other users and no write for the group.
  `0600`, `0400` and `0640` pass; `0644`, `0604` and `0660` are refused.
  Until now only the two `catalog.postgres` files were checked, and a group
  could write them; a master key, TLS key or S3 secret that every user on the
  host could read was used without a word. `serve` and each command stop with
  an error that names the key, the file, its mode and the `chmod` or `chown`
  that fixes it. A FIFO is refused without being opened, so it cannot block
  the start. The check is one function, `crate::secret_file`, used by every
  reader. **Upgrade note:** before installing, run
  `stat -c '%a %U %n'` on each secret file the configuration names, and
  `chmod 600` (or `chown` to the server's account) any that would be
  refused; the Upgrade page's "From 0.4.0 to 0.4.1: the secret-file rule" has the
  steps. For the `vcon-server` library, `KeyError` gains `Refused(String)`,
  and `TlsError` gains `KeyFile(String)`; neither enum is
  `#[non_exhaustive]`, so a `match` on either needs the new arm. A directory
  given as the master key is now `KeyError::Refused`, not `KeyError::Read`.

### Added

- A preview of the PostgreSQL catalog, behind the `postgres` cargo feature
  (not on by default; it turns on `s3`). The `[catalog]` table names where
  the catalogs are kept: `backend = "local"`, the default, or `"postgres"`
  with a `[catalog.postgres]` table, which needs `storage.objects = "s3"`.
  Every key is checked at start, each refusal naming its key: hosts (DNS
  names in ASCII or IP addresses, no Unix socket), the port, database and
  role names of at most 63 bytes with no NUL, a password file or a client
  certificate, the pool size and the timeouts. With it, `serve` and every
  command read the password file and the client key once, refusing a file
  other users may read, connect over TLS with the certificate chain and the
  host name verified by rustls, log in only by SCRAM-SHA-256 with channel
  binding or a client certificate (a server that asks for `md5`, a plain
  password or `trust` is refused before the password is sent), refuse a
  standby, a read-only server, a superuser and a role with `BYPASSRLS`, and
  then stop with exit code 1, since the catalogs are not kept in PostgreSQL
  yet. The connections come from a pool in the crate. New dependencies:
  tokio-postgres 0.7.18 and tokio-postgres-rustls 0.14.0. The how-to "Keep
  the catalogs in PostgreSQL" covers the database's side and each refusal.
- `scripts/pg-test-server.sh` starts the PostgreSQL servers the catalog's
  tests run against, in containers with TLS (a primary, a hot standby and a
  server with ssl off), and removes them. CI's "catalog backends
  (PostgreSQL 16)" and "(PostgreSQL 18)" jobs run those tests, and `serve`
  with a PostgreSQL catalog beside RustFS.
- `scripts/check_action_pins.py`, run by CI's scan job and the commit hook,
  refuses a `uses:` in `.github/workflows/` or `.github/actions/` that is not
  a local path in the repository or `owner/repo@<40-hex commit> # v<version>`:
  a tag, a branch, a short commit id, a `docker://` image or a commit without
  its version comment. The Development page says how to add an action.
- `.gitignore` ignores `*.local.md` (private spec and plan files) in any
  directory, and `scripts/check-ignored-exports.sh`, run by the commit hook,
  refuses a tracked or staged one, including one added with `--force`.

### Fixed

- CI no longer fails when github.com answers a release download with HTTP
  504 for a few minutes. On 2026-11-08, and on 2026-11-09 from about 05:15Z,
  it did so for most of an hour, and main's CI on 2cb225a went red at every
  job that fetched cargo-nextest, DuckDB, MinIO, cargo-fuzz, cosign or a
  vcon-server release, while the code was green: each fetch was one `curl`
  with about 25 seconds of retries. Every release download in the workflows
  now goes through `scripts/fetch-pinned.sh`, through the new
  `.github/actions/pinned-download` (cached by name, version and sha256,
  saved only on main) or `.github/actions/nextest`. The script checks the
  sha256 of every download and every restored file, retries an outage with
  a doubling delay for up to five minutes, and exits 1 for an outage, 3 for
  bytes that are not the pin and 4 for a permanent refusal.
  `scripts/check_workflow_downloads.py` refuses a step or script that
  fetches a release asset another way, an `actions/cache` save that is
  not main-only, and any cache in a workflow run by `workflow_run`. The how-to scenarios get each release asset a page
  downloads through a mirror in `scripts/docs_test.py` that fetches it the
  same way, against `scripts/docs-test-pins.txt`; their images are pinned
  by digest and pulled with retries, and apt in their containers retries
  each download. The Development page says how to bump a pin.
- The how-to "Keep vCons in an S3 bucket" waited for RustFS or MinIO to
  give any answer, then expected `403`; a service still starting answers
  `503`, which ended the wait and failed the next step (CI's s3 scenario,
  2026-11-09). Each wait now ends on the `403`.

## [0.4.0] - 2026-11-06

### Added

- `vcon-server storage move-objects` moves every tenant's sealed objects from
  the data directory to the bucket `storage.objects = "s3"` names, while no
  server runs: each object is written, read back and compared byte for byte
  before its file is removed, so a run that stops loses nothing and the
  next one moves the rest. It prints, per tenant, how many moved and how
  many versions `check` then finds without an object. A server that kept
  its objects locally moves to Parquet storage with it, then with
  `[storage.parquet]`.

- Parquet storage: with `[storage.parquet]` beside `storage.objects = "s3"`,
  the server packs each tenant's objects in the bucket into Parquet files,
  one store date at a time, at
  `<prefix>tenants/<tenant id>/parquet/date=<yyyy-mm-dd>/part-<file id>.parquet`,
  in a pack run at start and every `pack_interval` seconds (300 unless set);
  a sealed object over `pack_limit` (16 MiB unless set) stays an object. A
  read fetches a packed version's row with one ranged read. A purge lists
  the version's file in the purge's own transaction, rewrites the file
  without the row, deletes the old file and checks with HeadObject that it
  is gone, before `purge_due` returns; start-up recovery and the next sweep
  finish one cut off. A bucket with versioning on or suspended, whatever
  `allow_versioning` says, or with Object Lock on, is refused for Parquet
  storage at start. Start-up recovery removes a pack file no catalog row
  names, and the object of a packed version. A backup writes each packed
  version as an object file, so it restores as before. Removing a tenant
  removes its pack files. Each change of which files hold which rows, and
  each pack run that finds the live versions changed, writes the next
  numbered manifest (`<prefix>tenants/<tenant id>/manifest/<20 digits>.json`,
  `If-None-Match: *`) naming the files and the live versions; start-up
  recovery writes one left unwritten. Each pack run merges a date's small
  files into one; a file merged away and an older manifest stay for
  `reader_grace` seconds (900 unless set), and a purge deletes them at once.
  The files hold a version's id, store time and sealed bytes only, whatever
  the tenant's metadata level; queryable columns come from `export
  parquet`. The how-to [keep vCons as Parquet files in a
  bucket](https://vcon-server.com/docs/parquet-storage/) turns it on over an
  existing bucket, and CI runs the store's conformance suite on it against
  RustFS and MinIO.

- vcon-server can keep its sealed vCon objects in a bucket of an
  S3-compatible service (RustFS, MinIO, AWS S3): `storage.objects = "s3"`
  and the `[storage.s3]` table, with the endpoint, region, bucket, prefix,
  path style (RustFS, MinIO) or virtual-hosted style (AWS S3), an access
  key id with its secret in a file or the AWS default credential chain,
  `ca_file` for a service whose certificate a private certificate authority
  signed, and a request timeout. Each object holds the same sealed bytes as
  the local file, at `<prefix>tenants/<tenant id>/objects/<version id>`;
  the catalogs, `server.sqlite`, the locks and the master key stay in
  `data_dir`. A purge removes the object from the bucket. A bucket with
  versioning on is refused unless `allow_versioning` is set. The first data
  directory to start on an empty bucket and prefix claims it
  (`storage.claim`, `<prefix>vcon-server.claim`), and any other is refused
  there, since start-up recovery removes every object no catalog of its own
  names. A start that cannot use the bucket names the cause: the service's
  error code, a refused connection or an untrusted certificate. Without a
  `[storage]` table nothing changes. Built with the `s3` cargo feature, on
  by default. The how-to [keep vCons in an S3
  bucket](https://vcon-server.com/docs/s3-storage/) sets up RustFS step by
  step, and CI runs the store's conformance suite against RustFS.
- CI runs the store's conformance suite, the S3 backend's tests and `serve`
  against MinIO RELEASE.2025-09-07T16-13-09Z as well (job "storage backends
  (MinIO)"), including a check that the service refuses a second write with
  `If-None-Match: *`, and runs the S3 how-to with MinIO as well as RustFS.
- Start-up recovery logs how many objects it removed (`start-up recovery
  removed objects that no catalog row names removed=<n>`), for local files
  and for objects in a bucket. Before, it removed them without a word.
- Documentation: the S3 how-to gives MinIO and AWS S3 their own setup,
  checks and troubleshooting, the services' required support for
  conditional writes, every start-up refusal with the server's message, and
  a clean-up; a test reads its AWS configuration and IAM policies. The
  audit-logs page lists every action; the command line page has `serve`;
  the client examples show gzip, `If-Match`, paging, a legal hold, an audit
  read, an admin call and MCP; the library page shows `vcon::write`. Tests
  hold the HTTP API page to the router and the command line page to the
  binary, both ways.
- `vcon-server export parquet --tenant <id> --to <dir | s3://bucket/prefix>
  [--since <time>] [--until <time>]` writes a copy of a tenant's kept
  versions, current and hidden, as Parquet files under
  `tenant=<id>/date=<yyyy-mm-dd>/`, then a numbered manifest,
  `tenant=<id>/manifest/<seq>.json`, naming them. It runs beside a serving
  server and changes nothing in the store but one `export` record in the
  tenant's audit log. Each row holds the version's sealed object byte for
  byte. Each file is written whole under its own name (a temporary name and
  a rename in a directory; one PutObject with `If-None-Match: *` in a
  bucket), the manifest last, and a run that fails removes its files. A
  bucket target is reached with the new `[export.s3]` table, or with
  `[storage.s3]`'s service and keys. Built with the `parquet` crate's
  column writer, without Arrow and without a compression codec. The how-to
  [export to Parquet](https://vcon-server.com/docs/export-parquet/) reads
  an export with DuckDB, from a directory and from a bucket.
- Each tenant has a metadata level for its Parquet exports: `none` (the
  default: only the version id, the store time and the sealed body),
  `counts` (also the form, gzip, size, the vCon's times, and its party and
  dialog counts) or `full` (also the uuid, the subject and the parties'
  `tel`, `mailto` and `name`). It is set with `vcon-server tenant
  set-metadata` or `PUT /admin/tenants/{id}/metadata`, recorded in both
  logs, and shown by the tenant list (`metadata`). Statistics are written
  only for the time and count columns, and no column has a dictionary page.
- The `vcon` crate reads, in `vcon::read::fields`, the members an export
  copies from an unsigned vCon or the vCon inside a signed one: `created_at`,
  `updated_at`, `subject`, each party's `tel`, `mailto` and `name`, and the
  number of dialogs.

### Changed

- The tests run under cargo-nextest, pinned to 0.9.148 by
  `scripts/nextest.sh`, in the commit hook and in CI: a test still running
  after 60 seconds is reported by name, and after 3 minutes it is ended and
  fails, where one hung test used to hang the hook. The doc tests still run
  under `cargo test --doc`. Test builds on aarch64 Linux link with the Rust
  toolchain's own lld (`scripts/cc-lld.sh`), and fall back to the system
  linker when the toolchain has none; release binaries link as before.

- Every catalog is at schema version 4: `server.sqlite` has each tenant's
  metadata level, and each tenant's catalog has a place for each version
  in a Parquet pack file and the tables of its pack files, all empty until
  a later change packs versions. A server opens an older catalog and brings
  it up to date. Release 0.3.0 does not open a data directory at version 3
  or 4: go back to it only with the copy taken before the upgrade, as
  [upgrade](https://vcon-server.com/docs/upgrading/) describes.
- Once a tenant has a version packed into a Parquet file, `serve` refuses
  to start with a configuration that has no `[storage.parquet]`, naming the
  table: without it the server packed no more, and its start no longer
  refused a bucket with Object Lock or versioning, either of which keeps the
  bytes of a pack file that a purge rewrote. `vcon-server check` gives each
  tenant's `packed` versions and reports packed versions without the table
  as a problem; the other commands run as before.

### Fixed

- A purge sweep whose version had already lost its object file (removed by
  hand, or by a purge cut off after its commit) stopped with an I/O error
  after deleting the version's row, so the rest of the sweep and the WAL
  checkpoint that follows it did not run. A missing object now counts as
  removed.
- A read whose object the storage failed to give answered `500 damaged`,
  and `vcon-server check` listed the version as damaged, though the stored
  data may be sound: an object file the process may not read or that the
  disk fails to read, a key page the system would not lock, and with the
  objects in an S3 bucket a service that cannot be reached, refuses the key
  or does not answer in time. These now answer `500 internal`, and the
  server logs `a stored version's object could not be read` with the
  failure's `kind` (such as `permission denied` or `timed out`), never its
  text, a key or an id. `damaged` is now only an object that is missing
  while its row is kept, or one that does not decrypt and authenticate.
- For a tenant whose objects the storage did not read, `vcon-server check`
  reported `its catalog cannot be opened: Io`, though the catalog had
  opened. It now reports `its objects cannot be read:` with the failure's
  kind and, when the system gives one, its error number, such as
  `permission denied (os error 13)`, as the server's log line gives them,
  never an object's key; any other failure of the examination is
  `it could not be examined:` with its error.
- The vcon-server crate's description and the OpenAPI description spelled
  "authorised", and four site pages and llms.txt spelled "behaviour",
  "licence" and "honour(ed)"; each is now in US spelling, and a test holds
  every `Cargo.toml` description, every site page, llms.txt and the OpenAPI
  description to the README's US spelling. The development page's
  "Licence" heading is now "License".

## [0.3.0] - 2026-11-04

### Added

- The MCP server for AI agents: `POST /mcp` on the TLS listener, and
  `vcon-server mcp --config <client file>`, which speaks MCP on stdin and
  stdout and forwards to a running server with one key. Every request is
  authenticated with the bearer key as every route is, and counted in the
  key's tenant log or the admin log as action `mcp`. 19 tools: `vcon_get`,
  `vcon_list`, `vcon_store`, `vcon_replace` and `vcon_delete`; `legal_get`,
  `legal_versions`, `legal_restore`, `legal_hold` and `legal_release`;
  `audit_read`; and for operator keys `admin_tenant_create`,
  `admin_tenant_list`, `admin_tenant_retention`, `admin_tenant_remove`,
  `admin_key_create`, `admin_key_list`, `admin_key_revoke` and
  `admin_audit_read`. Each tool calls the function its HTTP route calls and
  leaves the same audit records. Every call passes one gate, in order: the
  key's rate (`mcp.calls_per_second`), a free slot (`mcp.concurrent`), a
  known tool, the tool's scope, and arguments that fit the tool. The
  problems `rate-limited`, `at-capacity`, `unknown-tool` and
  `invalid-arguments` name the refusals. A result holds at most
  `mcp.response_bytes` (1 MiB by default), with `next_cursor` to resume, and
  a vCon's text comes back between untrusted-data markers it cannot close.
- `vcon::write` in the vcon library: an object model of the vCon core
  draft (parties, dialogs of each type, attachments, analysis, extension
  members) and its unsigned and gzip forms. The output is compact JSON
  with members in the draft's order, so the same model gives the same
  bytes, and `read::inspect` reads every output back with the same form,
  gzip flag and uuid. It refuses what core-04 does not allow and names the
  JSON path: an index to nothing, a member Table 1 forbids or a missing one
  it requires, content without an https URL and a hash, an inline
  attachment without a mediatype, both `amended` and `redacted`, an
  extension member with a reserved name, and output deeper than
  `read::MAX_JSON_DEPTH`. `Vcon::amend` and `Vcon::redact` copy a vCon
  under a new uuid and link the prior one; a removed entry leaves a
  placeholder so no index moves. `uuid_v8` makes the version 8 uuid the
  draft recommends. The draft's examples A.4, A.8 and A.9 are built with it
  and equal the draft's files. `WriteError` implements `std::error::Error`.
- The signed and encrypted forms in `vcon::write`: `sign` writes a General
  JWS JSON Serialization with one signature per `Signer` and `updated_at`
  set to the signing time; `sign_again` signs a signed vCon again for a
  long-term archive, at most `read::MAX_SIGNED_LAYERS` layers; `encrypt`
  writes a General JWE JSON Serialization of a signed vCon, its content key
  from a `ContentEncrypter` and wrapped by each `KeyEncrypter`. The
  cryptography is the caller's, through those three traits; a payload or
  plaintext can be gzipped first (`Compression`). The input to sign again
  or encrypt is read under the caller's `read::Limits`, and every output
  reads back with `read::inspect`. The draft's recommended algorithms
  follow as an option in a later release.
- Each release also holds `SHA256SUMS.txt.sigstore.json`, a keyless cosign
  signature over `SHA256SUMS.txt`, and `vcon-server-<version>.intoto.jsonl`,
  the build provenance as a file, both made from the release workflow's
  OIDC identity with no stored key and verified before they are released.
  The install page shows how to check the signature with cosign; 0.2.0 and
  earlier have neither file.

### Changed

- The release binaries are built on glibc 2.34 (AlmaLinux 9), and the
  release workflow refuses a binary that needs a newer glibc symbol. They
  run on Debian 12, Ubuntu 22.04 and AlmaLinux 9, where the 0.2.0
  binaries, which needed glibc 2.38, did not start. Each binary is run
  on Debian 12, Ubuntu 22.04 and AlmaLinux 9 before a release, and the
  release build is reproducible: CI builds it twice and compares the bytes.
- A failure to open `server.sqlite` at start says which step failed, on
  which file, and the error underneath: a wrong key, an unfinished
  recovery, or the system's error. It said only `Io` before.

### Fixed

- Every file the server and its commands write into the data directory or a
  backup is made mode `0600` and every directory `0700`, whatever the umask
  of the process. They followed the umask before: `backup` under umask 0002
  wrote its files `0664` and its `tenants` directory `0775`, inside its own
  `0700` directory, and the catalogs were made `0644`.
- Storing or replacing a vCon, or creating a tenant, removed the new object
  file or tenant directory whenever the commit returned an error. When
  SQLite cannot say whether a failed COMMIT was kept, the file now stays,
  and start-up recovery removes it only if no row names it. Before, a commit
  kept after such an error left a row without its object, read back as
  damage, or a tenant without its catalog.

## [0.2.0] - 2026-10-27

### Added

- The `not-a-vcon` problem (422): the answer to a refusal from the vcon
  library that this server does not name with a problem of its own.
- `vcon-server backup <dir>`: a consistent copy of the data directory,
  taken while the server runs. Each catalog is copied with SQLite's online
  backup API, then every object file a copied catalog names; a version
  purged or a tenant removed during the copy makes it try again, at most 5
  times. The copy is a data directory that opens under the same master key,
  and the admin log records each run as `backup`.
- The release workflow publishes `vcon` and `vcon-server` on crates.io by
  trusted publishing: on a `v*` tag, after the GitHub release, each crate
  whose version crates.io does not have yet, with a token the job gets from
  its OIDC identity and that is revoked when the job ends. A run started by
  hand makes the same token exchange and dry-runs both crates, uploading
  nothing. 0.1.0 was published by hand.

### Changed

- vcon 0.2.0: the reading API is in `vcon::read` (`inspect`, `Inspection`,
  `Limits`, `Rejection`, `MAX_SIGNED_LAYERS` and `MAX_JSON_DEPTH`); `Form`
  and `parse_uuid` stay at the crate root, for what writing will share.
  `Inspection`, `Limits` and `Rejection` are `#[non_exhaustive]`, so a later
  release can add a field or a refusal without a breaking change; limits are
  made with `Limits::new`. `CRATE_NAME` is gone.

### Fixed

- The release workflow's crates.io job read one version, the workspace's,
  for both crates, so it would have skipped a vcon version that differs from
  the server's. It looks each crate up at its own version, and a run by hand
  dry-runs both crates in one `cargo publish`, so a new vcon version the
  server needs is verified before it is on crates.io.
- The commit hook's package check could build the packaged server against
  an older copy of `vcon` 0.1.0 left in the same target directory by an
  earlier run, because cargo's own verify step reuses a registry crate it
  has built at the same version. The check now packages with `--no-verify`
  in a directory made for each run, and builds each package from its
  `.crate` with the published crates patched in by path, which cargo
  checks by content. `scripts/test-check-package.sh`, run in CI, checks a
  changed `vcon` 0.1.0 in the same target directory and two trees at once.

## [0.1.0] - 2026-10-26

### Added

- Cargo workspace with the `vcon` library and the `vcon-server` binary.
- `vcon-server --version`.
- `vcon::inspect`: reads a received vCon without changing it and reports
  its form (unsigned, signed or encrypted), whether it is gzipped, and its
  uuid, or why it cannot be stored.
- Licence files (MIT OR Apache-2.0), README and this changelog.
- `LICENSES/`: the two licence texts again, named by SPDX identifier
  (`Apache-2.0.txt`, `MIT.txt`), kept identical to the root files by a test.
- Git hooks: fmt, clippy and tests on the staged tree, and refusal of personal
  data and AI-tool attribution in changes and commit messages.
- Pre-push hook: every outgoing commit must be signed by an allowed signer and
  carry the pusher's configured identity.
- `GET /vcons`: a tenant's live vCons, newest first by when each was first
  stored, a page at a time with a cursor, bounded by `since` and `until`.
  A tenant catalog made by an earlier build gains the first-store time when
  it is next opened.
- `GET /audit` and `GET /admin/audit`: a tenant's audit log, for a tenant key
  with the new `audit` scope, and the admin log, for an operator key, a page
  at a time, as the same lines `audit export` writes. Each read is recorded
  as `audit_read`.
- The console: a web page at `/console/` that signs in with an API key held
  in page memory only, served with a strict content-security policy and no
  framing; browser tests run it against the real server.
- `GET /key`: the calling key's own id, tenant and scopes, recorded as
  `key_info`.
- An OpenAPI 3.1 description of the HTTP API, served at `GET /openapi.json`
  to any valid key and published at <https://vcon-server.com/openapi.json>.
- The console's vCons and legal views: list, read (as text, gzip inflated),
  store from a file and delete, and a vCon's versions for a legal key, each
  shown only to a key whose scopes allow it.
- Both crates are ready for crates.io: each has its crates.io README, its
  page metadata and the two licence texts, and packages only its sources.
  `scripts/check_package.py`, run by the commit hook and CI, packages and
  builds each crate, then refuses any file in a `.crate` other than the
  sources, the files they include, the README and the licences; a missing
  README or licence; a git or path dependency; a relative README link; and
  anything the commit scan refuses.
- Client examples in `clients/` (curl, Python, TypeScript, Go and Rust), each
  storing, listing, reading and deleting a vCon; CI runs every one against a
  live server (`scripts/run-clients.sh`).
- The console's legal actions (hold, release, restore), audit log view
  (tenant log or admin log, a page at a time), and tenants and keys views
  for an operator key, each leaving the same audit record as the API.

### Changed

- The tests use a fork of rusqlite (`github.com/NormB/rusqlite`) as a
  dev-dependency, to check SQLCipher's settings through rusqlite's pragma
  helpers, which rusqlite 0.41.0 refuses for SQLCipher's pragma names. The
  server still builds with crates.io rusqlite, but a build of the repository
  now fetches the fork from github.com; `cargo vendor` copies it for offline
  builds.

### Security

- A signed vCon's nested JWS layers are read in a loop, not by recursion, and
  at most 64 are read; JSON is read at most 64 arrays and objects deep. A body
  past either limit is refused with the `too-deep` problem (422). Before,
  each layer took a stack frame, so a crafted body under a high
  `limits.inflated`, or any such body in a debug build, could overflow the
  stack and abort the server. `vcon::MAX_SIGNED_LAYERS`,
  `vcon::MAX_JSON_DEPTH` and `vcon::Rejection::TooDeep` name the limits.

[Unreleased]: https://github.com/NormB/vcon-server/compare/v0.4.6...HEAD
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
