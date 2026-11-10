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
| `actor` | the key id; `sweep` for the purge sweep; `cli` for a command on the host; empty for a token that did not parse or names no known key |
| `action` | what was asked: one of the [actions](#every-action) below |
| `target` | the uuid, tenant id or key id acted on, if any |
| `version` | the version id involved, if any |
| `status` | the HTTP status answered |
| `count` | how many requests the record stands for |

A record holds no vCon content, no token and no client address.

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
| `auth_failed` | a request whose token was refused, [counted](@/docs/audit-logs.md#counted-records) | the key's tenant's when the key is known, otherwise the admin log with an empty actor |
| `mcp` | a request to `/mcp`, [counted](@/docs/audit-logs.md#counted-records) | the key's |
| `wrong_listener` | a valid key refused because it came to the listener that serves the other kind of key, on a server with an [operator listener](@/docs/operator-listener.md), [counted](@/docs/audit-logs.md#counted-records) | the admin log for an operator key; the tenant's for a tenant key, or the admin log when the tenant's cannot be opened |
| `key_create` | a key made, over the admin API or by `vcon-server key create` | admin, copied to the tenant's for a tenant key |
| `key_revoke` | a key revoked | admin, copied to the tenant's for a tenant key |
| `key_list` | the keys listed | admin |
| `tenant_create` | a tenant made | admin |
| `tenant_list` | the tenants listed | admin |
| `tenant_retention` | a tenant's retention period changed | admin |
| `tenant_metadata` | a tenant's metadata level changed (`PUT /admin/tenants/{id}/metadata`, `vcon-server tenant set-metadata`) | admin |
| `tenant_remove` | a tenant removed | admin |
| `rekey_tenant` | a tenant key rotated (`vcon-server rekey tenant`) | admin |
| `rekey_master` | the master key rotated (`vcon-server rekey master`) | admin |
| `backup` | a run of `vcon-server backup`, with its outcome | admin |
