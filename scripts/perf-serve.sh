#!/usr/bin/env bash
# The server wrapper of the performance gate: scripts/scale-run.sh runs it
# in place of the server when SCALE_SERVE_WRAP names it, and it runs the
# server under valgrind (`perf_gate.py serve`; PERF_GATE_TOOL is cachegrind
# or dhat, PERF_GATE_OUT the directory of its files).
set -euo pipefail
exec python3 -I "$(dirname "$0")/perf_gate.py" serve -- "$@"
