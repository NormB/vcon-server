+++
title = "Scale numbers"
description = "Measured ingest rates, read latencies and disk use, each with the machine, the commit and the vCon count, and how to run the harness that measures them."
weight = 64

[extra]
group = "project"
+++

The scale harness, `vcon-scale`, stores synthetic vCons in a running
server over HTTPS, reads some of them back, follows list pages, and
writes a result file. This page shows every result file in the
repository. A test holds the table to the files, and CI runs the harness
at 100,000 vCons on each change. The [1.0 checklist](@/docs/production-checklist.md#published-scale-numbers)
names what is left: 10 million and 100 million vCons.

## Results

| Run | Machine | vCons | Clients | Stores a second | Read p50 | Read p99 | Read p99.9 | Errors | Bytes per vCon | Peak server memory | Commit |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| pilot-10k-jetson-agx-thor | NVIDIA Jetson AGX Thor Developer Kit, aarch64, 14 CPUs, 123 GiB, CT4000T710SSD8 nvme | 10,000 | 8 | 383 | 4.0 ms | 50.4 ms | 53.2 ms | 0 | 20,281 | 131 MiB | `42faa9d` |

The result files:
[pilot-10k-jetson-agx-thor.json](https://vcon-server.com/scale/pilot-10k-jetson-agx-thor.json)
(2026-11-20, 10,000 reads, 1-minute load 20.1 at the start).

Each row is one run of one server, built in release mode from the commit
in the last column, on the machine the row names:

- **Stores a second:** the vCons stored over the run's wall time, with
  that many clients sending at once over HTTPS.
- **Read p50, p99 and p99.9:** the time from sending a `GET /vcons/{uuid}`
  to the last byte of its answer. Half of the reads name a vCon picked
  at random from all stored, half one of the newest hundredth.
- **Bytes per vCon:** the server's data directory after the stores, over
  the vCons stored. It holds the sealed bodies and the catalogs.
- **Peak server memory:** the server's peak resident memory (`VmHWM`).

The vCons follow a size mix of 70% at 2 KiB, 25% at 16 KiB and 5% at
256 KiB, each a text dialog between two parties with example.com
addresses and 555-01xx numbers. The data directory is on local disk, with
the catalogs in SQLite. The harness and the server share the machine.

The machine of the first row is a shared development host that runs
other builds and tests at the same time, so its numbers vary from run to
run. The harness refuses to start there while the 1-minute load is above
28.

## Before you run the harness

- A Linux machine with `openssl`, `jq`, `lsblk` and `findmnt`, and the
  toolchain of `rust-toolchain.toml`.
- Free disk for the vCons: the count times the bytes per vCon of a row
  above, for the same mix. In the first row's run, 10,000 vCons at the
  default mix took 202,802,376 bytes.
- A memory lock limit large enough for the server, which locks all of its
  memory and refuses to start if it cannot (`ulimit -l`; see
  [getting started](@/docs/getting-started.md#allow-it-to-lock-its-memory)).

## Run it

Build the server and the harness in release mode, and run the driver
script with the two binaries, the vCon count and the result file:

```sh
cargo build --locked --release -p vcon-server -p vcon-scale
SCALE_LABEL=my-run scripts/scale-run.sh target/release/vcon-server \
  target/release/vcon-scale 10000 my-run.json
```

The script makes a data directory, a master key, a CA and a server
certificate, a tenant and a key in a new directory under `$TMPDIR` (or
under a fifth argument, the parent directory), starts `vcon-server
serve`, and runs `vcon-scale run`. It deletes that directory, the stored
vCons with it, when the run ends, whatever the outcome.

Flags after `--` go to `vcon-scale run`:

| Flag | Default | What it sets |
|---|---|---|
| `--concurrency` | 8 | the clients that send at once (1 to 256) |
| `--reads` | 1000 | the reads after the stores |
| `--list-pages` | 10 | the list pages of 100 to follow |
| `--seed` | 1 | the generator's seed: the same seed makes the same vCons |
| `--mix` | `70:2048,25:16384,5:262144` | the size mix, as `weight:bytes` parts |
| `--max-load` | 28 | the highest 1-minute load at which a run starts |

## Check the result

The harness prints one line, the count, the rate, the read p99 and the
result file:

```text
vcon-scale: <count> stored at <rate> a second; reads p99 <microseconds> us; result in <file>
```

It exits 0 when no request failed. The result file holds each phase's
requests, errors, wall time, rate and latency percentiles in
microseconds, the data directory's bytes, the server's peak memory and
CPU time, and the machine: its model, CPU architecture and count, memory,
kernel and disk. To add a run to this page, copy the file to
`website/static/scale/<label>.json` and paste the row that the test
`the_page_table_is_the_committed_results` prints into the table above.

## When a run fails

- **"the 1-minute load is ..., above the cap of 28: not started":** the
  machine is busy. Wait, or name a cap that suits the machine with
  `--max-load`.
- **"the size mix part ... is not weight:bytes":** each part of `--mix` is
  two whole numbers with a colon between them.
- **Exit 1 and "ingest: N of M failed":** the server refused or dropped
  requests. The line "first error" names the first one, and the script
  prints the server's last 20 log lines.
- **"the server stopped":** the server did not start. The script prints
  its log, which names the reason, for example a memory lock limit too
  small for the server.
