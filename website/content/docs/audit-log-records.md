+++
title = "Audit log records"
description = "The members of an audit record, and every action a record can name, with what it records and which log holds it."
weight = 57

[extra]
group = "reference"
+++

Each line of an audit log is one record. This page lists a record's
members and every action. [Audit logs](@/docs/audit-logs.md) explains what
is recorded and why, and [read the audit logs](@/docs/read-audit-logs.md)
shows how to read them.

## A record

| Field | Holds |
|---|---|
| `seq` | the record's number in its log, rising |
| `at` | when, RFC 3339 in UTC |
| `actor` | the key id; `user:` and the user id for a request by session; `sweep` for the purge sweep; `cli` for a command on the host; empty for a token that did not parse or names no known key, and for a request by session refused before its session is known |
| `action` | what was asked: one of the [actions](#every-action) below |
| `target` | the uuid, tenant id, key id or user id acted on, if any; for `listener_full`, the configuration key of the listener, `listen` or `operator.listen`; for `membership_grant` and `membership_revoke`, the user id, then the tenant id or `operator`, then the roles the command named if it named any, joined by `/`; for `role_cap_set`, the tenant id, then the cap's roles or `all`, joined by `/`; for `settings_set`, the tenant id, then after a `/` the names of the settings changed, joined by `,`, and never a value |
| `version` | the version id involved, if any |
| `status` | the HTTP status answered, or `null` for a record that answers no request (`listener_full`) |
| `count` | how many requests the record stands for; for `listener_full`, how many times the listener became full, not how many connections waited |

A record holds no vCon content, no token and no client address, and nothing
of a user but its id: no label, issuer or subject.

## Every action

Each action, what it records, and the log it is in. "The key's log" is the
tenant log of the key's tenant, or the admin log for an operator key. An
action through MCP is recorded as its HTTP route records it.

| Action | Records | Log |
|---|---|---|
| `create` | a vCon stored (`POST /vcons`) | the tenant's |
| `replace` | a vCon replaced by a new version (`PUT /vcons/{uuid}`) | the tenant's |
| `read` | a vCon read (`GET /vcons/{uuid}`) | the tenant's |
| `reveal_parties` | a vCon read as a [reveal](@/docs/audit-logs.md#reveals) of the parties (`GET /vcons/{uuid}?reveal=parties`) | the tenant's |
| `reveal_subject` | a vCon read as a [reveal](@/docs/audit-logs.md#reveals) of the subject (`GET /vcons/{uuid}?reveal=subject`) | the tenant's |
| `reveal_dialog` | a vCon read as a [reveal](@/docs/audit-logs.md#reveals) of the dialog (`GET /vcons/{uuid}?reveal=dialog`) | the tenant's |
| `reveal_analysis` | a vCon read as a [reveal](@/docs/audit-logs.md#reveals) of the analysis (`GET /vcons/{uuid}?reveal=analysis`) | the tenant's |
| `reveal_links` | a vCon read as a [reveal](@/docs/audit-logs.md#reveals) of the links (`GET /vcons/{uuid}?reveal=links`) | the tenant's |
| `reveal_all` | a vCon read as a [reveal](@/docs/audit-logs.md#reveals) of the whole vCon (`GET /vcons/{uuid}?reveal=all`) | the tenant's |
| `reveal_audio` | a vCon read as a [reveal](@/docs/audit-logs.md#reveals) of one dialog's recording, to play it (`GET /vcons/{uuid}?reveal=audio`) | the tenant's |
| `delete` | a vCon deleted (`DELETE /vcons/{uuid}`) | the tenant's |
| `list` | a page of the vCon list (`GET /vcons`) | the tenant's |
| `purge` | a version erased, by the purge sweep (actor `sweep`), or at once by a delete when the period is `0d` | the tenant's |
| `legal_read` | a read through the legal routes of the latest kept version or of one version (`GET /legal/vcons/{uuid}`, `GET /legal/vcons/{uuid}/versions/{version}`) | the tenant's |
| `legal_reveal_parties` | a legal read as a [reveal](@/docs/audit-logs.md#reveals) of the parties (`?reveal=parties` on either legal read route) | the tenant's |
| `legal_reveal_subject` | a legal read as a [reveal](@/docs/audit-logs.md#reveals) of the subject (`?reveal=subject` on either legal read route) | the tenant's |
| `legal_reveal_dialog` | a legal read as a [reveal](@/docs/audit-logs.md#reveals) of the dialog (`?reveal=dialog` on either legal read route) | the tenant's |
| `legal_reveal_analysis` | a legal read as a [reveal](@/docs/audit-logs.md#reveals) of the analysis (`?reveal=analysis` on either legal read route) | the tenant's |
| `legal_reveal_links` | a legal read as a [reveal](@/docs/audit-logs.md#reveals) of the links (`?reveal=links` on either legal read route) | the tenant's |
| `legal_reveal_all` | a legal read as a [reveal](@/docs/audit-logs.md#reveals) of the whole vCon (`?reveal=all` on either legal read route) | the tenant's |
| `legal_reveal_audio` | a legal read as a [reveal](@/docs/audit-logs.md#reveals) of one dialog's recording, to play it (`?reveal=audio` on either legal read route) | the tenant's |
| `legal_versions` | a read of a vCon's version history | the tenant's |
| `restore` | a deleted vCon made live again | the tenant's |
| `hold` | a legal hold placed | the tenant's |
| `release` | a legal hold released | the tenant's |
| `audit_read` | a page of a log read over HTTPS or MCP (`GET /audit`, `GET /admin/audit`) | the log read |
| `retention` | the tenant's retention period changed, copied from the admin log's `tenant_retention` | the tenant's |
| `metadata` | the tenant's metadata level changed, copied from the admin log's `tenant_metadata` | the tenant's |
| `export` | a run of `vcon-server export parquet`, with its outcome (actor `cli`) | the tenant's |
| `key_info` | a key reading what it may do (`GET /key`) | the key's |
| `openapi` | a read of `GET /openapi.json` | the key's |
| `health` | a health check, [counted](@/docs/audit-logs.md#counted-records) | the key's |
| `auth_failed` | a request whose token was refused (status 401); a request with both a key and the session cookie (400); a request by session refused for its cookie (401), its origin (403) or its CSRF token (403); [counted](@/docs/audit-logs.md#counted-records) | the key's tenant's when the key is known, and the session's tenant's, under `user:` and the user id, for a wrong CSRF token; otherwise the admin log with an empty actor |
| `mcp` | a request to `/mcp`, [counted](@/docs/audit-logs.md#counted-records) | the key's |
| `wrong_listener` | a valid key, or a live session, refused because it came to the listener that serves the other kind, on a server with an [operator listener](@/docs/operator-listener.md), [counted](@/docs/audit-logs.md#counted-records) | the admin log for an operator key or session; the tenant's for a tenant key or session, or the admin log when the tenant's cannot be opened |
| `listener_full` | a listener that held [`limits.connections`](@/docs/configuration.md#the-most-connections-at-once) connections and took no more until one ended. The record describes a state of the listener and answers no request: its actor is empty, its `status` is `null`, and its `count` is the times the listener became full in the minute, not the connections that waited; [counted](@/docs/audit-logs.md#counted-records) | admin |
| `csp_report` | a request to `POST /console/csp-report`, where a browser reports that a [console](@/docs/console.md#reports-of-a-broken-policy) page broke its content-security policy. The route takes no key, so the actor is empty; `status` is 204 for a report taken, 400, 408, 413 or 415 for a request refused, or 429 for the requests refused in the minute past the listener's number of reports (one record, written when the minute ends); nothing the report says is kept; [counted](@/docs/audit-logs.md#counted-records) | admin |
| `key_create` | a key made, over the admin API, at `POST /tenant/keys` or by `vcon-server key create` | admin, copied to the tenant's for a tenant key |
| `key_revoke` | a key revoked, over the admin API, at `DELETE /tenant/keys/{id}` or by `vcon-server key revoke` | admin, copied to the tenant's for a tenant key, and for a refusal at `DELETE /tenant/keys/{id}` to the log of the tenant whose key asked |
| `key_list` | the keys listed | admin, copied to the tenant's for `GET /tenant/keys` |
| `user_add` | a user made by `vcon-server user add`; the target is the new user's id, and a refused one has none | admin |
| `user_list` | the users listed by `vcon-server user list` | admin |
| `user_deactivate` | a user deactivated and its memberships removed, by `vcon-server user deactivate` | admin, copied to the log of each tenant the user held a membership in |
| `membership_grant` | roles given to a user in a tenant, or the operator role, by `vcon-server membership grant` | admin, copied to the tenant's for a tenant membership |
| `membership_revoke` | roles taken from a user's membership, by `vcon-server membership revoke` | admin, copied to the tenant's for a tenant membership |
| `role_cap_set` | a tenant's role cap set or removed, by `vcon-server tenant set-role-cap` | admin, copied to the tenant's |
| `role_cap_read` | a tenant's role cap read, by `vcon-server tenant role-cap` | admin |
| `settings_read` | a tenant's settings read, at `GET /tenant/settings`; the target is the tenant id | admin, copied to the tenant's |
| `settings_set` | a tenant's settings changed, at `PUT /tenant/settings`; a refused change is recorded too | admin, copied to the tenant's |
| `membership_list` | a tenant's memberships listed, at `GET /tenant/memberships`; the target is the tenant id | admin, copied to the tenant's |
| `session_start` | a session started for a user who signed in; the actor is `user:` and the user id, and the target is the user id, then the tenant id or `operator`, joined by `/`; a refused start is recorded too. No sign-in method starts one yet | admin, copied to the tenant's |
| `session_read` | a session read by its own browser, at `GET /session`, one record for each user and minute | the tenant's; admin for an operator's session |
| `session_end` | a session ended by its user, at `POST /session/logout`; actor and target as for `session_start` | admin, copied to the tenant's |
| `tenant_create` | a tenant made | admin |
| `tenant_list` | the tenants listed | admin |
| `tenant_retention` | a tenant's retention period changed | admin |
| `tenant_metadata` | a tenant's metadata level changed (`PUT /admin/tenants/{id}/metadata`, `vcon-server tenant set-metadata`) | admin |
| `tenant_remove` | a tenant removed | admin |
| `rekey_tenant` | a tenant key rotated (`vcon-server rekey tenant`) | admin |
| `rekey_master` | the master key rotated (`vcon-server rekey master`) | admin |
| `backup` | a run of `vcon-server backup`, with its outcome | admin |
