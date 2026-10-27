+++
title = "vcon-server"
description = "vcon-server stores vCons, the IETF format for conversation records, and returns them only to the keys allowed to see them. Each vCon is encrypted at rest under its own key, kept for its tenant's retention period, erased after it, and held past it when a legal hold says so. Every request is recorded."
[extra]
tagline = "encrypted, multi-tenant, audited storage for vCons"
headline = "Encrypted, multi-tenant, audited storage for vCons"
# The target of the hero's primary button.
install_link = "@/docs/getting-started.md"

# The texts around "A first look", inline markdown.
first_look_intro = "Send a vCon with a key that holds the `write` scope, then read it back."
first_look_outro = "Sending the same bytes again answers `200` with the same version, so a producer can retry safely."
# The Install and How-to sections: their text is the docs' (see the how-to
# pages); the layout is the template's. Plain text, not markdown. Every plain
# key sits above the first [[...]] table, or TOML puts it inside that table.
install_intro = "With Rust 1.99 or later, from crates.io. Getting started also covers the signed release archives and a build from source."
howto_intro = "Where to start, by what you are doing."

# "What it guarantees": one card each. `body` is inline markdown; `icon` names
# a drawing in templates/components.html.
[[extra.features]]
title = "Encrypted at rest, key by key"
icon = "lock"
body = "Each stored version is sealed with AES-256-GCM under its own data key, wrapped by its tenant's key, which is wrapped by the master key. The catalogs are SQLCipher databases."
link = "@/docs/security.md"
link_text = "Security"

[[extra.features]]
title = "Stored exactly as received"
icon = "file"
body = "Unsigned, signed (JWS) and encrypted (JWE) vCons, plain or gzipped, are checked and stored byte for byte. Nothing is rewritten, so a signature stays valid."
link = "@/docs/storing-vcons.md"
link_text = "Storing vCons"

[[extra.features]]
title = "Tenants kept apart"
icon = "tenants"
body = "Each tenant has its own key, catalog, files, retention period and audit log. A key belongs to one tenant and reaches nothing else."
link = "@/docs/tenants-and-keys.md"
link_text = "Tenants and keys"

[[extra.features]]
title = "Kept as long as required, then erased"
icon = "clock"
body = "A deleted vCon is hidden at once and purged once its tenant's retention period has passed since it was stored. A legal hold stops the purge."
link = "@/docs/retention-and-holds.md"
link_text = "Retention and legal holds"

[[extra.features]]
title = "Every request recorded"
icon = "log"
body = "Each tenant has an append-only audit log, and the server keeps one for its own administration. No log holds vCon content or a token."
link = "@/docs/audit-logs.md"
link_text = "Audit logs"

[[extra.features]]
title = "Errors you can act on"
icon = "alert"
body = "Every refusal is an RFC 9457 problem document whose `type` is a page on this site."
link = "@/problems/_index.md"
link_text = "Problem types"

# "A first look": one framed code block per step.

[[extra.first_look]]
title = "Store a vCon"
code = """
curl --fail-with-body https://vcon.example.com/vcons \\
  -H "Authorization: Bearer $VCON_KEY" \\
  -H "Content-Type: application/vcon" \\
  --data-binary @call.vcon
# 201 Created
# Location: /vcons/01a07da8-c2bb-83e5-b9a2-279e0d16bc46
# ETag: "…version id…""""

[[extra.first_look]]
title = "Read it back"
code = """
curl --fail-with-body https://vcon.example.com/vcons/01a07da8-c2bb-83e5-b9a2-279e0d16bc46 \\
  -H "Authorization: Bearer $VCON_KEY""""

[[extra.install]]
title = "Install from crates.io"
code = """
cargo install vcon-server --locked
vcon-server --version"""
note = "The build compiles SQLCipher, OpenSSL and AWS-LC, so the host needs a C compiler, make and perl."


[[extra.howto]]
title = "Running a server"
link = "@/docs/getting-started.md"
summary = "Getting started, then configuration and operations."

[[extra.howto]]
title = "Sending vCons from a producer"
link = "@/docs/producers.md"
summary = "Producers and the HTTP API."

[[extra.howto]]
title = "Working on the code"
link = "@/docs/development.md"
summary = "Building, testing, fuzzing and contributing."
+++

> **Status: 0.2.0 released, not yet for production.** The store, the HTTP API
> with its client routes and OpenAPI description, tenants and their keys,
> retention and the purge sweep, legal holds, the audit logs, the console, the
> command line, key rotation and `check` are built and tested. The MCP
> endpoint is built; its tools are being built. See
> [what is built](@/docs/status.md).

## What it is for

A vCon holds a conversation: who took part, the dialog itself (text, or a
recording inline or by URL), and what was learned from it (transcripts,
summaries, analysis). That is some of the most sensitive data a business keeps,
and much of it must be kept for a fixed time and then destroyed, unless a
court says otherwise.

vcon-server is the place a producer sends vCons to and the place they are read
back from: an HTTP API over an encrypted store, with tenants kept apart, every
request recorded, and retention periods and legal holds applied to each vCon.
