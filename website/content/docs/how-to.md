+++
title = "How-to guides"
description = "Step-by-step guides for installing and running vcon-server. Each shows every command and its output, and CI runs each against a live server."
weight = 7
+++

Each guide gives every command to paste, in order, and what the command
prints. CI runs the commands of each guide against a live server on each
change, and fails when a command prints something other than the guide
shows. `scripts/docs_test.py` runs them. See
[development](@/docs/development.md#the-how-to-pages-are-tested).

## Install and start

- [Install](@/docs/install.md): download a release and check it, install
  with cargo, or build from source, with what each Linux distribution needs
  first.
- [Getting started](@/docs/getting-started.md): the service account, the
  master key, a certificate, the configuration and the first keys. Then the
  server is started under systemd or in the foreground.

## Use it

- [Store your first vCon](@/docs/first-vcon.md): store, store again, see a
  conflict refused, read, list and delete a vCon, with a key for each scope.

## Connect an AI agent

- [MCP](@/docs/mcp.md): connect an AI agent over HTTPS at `/mcp`, or with
  `vcon-server mcp`. The server has no MCP tools yet.
