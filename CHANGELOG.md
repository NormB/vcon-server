# Changelog

All notable changes to this project are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **The PostgreSQL preview's schema is at version 11.** A database that
  0.4.5 or an earlier release migrated is at version 6: run
  `vcon-server migrate` with the new binary before a server starts, as
  [after an upgrade from 0.4.5](https://vcon-server.com/docs/postgres-catalog/#after-an-upgrade-from-0-4-5)
  shows. Until then a server stops with `the database schema is version
  6, older than this binary's 11: run vcon-server migrate`. Version 7
  gives the `version` table the column `seq`, the order versions were
  stored in, and on PostgreSQL 17 and later grants `vcon_app` `MAINTAIN`
  on the tables `version`, `vcon` and `tombstone`, for the `VACUUM` the
  server will run after a purge. Version 8 makes the audit logs
  append-only in the database: triggers on the tables `audit` and
  `admin_audit` refuse every `DELETE` and `TRUNCATE`, and every `UPDATE`
  but a larger `count` of a record the server counts requests in, for
  `vcon_owner` as for `vcon_app`. Version 9 sets the rule for that
  `count`: an `UPDATE` passes when the new count is larger than the old
  one, by any number, and no other column is different, where version 8
  took exactly one more; the server then writes the refused requests of a
  minute to a record that is there with one statement.
  The count only grows: the same number or a smaller one is refused with
  `an audit record is not changed, but for a larger count of a counted
  record`. Version 10 lets a transaction of the registry read every row
  of the table `tenant_lock`, which has one row for each tenant that has
  a catalog, and write none; a tenant's transaction reads and writes its
  own row, as before. Version 11 gives each tenant's audit log a key of
  its own: the table `audit_key` holds it sealed under the tenant's
  catalog key, and each record in `audit` names the key that sealed its
  target in the new column `key_id`. `vcon_app` reads an audit key, makes
  one and seals one again, and no role is granted a `DELETE` there. No
  row is changed by any of these versions: a database of the preview
  holds no audit record, so version 11 converts none. The local catalogs
  stay at schema version 9.
- **The statements that keep a tenant's catalog in PostgreSQL**, behind
  the `postgres` cargo feature and used by no server yet: a start with
  `catalog.backend = "postgres"` still connects, checks and stops. They
  pass the store's conformance suite on PostgreSQL 16 and 18 with the
  objects in a bucket, with and without Parquet storage. The uuid of a
  vCon is kept as an HMAC and a sealed copy; a version's form, gzip flag
  and size, and an audit record's target, are sealed; times, holds,
  counts and random ids are clear.
- **The statements that make, open, list and remove a tenant in
  PostgreSQL**, behind the `postgres` cargo feature and used by no server
  yet. A tenant is made by its row in `tenant_lock` and the id of its
  catalog key, and its objects are in the bucket. A removal deletes the
  bucket's objects and then the tenant's rows, at most 10,000 rows of a
  table in one transaction. A removal keeps the tenant's audit records:
  the rows of `audit` and `audit_key` stay, and the database refuses a
  `DELETE` of a record from every role.
- **The statements that move a tenant's catalog in PostgreSQL to a new
  tenant key**, behind the `postgres` cargo feature and used by no server
  yet. One transaction makes the sealed columns and the lookup values of
  the tenant's vCons, versions and purge records again under the new key
  and seals the key of the tenant's audit log again; it changes all of
  them or none. The audit key does not rotate with the tenant key, and no
  audit record is changed: a record's target is sealed under the audit
  key. Giving a log a new audit key is not written.

### Changed

- **The terminal UI's page states the keys once.** The
  [Keys](https://vcon-server.com/docs/tui/#keys) table is the one list of
  the keys and of the views each works in. The page no longer repeats the
  keys under each of the 25 screens, in a table of last lines or in a copy
  of the help view. A second table under Keys names the screens where
  fewer keys work, and a test holds both tables to the code. A screen's
  section says which screens follow it, and names a key only in a step.
  The command is unchanged.

### Fixed

- **No test and no log line shows the uuid of a vCon.** Code scanning
  refused a pull request for an assertion message that wrote a vCon's
  uuid to the test log (`rust/cleartext-logging`), the second time for
  that cause, and nothing on the developer's side held it. Now
  `scripts/check_style.py` refuses, under `crates/`, an assertion
  message, a panic text, a debug print or a log line that names a value
  whose name ends in `uuid` (`uuid-in-message`), and the commit hook runs
  it. The two messages it found name the vCon by its place in the test.
  The rule reads names: it does not find a uuid under another name, one
  that a call returns, or the values a failed `assert_eq!` prints.
- **The workflows named a floating runner image.** Every job ran on
  `ubuntu-latest`. On 2026-11-13 GitHub began to give that label Ubuntu
  26.04 for some jobs: the website job's Playwright install failed on
  `main`, on a commit whose pull request had passed on Ubuntu 24.04
  minutes before. Every job now names `ubuntu-24.04` (the aarch64 release
  jobs already named `ubuntu-24.04-arm`), and
  `scripts/check_runner_images.py`, in the commit hook and in CI's scan
  job, refuses a label that ends in `-latest` in a workflow or a composite
  action. The release binaries are not changed by this: they are built in
  an AlmaLinux 9 container pinned by digest, on glibc 2.34, as before.
  [Runner images](https://vcon-server.com/docs/development/#runner-images)
  shows how to move to a newer image. No server code changed.
- **The commit hook did not run every check of the tree that CI runs.** No
  hook ran `scripts/test_docs_test.py`, the how-to runner's own tests,
  which read the how-to pages: on 2026-11-13 a page lost a line of a
  step's expected output, the hooks accepted the commit and CI's how-to job
  refused it. Nothing compared the scripts the workflows run with the
  scripts the hooks run. `scripts/check_hook_steps.py` now does, in the
  commit hook and in CI's scan job: a script of `scripts/` that a workflow
  runs is run by a hook, or is listed in the script with the reason it runs
  only in CI. The commit hook now runs the seven it found:
  `scripts/test_docs_test.py`, `scripts/check_test_runner.py`,
  `scripts/ci_success.py check`, `scripts/clippy_features.py check`,
  `scripts/check_dependabot.py`, `scripts/check_test_names.py` and
  `scripts/check_glibc.py --docs .`. The hook runs in a copy of the staged
  files, which is no git repository, so the two checks that read git's
  tracked files take `--walk` there and read every file of the copy.
  [Scripts only CI runs](https://vcon-server.com/docs/development/#scripts-only-ci-runs)
  shows how to add a script step. No server code changed.
- **The commit hook did not lint the feature sets CI lints.** The hook ran
  clippy for the default features only, so a lint in code that only the
  `postgres` feature, the `backend-tests` feature or a build without the
  default features compiles failed first in CI: an unused `mut` did on
  2026-11-12. The feature sets are now the rows of
  `.github/clippy-feature-sets.txt`; `scripts/clippy_features.py` lints
  each with warnings denied, in the commit hook (every row) and in CI's
  backend and PostgreSQL jobs (their groups), and its `check`, in CI's scan
  job, refuses a feature set linted outside the list and a row no CI job
  runs.
  [Feature sets clippy lints](https://vcon-server.com/docs/development/#feature-sets-clippy-lints)
  shows how to add one. No server code changed.
- **In the PostgreSQL preview a statement could choose a row's number.**
  The column `seq` of the tables `version`, `manifest`, `audit` and
  `admin_audit` is numbered by PostgreSQL, and an `INSERT` with
  `OVERRIDING SYSTEM VALUE`, or an `UPDATE` that set it to `DEFAULT`,
  gave a row another number; the newest version of a vCon and the order
  of an audit log are read by it. From schema version 8 a trigger takes
  the number of a new row from the sequence whatever the statement says
  and keeps the number of an updated row. No released server writes
  these tables.
- **A server started the moment `vcon-server migrate` returned could be
  refused by the migration's locks.** The command left its two advisory
  locks to the end of its database session, and PostgreSQL releases
  those only when it notices the closed connection, which can be after
  the command has exited. `migrate` now gives up the role and both locks
  before it returns, also when it fails or was refused one of the two.
  Only the PostgreSQL preview is affected.

- **The PostgreSQL preview's pages said more of row-level security than
  it does.** The how-to and the status page said that a query with no
  tenant set is refused. That holds for a statement the server plans: the
  planner evaluates the policy. A prepared statement whose plan the
  server has cached is not planned again, and the policy is then
  evaluated only for a row the statement examines, so on a table with no
  rows, or with an index condition that finds none, such a statement ends
  with nothing and no error. What holds for every plan: with no tenant
  set no row is returned or changed, and the statement is refused at the
  first row it examines. The
  [how-to](https://vcon-server.com/docs/postgres-catalog/) and the
  [status page](https://vcon-server.com/docs/status/) now say that, and
  tests on PostgreSQL 16 and 18 hold it for every table. The schema is
  unchanged, and no row was readable without a tenant before or after.

## [0.4.5] - 2026-11-12

The upgrade from 0.4.4 is the binary alone. No stored data and no schema
changes: the catalogs stay at schema version 9, and the PostgreSQL
preview's schema at version 6. A 0.4.4 configuration is read as it was.
A downgrade is possible: 0.4.4 opens a data directory that 0.4.5 has
served. Three things look different after the upgrade; each is under
Fixed or Changed below, and the Upgrade page's "From 0.4.4 to 0.4.5" has
the steps.

- **The terminal UI's last line names only the keys that act.** On each
  screen of `vcon-server tui` the last line lists the keys that do
  something in the state shown. 0.4.4 showed the list's keys on the
  connecting screen, an empty tenant and every failure.
- **Two failure screens say what can be done.** For an operator key, and
  for a key without the `read` scope, the screen says `Press q to quit.`
  where 0.4.4 said `Press r to try again, or q to quit.`. While a message
  stands in place of the list, the keys that select and open do nothing.
- **`expires_at` of a new key is a whole second.** A key made by 0.4.5
  has an expiry with no fraction of a second, such as
  `2027-02-10T12:00:00Z`. Keys made before keep the expiry they have, so
  a client must still read an `expires_at` with a fraction.

### Fixed

- **The terminal UI's last line named keys that did nothing.** The
  connecting screen, an empty tenant and every failure showed the list's
  keys (`j down  k up  Enter open  r refresh  n more`). The last line now
  names only the keys that act on the screen: `? help  q quit` while
  connecting or loading, `r refresh  ? help  q quit` for an empty tenant
  or a failure, no `n more` when every vCon is read, and no `j down  k up`
  for a list of one. For a key that may not list vCons (an operator key,
  or a key without the `read` scope) the screen says `Press q to quit.`
  in place of `Press r to try again, or q to quit.`, since `r` cannot
  read another key. While a message stands in place of the list, the keys
  that select and open do nothing. The
  [terminal UI page](https://vcon-server.com/docs/tui/) lists
  the last line of each screen.

### Changed

- **The terminal UI page shows every screen.** The
  [page](https://vcon-server.com/docs/tui/) has a table of the 25 screens
  the command draws and a section for each: the screen as the command
  draws it, what it shows, the keys that do something on it, how to leave
  it and, for a failure, what to do. Nine screens are new on the page:
  connecting, loading, the list of one vCon, the list while the next page
  is read, a key without the `read` scope, a list the server did not
  give, and three of the detail view. The prerequisites name what the
  server must hold: a tenant, a tenant key with the `read` scope and no
  operator configuration.
- **A new key's expiry is a whole second.** `expires_at` of a key made
  from now on has no fraction of a second (`2027-02-10T12:00:00Z`): the
  days are counted from the second the key is made in, so a key lives up
  to one second less than the days asked and never longer. The answer
  that makes a key with an expiry is then 136 bytes whenever the key is
  made; before, its length changed with the clock's sub-second digits.
  Keys made before keep the expiry they have.
- **A GitHub release's text is its changelog section.** The release
  workflow writes the section of the tag's version, then the line that
  says how to verify a download (`scripts/release_notes.py`). Each
  paragraph and list item is written on one line, since GitHub shows each
  line break of a release's text. A tag whose version has no section in
  `CHANGELOG.md` stops the workflow before the builds. Before, the text
  was the verify line alone, and the notes of 0.4.4 were added by hand.
- **No test shows a credential when it fails.** Assertion messages, panic
  texts and debug prints in the tests no longer show a key's token, a
  secret, a cookie or a session id, and `scripts/check_style.py` refuses
  one that names such a value (`credential-in-message`).

## [0.4.4] - 2026-11-12

There is no downgrade from 0.4.4. The first time 0.4.4 opens a data
directory it brings the catalogs from schema version 4 to 9, and release
0.4.3 and earlier refuse a catalog at version 9. Copy the data directory
before the upgrade, with the server stopped: that copy is the only way
back, and everything stored after it is lost on the way back. A 0.4.3
configuration is read as it was. Each of the following is under Security,
Added or Changed below, and the Upgrade page's "From 0.4.3 to 0.4.4" has
the steps.

- **New keys expire.** A key made by 0.4.4 expires 90 days after it is
  made, unless its request names 1 to 365 days or asks for no expiry. An
  operator can refuse new keys with no expiry by setting `keys.no_expiry`
  to `false`. Keys made before the upgrade keep working with no expiry.
- **A request that carries both a key and the session cookie is
  refused.** A request with an `Authorization` header and a cookie named
  `__Host-vcs_session` is answered 400 `key-and-session`.
- **Sessions and users have no sign-in method.** 0.4.4 keeps users,
  memberships and sessions and serves the two session routes, and nothing
  in it starts a session: nobody can sign in as a user. The status page
  says so. API keys work as before.
- **Readers of the audit log.** Thirteen action names are new:
  `user_add`, `user_list`, `user_deactivate`, `membership_grant`,
  `membership_revoke`, `role_cap_set`, `role_cap_read`, `settings_read`,
  `settings_set`, `membership_list`, `session_start`, `session_read` and
  `session_end`. The `actor` of a record may be `user:` and a user id,
  and an `auth_failed` record may have the status 400 or 403.
- **The PostgreSQL catalog preview.** Run `vcon-server migrate` after the
  new binary is installed and before a server starts: the database's
  schema goes from version 1 to 6.
- **Code that uses the `vcon-server` library.** `config::Config` has two
  new members, `public_origin` and `keys_no_expiry`, `config::Operator`
  one, `public_origin`, and `api::App` one, `origin`, so a struct literal
  of any of them must set them. `audit::Action` has thirteen new
  variants, one for each action above, and `audit::Actor` one, `User`,
  which a `match` that lists every variant must take.
  `catalog::SCHEMA_VERSION` is 9.

### Security

- rusqlite goes from 0.41.1 to 0.42.0, which moves the bundled SQLCipher
  from 4.19.0 to 4.20.0 and the SQLite inside it from 3.53.4 to 3.54.1
  (CVE-2026-119375, RUSTSEC-2026-0459). SQLite before 3.54.0 can return
  uninitialized page-cache memory when a process opens a database file
  beside a write-ahead-log file (`-wal`) whose page size differs from the
  database's. The vector is local: someone who can write a crafted pair of
  files into the data directory, where the server opens `server.sqlite`
  and each tenant's catalog, could make queries return stale bytes of the
  server's memory. Releases 0.4.3 and earlier bundle an affected SQLite.
  The file format does not change: the new build opens the encrypted
  catalogs that 0.4.3 wrote, and a test now fails if the linked SQLite is
  older than 3.54.0. The crate requires rusqlite 0.42.0 or later. 0.42.0
  was published on 2026-11-11, less than the 14 days CONTRIBUTING.md asks
  of a new version; a published advisory against a shipped dependency
  comes before that wait, as it does for Dependabot's security updates.

### Added

- Users and memberships, the first part of sign-in. `server.sqlite` keeps
  users (a label, and for a user of an identity provider its issuer and
  subject) and memberships (a user's roles in one tenant, or the operator
  role). Five commands on the host make and change them, beside a running
  server: `user add`, `user list`, `user deactivate`, `membership grant`
  and `membership revoke`. The tenant roles are `reader`, `producer`,
  `records`, `legal`, `auditor` and `tenant-admin`. Each command is
  recorded in the admin audit log as `user_add`, `user_list`,
  `user_deactivate`, `membership_grant` or `membership_revoke`, and a
  change to a tenant membership is copied to that tenant's log; a record
  names ids and role names, never a label, an issuer or a subject. There
  is no route for either, and no request reads them yet: nobody can sign
  in as a user until a later release adds a sign-in method. See the how-to
  "Add users and memberships".
- Roles as sets of scopes, and a role cap for each tenant. Each role gives
  one scope: `reader` gives `read`, `producer` `write`, `records` `delete`,
  `legal` `legal`, `auditor` `audit`, `tenant-admin` the new scope
  `tenant_admin`, and `operator` `admin`; a membership's scopes are those
  of its roles. `tenant set-role-cap` sets the roles a membership in a
  tenant may be given, and `tenant role-cap` prints them; both run on the
  host, beside a running server. A grant of a role outside the cap is
  refused, and a role granted before the cap was set stops counting until
  the cap allows it again. The two commands are recorded as `role_cap_set`
  (in the admin audit log and the tenant's) and `role_cap_read` (in the
  admin audit log). See the how-to "Roles, scopes and a tenant's role cap".
- Routes for a tenant's administrator. A key of a tenant that holds the scope
  `tenant_admin` reads and changes the tenant's settings (`GET` and
  `PUT /tenant/settings`), makes, lists and revokes the tenant's keys
  (`POST` and `GET /tenant/keys`, `DELETE /tenant/keys/{id}`) and lists the
  tenant's memberships (`GET /tenant/memberships`). Each route works in the
  tenant of the key that asks; an operator key is refused. The settings are
  a fixed list: `masking.<role>` for each of the six tenant roles (`true`
  or `false`, `true` while unset), `session.idle_seconds` (60 to 900) and
  `session.absolute_seconds` (300 to 28800), with the idle limit never above
  the absolute one; a change outside the list is refused with the new
  problem `bad-setting`. Sessions read the two limits; nothing reads the
  masking settings yet: the console will, in a later release. A key made at
  `POST /tenant/keys` holds scopes within the tenant's role cap and never
  `tenant_admin` or `admin`. Each call is recorded in the admin audit log
  and the tenant's, as `settings_read`, `settings_set`, `key_create`,
  `key_list`, `key_revoke` or `membership_list`; a `settings_set` record
  names the settings changed and never a value. The console's key form
  offers `tenant_admin`. See the how-to "Administer a tenant with a key".
- Sessions and their CSRF protection, the fourth part of sign-in.
  `server.sqlite` keeps a session for a user's membership: the SHA-256 of
  a random 256-bit id, the user, the tenant (none for an operator), the
  scopes the membership gave when the session started, its times and the
  SHA-256 of its CSRF token. The id travels in the cookie
  `__Host-vcs_session` (`Secure`, `HttpOnly`, `SameSite=Strict`, `Path=/`).
  A session ends when it has not been seen for its idle limit, at its
  absolute limit, when its user signs out, when a role of its user is
  granted or taken or the user is deactivated, when its tenant's role cap
  changes, and with its tenant. The limits are the tenant's settings
  `session.idle_seconds` and `session.absolute_seconds` as they are when
  the session starts (15 minutes and 8 hours unless set, and for an
  operator). Two routes take the cookie and no key: `GET /session` answers
  the session and its CSRF token to a request with
  `Sec-Fetch-Site: same-origin`, and `POST /session/logout` ends it for a
  request with the listener's `Origin` and the token in `X-CSRF-Token`.
  The origin is the new configuration key `public_origin`, and
  `operator.public_origin` for the operator listener; a listener without
  it serves no session, and the two must name different hosts. New problem
  types: `no-session` (401), `bad-origin` and `bad-csrf` (403), and
  `key-and-session` (400). New audit actions: `session_start`,
  `session_end` and the counted `session_read`, with the actor
  `user:<id>`; a refused request by session is counted as `auth_failed`.
  No sign-in method exists yet, so nothing starts a session outside the
  tests: passkeys and single sign-on will, in a later release. The routes
  that take a key take no session yet. See the how-to "Set a listener's
  public origin for sessions".
- Key expiry and last-used time. A key now has an expiry, `expires_at`:
  90 days after it is made by default, 1 to 365 days with
  `expires_in_days` (`key create --expires-days`), or none with
  `"no_expiry": true` (`key create --no-expiry`). The new configuration
  key `keys.no_expiry` (default `true`) lets an operator refuse new keys
  with no expiry. The answer that makes a key carries `expires_at`. A
  request by an expired key is refused with the new problem `key-expired`
  (401) and counted as `auth_failed`; an expiry a key cannot have is the
  new problem `bad-expiry` (422). Every list of keys (`key list`, `GET
  /admin/tenants/{id}/keys`, `GET /tenant/keys`, `admin_key_list`) shows
  `expires_at` and `last_used_at`. The last-used time is the start of the
  minute of an accepted request, and the server writes it at most once a
  minute for each key; a failure to write it refuses no request. The
  console's key form takes the days, and its key table shows both times.
  See the how-to "Administer a tenant with a key", under "Set a key's
  expiry and see when it was last used".

### Changed

- **A request with both a key and the session cookie is refused.** Every
  route answers 400 `key-and-session` to a request that carries an
  `Authorization` header and a cookie named `__Host-vcs_session`, before
  either is read. No release set that cookie, so a client is affected only
  if it sends a cookie of that name itself.
- **Breaking for a reader of the audit log:** the `actor` of a record may
  be `user:` and a user id, and the actions `session_start`,
  `session_read` and `session_end` are new. `auth_failed` records may have
  the status 400 or 403.
- **No downgrade: the catalogs move to schema version 9.** The first time
  this server or one of its commands opens a data directory, it brings
  `server.sqlite` from schema version 4 to 9, and each tenant's catalog at
  that tenant's first use, each in one transaction. Version 5 adds the
  tables `user_account` and `membership` to `server.sqlite`, and lets every
  audit log hold a record with no status: the number 0 that a version-4
  log kept for a `listener_full` record becomes no status, under the same
  sequence numbers, and readers see `null` as before. Version 6 adds the
  column `role_cap` to the `tenant` table of `server.sqlite`, empty for
  every tenant. Version 7 adds the table `tenant_setting` to
  `server.sqlite`, empty. Version 8 adds the table `session`, empty.
  Version 9 adds the columns `expires_at` and `last_used_at` to the
  `api_key` table, empty for every key there is.
  Release 0.4.3
  and earlier refuse a catalog at version 9 (`schema version 9 is not one
  this build knows`), so the way back is the copy of the data directory
  taken before the upgrade. With the PostgreSQL catalog preview, run
  `vcon-server migrate` to bring the database from schema version 1 to 6
  before a server starts. See "Schema version 9" on the upgrade page.
- **A new key expires 90 days after it is made, unless its request says
  otherwise.** This holds for `key create`, `POST
  /admin/tenants/{id}/keys`, `POST /tenant/keys` and the MCP tool
  `admin_key_create`. A script that makes a key and never replaces it
  stops working 90 days later with 401 `key-expired`: replace its key on a
  schedule, or make it with `--no-expiry` (`"no_expiry": true`). Keys made
  before the upgrade are not changed: they have no expiry and keep
  working. See "Keys made before the upgrade have no expiry" on the
  upgrade page.
- **Breaking for a reader of the audit log that knows a fixed list of
  actions:** add `user_add`, `user_list`, `user_deactivate`,
  `membership_grant`, `membership_revoke`, `role_cap_set`,
  `role_cap_read`, `settings_read`, `settings_set` and `membership_list`.
- **`user list` prints each membership's `scopes`** beside its `roles`:
  what the membership lets its user do, under the tenant's role cap.
- **A tenant key may hold the scope `tenant_admin`.** `key create --scope`
  and `POST /admin/tenants/{id}/keys` take the name with the five there
  were. Such a key reaches the routes under `/tenant`.
- The terminal UI's page is rewritten. It opens with what the command shows
  and one screen, says when to use the console or the API instead, lists
  what to have ready in a table, and has a quick start for the server's own
  machine and a separate procedure for a terminal on another machine. Each
  task has a section and a screen: reading the screen, finding a vCon,
  the next page, reading the list again, help, leaving, and narrow and wide
  terminals. The reference has the client file, the keys by view, the
  environment, and the sizes and limits as tables. The messages are grouped
  by where they appear, each with its cause and what to do. The page now
  shows sixteen screens, all drawn by the UI's own code, and tests hold its
  keys, client file keys, limits and every message it quotes to the code.
  The page said a refused list shows `the server answered 503
  (unavailable)`; the UI shows the problem's title:
  `the server answered 503 (The service cannot answer now)`.
- The how-to scenarios open the terminal UI's detail and help views on a
  live server, and start it with a revoked key: a `terminal` step of
  `scripts/docs_test.py` may name the keys to type (`keys Enter q`).
- `vcon-server tui`: when a request fails, the screen adds
  `A change to the client file or the key file takes effect at the next
  start.` under `Press r to try again, or q to quit.` The command reads
  both files once, at start, so `r` never reads a corrected file.

### Fixed

- `vcon-server tui`: in a terminal narrower than its last line (59 columns
  in the list, 49 in the detail view), the key hints were cut at the edge,
  and `? help` and `q quit` were not on screen. The last line now always
  ends with those two and leaves out, whole, the hints that do not fit.
- `vcon-server tui`: the line above the key hints was one line and was cut
  at the terminal's edge. A failed next page showed `The next page was not
  read: the server did not answer: error sending request for` and no cause.
  The line now wraps, up to half of the screen.
- `vcon-server tui`: a key revoked while the command ran left the first
  line saying `connected`, with the tenant, above `the server refused the
  key (401)`. The first line now says `not connected`, as at start, and
  `r` asks for the key again.

## [0.4.3] - 2026-11-11

An upgrade from 0.4.2 is the binary alone: the catalogs keep their schema
version, a 0.4.2 configuration is read as it was, and 0.4.2 opens a data
directory that 0.4.3 has served. Five changes can break something around a
0.4.2 deployment, and one breaks code that uses the library; each is under
Added, Changed or Fixed below, and the Upgrade page's "From 0.4.2 to
0.4.3" has the steps.

- **Readers of the audit log.** Seventeen action names are new: the
  fourteen reveal actions (`reveal_parties`, `reveal_subject`,
  `reveal_dialog`, `reveal_analysis`, `reveal_links`, `reveal_all`,
  `reveal_audio`, and `legal_reveal_` with the same seven endings),
  `wrong_listener`, `listener_full` and `csp_report`. A `listener_full`
  record has `null` as its `status`.
- **Reverse proxies and firewalls that pass the console's paths by name.**
  Pass `/console/player.html`, `/console/player.js` and
  `POST /console/csp-report` as well as `/console/format.js`, and pass
  each answer's `Content-Security-Policy` header unchanged.
- **A check that holds the console's content-security policy byte for
  byte.** Both policies end with a reporting directive,
  `report-uri /console/csp-report`.
- **Clients and proxies that hold many connections, or hold them idle.** A
  listener holds at most `limits.connections` connections (512), and a
  connection is closed after `limits.timeouts.idle` seconds without a
  request (120), after a TLS handshake longer than
  `limits.timeouts.handshake` (10), and after a request head slower than
  `limits.timeouts.first_head` or `limits.timeouts.head` (30 each). 0.4.2
  set none of these limits and refuses a configuration that has one of the
  five keys.
- **Operator keys, only with the new `[operator]` table.** They then work
  at `operator.listen` alone. Without the table nothing changes.
- **Code that uses the `vcon-server` library.** `config::Config` has three
  new members, `operator`, `connection` and `reports`, and `api::App` two,
  `plane` and `reports`, so a struct literal of either must set them, and
  `audit::Action` has five new variants (`Reveal`, `LegalReveal`,
  `WrongListener`, `ListenerFull` and `CspReport`), which a `match` that
  lists every variant must take. `console::FILES` holds six
  files, so its type is `[File; 6]`, where it was `[File; 4]`.
  `api::serve` takes any `api::Accept` where it took a
  `tokio::net::TcpListener`, which implements it. No public item is
  removed.

### Added

- An operator listener. With an `[operator]` table in the configuration,
  `serve` listens on a second address, `operator.listen`, and each address
  serves one kind of key: `listen` serves tenant keys only, and
  `operator.listen` serves operator keys only, so the routes that manage
  tenants and keys can sit on an address the internet does not reach, such
  as `127.0.0.1:8444`. A valid key that comes to the other listener is
  refused with 403 and the new problem type `wrong-listener`, before any
  route sees the request, on every route and on `/mcp`; a request with no
  key or a bad one is 401 on both, as before. Each such refusal is counted
  in the audit log under the new action `wrong_listener`, one record for
  each key and minute: in the admin log for an operator key and in the
  tenant's log for a tenant key. The operator listener presents `tls.cert`
  unless `[operator.tls]` names a certificate and key of its own. Both
  listeners serve the console, which tells a key of the other kind to use
  the other address. `serve` binds both addresses before it serves either,
  and logs `the operator listener is bound operator_listen=<address>`
  before its `serving` line. Without the table nothing changes: one
  listener serves both kinds of key. The operator-listener page has the
  how-to, with what to change in proxies and clients.
- The console plays a recording that a vCon holds inside a dialog: one
  **Play** button for each dialog whose `body` is `base64url` or `base64`
  and whose `mediatype` is `audio/wav`, `audio/mpeg`, `audio/ogg` or
  `audio/opus`, up to 104,857,600 bytes. Six other names are read as the
  type each stands for: `audio/x-wav`, `audio/wave` and `audio/vnd.wave`
  as `audio/wav`; `audio/x-mp3` and `audio/mp3` as `audio/mpeg`; and
  `audio/x-ogg` as `audio/ogg`. `application/ogg` is not played, because
  the name does not say whether the stream is audio. The player is a page of its own,
  `/console/player.html`, opened in a new tab. Its content-security policy
  alone has `media-src blob:`, and has `connect-src 'none'`; the policy of
  every other console answer is unchanged. The console reads the vCon with
  the key as a reveal of `audio`, recorded as `reveal_audio` or
  `legal_reveal_audio`, checks that the recording's bytes carry the
  signature of the type the dialog declares, and hands the bytes to the
  player, which has no key, makes no request, keeps nothing in the browser,
  never starts by itself and lets go of the recording when playback ends or
  the tab closes. A recording at a link is never loaded. The console page
  has the how-to. A proxy that passes the console's paths by name must also
  pass `/console/player.html` and `/console/player.js`.
- A how-to, "Get a Let's Encrypt certificate": a free, publicly trusted
  certificate in place of the self-signed one, by HTTP-01 or DNS-01 with
  certbot, with a deploy hook that installs each new certificate for the
  server and restarts it. Clients then need no CA file. CI runs the page
  once per challenge, with Pebble, Let's Encrypt's test server, and BIND in
  its place. Getting started, the how-to list and the home page link it.
- The terminal UI's page and the home page show its list and detail
  screens, drawn by the UI's own code: a test fails when the site's copy
  differs from what it draws.
- A read of a vCon can be marked as a reveal of one part of it:
  `GET /vcons/{uuid}?reveal=<part>`, and the same parameter on
  `GET /legal/vcons/{uuid}` and
  `GET /legal/vcons/{uuid}/versions/{version}`, with the part `parties`,
  `subject`, `dialog`, `analysis`, `links`, `all` or `audio`. The answer is the one
  the read gives without the parameter, and the key needs the same scope:
  the parameter only changes the action the audit log records, to
  `reveal_<part>`, or `legal_reveal_<part>` on a legal route. A `reveal`
  that is not the only parameter, is given twice or names no part is
  refused with `bad-query` (400). A read without the parameter is recorded
  as `read` or `legal_read`, as before. The audit-logs page has the how-to
  for listing who revealed what.
- Reports of a broken console policy. Both of the console's
  content-security policies now end with
  `report-uri /console/csp-report`, so a browser that blocks something in
  a console page reports it to the server. The policies do not use
  `report-to`: in the browser tests Chromium, given `report-to`, sent no
  report to the test server within 100 seconds, and with `report-uri`
  alone it sent one within a second. The new route `POST /console/csp-report` takes no key, since a
  browser sends a report with none. It reads at most 16384 bytes, checks
  that the body is JSON of the shape its media type names
  (`application/csp-report` or `application/reports+json`), stores and
  logs nothing of it, and answers 204; a request it refuses is answered
  415, 413, 408 or 400. Each request is counted in the admin audit log
  under the new action `csp_report` and the status that answered it, one
  record for each status and minute. Two new configuration keys bound the
  writes that a sender without a key can cause: a listener takes at most
  `limits.csp_reports` requests at the route in a minute (default 120)
  and `limits.csp_reports_per_address` from one source address (default
  20). A request past either is answered 429 with a `Retry-After` header
  and no write of its own; the refusals of a minute are written as one
  record with status 429. The console page says how to list
  them.

### Changed

- **Breaking for a check that holds the console's content-security policy
  byte for byte, and for a reader of the audit log that knows a fixed list
  of actions.** Both policies gained the reporting directive named above
  and no other change; such a reader must add `csp_report`. A proxy that
  passes only listed paths must pass `POST /console/csp-report`.
- For users of the `vcon-server` library: `console` has a new constant
  `REPORT_PATH`, `console::router()` answers a report at that path with
  204 and counts nothing, `audit::Action` has a new variant `CspReport`,
  `config::Config` has a new field `reports`, `api::App` has a new field
  `reports` (an `api::ReportGate` made from `api::ReportLimits`), and
  `audit` has a new function `count_many`.

- **Breaking for a reader of the audit log that knows a fixed list of
  actions, or that counts the console's reveals as `read`.** The console
  now marks each Reveal, so the tenant log holds `reveal_parties`,
  `reveal_subject`, `reveal_dialog`, `reveal_analysis`, `reveal_links` or
  `reveal_all` where 0.4.2 wrote `read`, and the `legal_reveal_` actions
  with the same six endings where it wrote `legal_read`. Playing a
  recording, which 0.4.2 could not do, is recorded as `reveal_audio` or
  `legal_reveal_audio`. Opening a vCon and **Download** are still `read`.
  Upgrade steps: before installing the new binary, add the fourteen action
  names to any rule, parser or report that lists actions; to keep a count of every read of content, count the
  actions that start with `reveal_` together with `read`, and those that
  start with `legal_reveal_` together with `legal_read`. The record's
  fields, the catalogs' schema version and every API answer are unchanged,
  and no other client is affected.
- The description at `/openapi.json` lists 403 for `GET /health`,
  `GET /key` and `GET /openapi.json`, which answer it only on a server
  with an operator listener, and names `wrong-listener` among the problem
  types. A reader of the audit log that knows a fixed list of actions
  must add `wrong_listener`; a server without an `[operator]` table never
  writes it.
- For users of the `vcon-server` library: `config::Config` has a new
  member, `operator`, and `api::App` a new member, `plane`, so code that
  builds either with a struct literal must set them (`None` and
  `access::Plane::Both` keep the behavior as it was).
- The console is made of two more files, `/console/player.html` and
  `/console/player.js`. A reverse proxy or firewall that passes the
  console's paths by name must pass both, and must pass each answer's
  `Content-Security-Policy` header unchanged: the player's page has a
  policy of its own. A proxy that passes everything under `/console/`
  needs no change.
- The documentation's sidebar is folded behind one button on a narrow
  screen, so a page's own content comes first; without JavaScript it is
  shown as before.
- Three documentation pages each keep to one kind of writing. The audit
  log's record members and actions are on a new page, Audit log records,
  and the steps for reading a log on a new guide, Read the audit logs; the
  operations page's steps are on the rotation and backup guides and
  `check`'s members on the command-line page; the MCP client configuration
  is a step of the Connect an AI agent guide. Every heading the three
  pages had is still there, so a link to one still lands on it.
- The PostgreSQL catalog guide's commands are run by CI on each change, as
  the other guides' are.
- The `parquet` crate, which writes `export parquet` files and the pack
  files of Parquet storage, goes from 60.0.0 to 60.1.0. The files written
  and read are checked as before: the export round trip, the DuckDB reads
  and Parquet storage on an S3 store all pass unchanged.
- rusqlite goes from 0.41.0 to 0.41.1, and every dependency now comes from
  crates.io. rusqlite 0.41.0's pragma helpers refused SQLCipher's own
  pragma names, so the server set `cipher_log_level` as plain SQL, and the
  tests used a fork of rusqlite from github.com to check that the helpers
  would read the same values. 0.41.1 takes the names: the server sets and
  reads SQLCipher's settings through the helpers, and the fork, the
  `[patch]` that gave it the server's libsqlite3-sys, and its entry in
  cargo-deny's allowed sources are gone. A build from source, tests
  included, no longer fetches from github.com; with a mirror of crates.io
  or `cargo vendor` it needs no other host. Dependabot proposes rusqlite
  updates again, which the fork had made it skip. The catalogs are opened
  with the same settings as before, and tests hold the manifests, both
  lockfiles and the pages to having no git source.

### Fixed

- The `vcon` crate's README and the library page passed
  `application/vcon+json` to `vcon::read::inspect` in their example. The
  vCon draft registers `application/vcon` and `application/vcon+gzip`, and
  the server refuses any other vCon media type with
  `unsupported-media-type`; the example now passes `application/vcon`, and
  a test holds every vCon media type the pages name to the ones the API
  takes.
- The site's pages no longer fetch the search index, a file of over a
  megabyte, until the search box is used.
- The documentation's sidebar and index list the pages under five group
  headings, starting with install, getting started and the first vCon; no
  two pages share a place in the order, as five pairs did. Each how-to guide says what
  to do when a step fails, `llms.txt` links every page once, and between
  releases the status page lists what the newest release does not hold
  yet.
- Seven words on the site were not in US spelling, and the
  tenants-and-keys page left `GET /key` out of the routes any valid key
  may ask.
- A server stopped by the PostgreSQL catalog preview said to set
  `catalog.backend = "local"` to serve. A configuration with that and the
  `[catalog.postgres]` table still in it is refused, so the message now
  says to remove the `[catalog]` and `[catalog.postgres]` tables. Only a
  build with the `postgres` feature prints it.
- The upgrade and operations pages made `/backup` a directory for root
  alone (`install -d -m 0700 /backup`), after which `vcon-server backup
  /backup/vcon-server/...`, run as `vcon` as the backup page shows, was
  refused. The copies root makes now go in `/backup/stopped`.
- The operations page's list of what `check` prints left out four members,
  the tenants-and-keys page said a tenant under a legal hold is refused
  with `under-hold` whatever the query (without `force=true` the answer is
  `holds-vcons`), and the development page's layout left out seven source
  files.
- On the site, the search box's edge had a contrast of 1.2 to 1 against the
  page (WCAG 1.4.11 asks 3 to 1), and the focus ring of the sidebar's links
  and of the home page's code blocks was cut off by the box around them.
- A listener holds at most `limits.connections` connections at once, 512
  unless the configuration says another number from 1 to 65536. Before,
  it took every connection it was offered, so connections alone could use
  up the process's file descriptors, and with them the catalogs and
  objects the server must open to answer anyone. At the limit the
  listener takes no more until one ends or a time limit closes it; a new
  connection waits in the system's queue. Each listener has its own
  places, so connections to the tenant listener take none from an
  operator listener. Keep the number below the process's limit on open
  files. A listener that fills logs one warning, at most one a minute,
  with its own address and the limit, and each time it fills is counted
  in the admin audit log under the new action `listener_full`, with the
  listener's configuration key as the target. The record answers no
  request, so its `status` is `null`, the first record whose status is not
  a number; its `count` is the times the listener became full, not the
  connections that waited. A reader of the audit log that knows a fixed
  list of actions must add `listener_full`, and one that reads `status`
  as a number must take `null` for it.
- A connection that waits for its next request is closed after
  `limits.timeouts.idle` seconds, 120 unless the configuration says
  another number from 1 to 3600. Before, an HTTP/2 connection that had
  sent one request was kept for as long as the other side kept it, with
  its file descriptor. The limit applies to an HTTP/2 connection with no
  stream open and to an HTTP/1.1 connection between requests. An HTTP/2
  connection is sent GOAWAY with `NO_ERROR`, a request that arrives at
  that moment is still answered, and the connection is closed five seconds
  later if the client has not ended it. No limit runs while a request is
  being answered. A client finds the connection closed and opens another,
  as HTTP clients do without being told. An idle HTTP/1.1 connection is
  kept up to 120 seconds; `idle = 30` in `[limits.timeouts]` closes it
  after 30.
- For users of the `vcon-server` library, all new in this release:
  `api::ConnectionLimits` holds the limits (`max`, `handshake`,
  `first_head`, `head`, `idle`, `goodbye` and `accept_pause`);
  `api::serve_within` is `api::serve` with limits given;
  `api::serve_observed` is `api::serve_within` with a function called each
  time the listener fills; and `api::Accept`, what a listener must do, has
  `accept` and a method with a default, `local`, the listener's address
  for the log.
- The server no longer stops when the system cannot hand it a connection.
  Before, one failed accept, such as the one a process out of file
  descriptors gets, ended `serve` with `serving failed`. Now the listener
  goes on: at once when the failure was one connection's own (it ended
  before it was taken), and after one second otherwise, with a warning in
  the log each time.
- A connection that stalls is closed, where before it was kept for as long
  as the other side kept it, with its file descriptor. A connection is
  closed when its TLS handshake is not done 10 seconds after it was taken,
  when no request head has arrived 30 seconds after the handshake, and when
  an HTTP/1.1 request head on it takes more than 30 seconds from its first
  bytes. The three limits are new configuration keys,
  `limits.timeouts.handshake`, `limits.timeouts.first_head` and
  `limits.timeouts.head`, each from 1 to 3600 seconds, with those
  defaults. The wait for the next request on a connection kept open is
  under its own key, `limits.timeouts.idle`. For users of the `vcon-server`
  library: `config::Config` has a new member, `connection`, and
  `api::serve` takes any `api::Accept`, which `tokio::net::TcpListener`
  implements.
- A SIGTERM or SIGINT that arrives as soon as `serve` has written its
  `serving` line stops the server in order, with exit status 0. Before,
  the server began to take the two signals only after that line, so a
  supervisor or script that stopped it at once could find it killed by
  the signal instead. `serve` now takes both signals before it opens the
  data directory, and a server that cannot take them does not start
  (`the stop signals cannot be taken`), where before it started and then
  stopped at once.
- The release build no longer depends on one package mirror. Its
  AlmaLinux 9 container installed git and the build tools with
  `dnf install`, retried five times; on 2026-11-11 each of the five tries
  read the same mirror, whose `repomd.xml` dnf could not parse, and the
  build job failed. dnf asks the next mirror only when a download fails,
  not when a file arrives and does not parse. `scripts/dnf_install.py` now
  follows a failed try with one from the address each repository file
  names in its commented `baseurl` line, the distribution's own server,
  and takes turns between the two sources for at most five tries. Packages
  are checked against the distribution's key as before. The build jobs
  check out twice, the first time for the script and the second, with git
  installed, for `.git`.
- The release's smoke job no longer depends on one host for each package
  source. In its debian:12 and ubuntu:22.04 containers it ran `apt-get`
  against the one host the image names for each source, and apt's retries
  ask that host again: the failure the how-to runner had on 2026-11-10,
  when archive.ubuntu.com did not answer on port 80. The job now runs
  `scripts/apt-mirror-setup.sh` before its first `apt-get`. The script
  asks each mirror of each source the system reads for its index, keeps
  those that answer within five seconds, and gives apt that list through
  its `mirror+file:` method; when no mirror of a source answers it changes
  nothing and ends with exit code 3. The mirrors are one table,
  `scripts/apt-mirrors.txt`, which the how-to runner reads too.
  `scripts/check_workflow_downloads.py` refuses an `apt-get update` or
  `install` in a container job that comes before the script.
- `vcon-server tui` is now tested on a terminal. Until now the part of it
  that takes the terminal (keys one at a time and not echoed, the alternate
  screen), reads the keyboard and starts a thread for each request had been
  run by hand only. `tests/tui_terminal.rs` runs the binary on a
  pseudo-terminal and checks the terminal's modes and screen while it runs
  and after `q`, Ctrl-C, a new size, a failed request and a start that
  fails. A new library function, `tui::with_terminal`, is the one place the
  terminal is taken and put back; the tests use it to check that a panic
  puts the terminal back before its message is written.
- CI's object-store jobs wait until the store is ready, not until it first
  answers. MinIO listens before it is initialized and answers 503 until it
  is; on 2026-11-11 the MinIO job took that first answer as ready and
  failed at its first write. `scripts/wait-for-s3.sh` waits for a signed
  request for the list of buckets to answer 200, for at most 60 seconds,
  and the MinIO job and the two jobs with a RustFS service use it.
- The tests of the commit hook no longer run every script's own test once
  per case. `scripts/test-hooks.sh` commits one change per case, and each
  commit the scan accepts runs the whole hook, which runs the 23 script
  tests; CI's "git hooks" job runs that twice and took 15 minutes of its 20
  on 2026-11-11, after a 12-second script test had ended it at the limit.
  The fixture's first commit still runs every script test as it is. After
  it, `scripts/hook_fixture.py` replaces each one no case exercises with a
  stand-in that records its name and passes, and the test checks that the
  hook ran each stand-in once. The list is read from the hook, so a script
  test added to the hook needs no change there.

### Security

- rustix goes from 1.1.5 to 1.1.6 (RUSTSEC-2026-0486). Before 1.1.6,
  rustix's extended-attribute functions (`getxattr`, `listxattr` and their
  `l` and `f` forms) marked bytes as initialized that the operating system
  had not written when they were given a zero-length buffer, which is
  undefined behavior reachable from safe code. vcon-server takes rustix
  through crossterm, for the terminal UI, and through tempfile, in tests.
  No source file of vcon-server, of crossterm 0.29.0 or of tempfile 3.27.0
  names an extended-attribute function, so no call to the affected
  functions was found; the bump removes the affected version from the
  build all the same.

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

[Unreleased]: https://github.com/NormB/vcon-server/compare/v0.4.5...HEAD
[0.4.5]: https://github.com/NormB/vcon-server/releases/tag/v0.4.5
[0.4.4]: https://github.com/NormB/vcon-server/releases/tag/v0.4.4
[0.4.3]: https://github.com/NormB/vcon-server/releases/tag/v0.4.3
[0.4.2]: https://github.com/NormB/vcon-server/releases/tag/v0.4.2
[0.4.1]: https://github.com/NormB/vcon-server/releases/tag/v0.4.1
[0.4.0]: https://github.com/NormB/vcon-server/releases/tag/v0.4.0
[0.3.0]: https://github.com/NormB/vcon-server/releases/tag/v0.3.0
[0.2.0]: https://github.com/NormB/vcon-server/releases/tag/v0.2.0
[0.1.0]: https://github.com/NormB/vcon-server/releases/tag/v0.1.0
