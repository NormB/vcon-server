+++
title = "Roadmap"
description = "The next milestones for vcon-server."
weight = 61

[extra]
group = "project"
+++

These are the next milestones. They have no dates, and their order may
change. [What is built](@/docs/status.md) says what works today.

- **A PostgreSQL catalog, the rest of it:** a command that moves a data
  directory's tenants into PostgreSQL and back, and several servers on one
  database. On `main`, one server per database keeps the registry and the
  catalogs in PostgreSQL, as a preview behind the `postgres` cargo feature
  (see [what is built](@/docs/status.md#on-main-only) and
  [keep the catalogs in PostgreSQL](@/docs/postgres-catalog.md)).
- **The draft's recommended algorithms** for `vcon::write` (RS256,
  RSA-OAEP and A256CBC-HS512), as an option of the `vcon` crate.

A milestone is listed here before it is built and moves to
[What is built](@/docs/status.md) when it ships. The
[commit history](https://github.com/NormB/vcon-server/commits/main) records
each change as it lands.
