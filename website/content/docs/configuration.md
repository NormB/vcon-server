+++
title = "Configuration"
description = "Every key of the configuration file, its default, and why the default is what it is."
weight = 20
+++

`vcon-server serve --config <file>` reads one TOML file. A wrong file stops the
start with an error that names the key. A key the server does not know is an
error too, so a misspelt key is never silently ignored.

Durations are whole seconds and sizes are bytes; each must be more than zero.

## Keys

| Key | Meaning | Default |
|---|---|---|
| `listen` | Address and port to serve on, such as `0.0.0.0:8443` | required |
| `data_dir` | Directory holding `server.sqlite` and the tenant directories | required |
| `master_key` | File holding the 32-byte master key | required |
| `tls.cert` | PEM certificate chain | required |
| `tls.key` | PEM private key | required |
| `retention_default` | Retention period for tenants that set none: years, months or days, written `7y`, `18m` or `90d` | `7y` |
| `purge_interval` | Seconds between purge sweeps | `3600` (1 hour) |
| `rotation_warn` | Seconds the oldest work may wait for key rotation before a warning is logged | `2592000` (30 days) |
| `limits.body` | Largest request body, in bytes | `16777216` (16 MiB) |
| `limits.inflated` | Most bytes gzip inflation may produce while one body is checked | `67108864` (64 MiB) |
| `limits.timeouts.read` | Seconds a request body may take to arrive | `60` |
| `supported_critical` | Extension names accepted in a vCon's `critical` list, for all tenants | none |

At start and then each day, `serve` logs a warning while the oldest purge or
tenant removal waiting for [key rotation](@/docs/operations.md#key-rotation)
is older than `rotation_warn`.

## A full file

```toml
listen = "0.0.0.0:8443"
data_dir = "/var/lib/vcon-server"
master_key = "/etc/vcon-server/master.key"
retention_default = "7y"
purge_interval = 3600
rotation_warn = 2592000
supported_critical = []

[tls]
cert = "/etc/vcon-server/cert.pem"
key = "/etc/vcon-server/key.pem"

[limits]
body = 16777216
inflated = 67108864

[limits.timeouts]
read = 60
```

A key left out of a table that is present takes its default; the others keep
their written values.

## Where the defaults come from

The size limits were set from measurement, not guessed. Of 34,885 vCon files
measured (public datasets and recorded test calls), the largest was 5,141,813
bytes: a two-minute call with its recording held inline, as an 8 kHz 16-bit
stereo WAV (42,671 encoded bytes a second). Only 26 of their 25,471 recordings
were inline; the other 25,445 were referenced by URL.

- `limits.body` at 16 MiB holds that file three times over, or about six and a
  half minutes of that recording format inline. Longer recordings should be
  sent by URL, or the limit raised.
- `limits.inflated` is four times the body limit, so a vCon accepted plain is
  accepted gzipped too, with room left for gzipped payloads inside signed
  layers. It bounds the work a small, highly compressed body can cause.
- A 16 MiB body arriving within 60 seconds needs about 2.2 Mbit/s.

The server holds each request's body in locked memory while it works on it. On
a host short of memory, lower `limits.body`.

## Fixed limits

Two limits on how deeply a body nests are fixed: no key changes them. A body
past either is refused with [`too-deep`](@/problems/too-deep.md) before
anything deeper is read.

| Limit | Value | Counted |
|---|---|---|
| Signed layers in one body | `64` | JWS objects from the body to the unsigned vCon: `1` for a vCon signed once |
| Arrays and objects open at once | `64` | in each JSON text read on its own: the body, each signed payload; the outer object counts as `1` |

- **Signed layers.** The vCon core draft (section 5.2.3, Long Term Archiving
  of Signed vCons) signs an archived vCon again, as the payload of a new JWS,
  before each signing certificate expires, for archives kept "as long as 10
  years or more" with certificates that last "a few years at best". A
  signed vCon encrypted (a JWE) is not opened, so it adds no layer. `64`
  allows a new signature every year for 63 years. Of 51,642 vCons in the
  public datasets measured, none was signed; the draft's own signed example
  has one layer.
- **Arrays and objects.** The deepest of those vCons nests 5 deep, and the
  draft's examples 6. `64` is ten times that, and below the JSON parser's own
  bound, so a body this deep is told apart from one that is not JSON.

Each layer is read in turn, not by recursion, so the stack a request needs
does not grow with the layers.

## Retention periods

A period is a whole number, with no leading zero, followed by `y`, `m` or `d`. It is counted in
calendar units from the moment a version was stored, in UTC; a day past the
end of a month clamps, so a version stored on 29 February with a period of
`1y` falls due on 28 February. `0d` is allowed: a deleted vCon is then
purged as soon as it is deleted, unless it is under a legal hold. See [retention and legal holds](@/docs/retention-and-holds.md).

## Extensions in `critical`

A vCon may list, in `critical` (or `must_support`, its older name), extensions
a reader must understand. vcon-server refuses such a vCon with
[`unsupported-extension`](@/problems/unsupported-extension.md) unless every
name is in `supported_critical`. Name an extension there only if storing it
unread is right for you: the server never interprets vCon content.
