+++
title = "Roadmap"
description = "The next milestones for vcon-server."
weight = 6
+++

These are the next milestones. They have no dates, and their order may
change. [What is built](@/docs/status.md) says what works today.

- **Storage backends:** keeping vCons in PostgreSQL and in S3-compatible
  object storage, beside today's local data directory.
- **The draft's recommended algorithms** for `vcon::write` (RS256,
  RSA-OAEP and A256CBC-HS512), as an option of the `vcon` crate.

A milestone is listed here before it is built and moves to
[What is built](@/docs/status.md) when it ships. The
[commit history](https://github.com/NormB/vcon-server/commits/main) records
each change as it lands.
