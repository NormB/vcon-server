"""Holds the number of Rust files CodeQL extracts with errors to the number
recorded here.

A file the CodeQL Rust extractor cannot fully read is analyzed in part, and
a finding in the part it skipped is never reported. The analysis passes all
the same: on 2026-11-25 (push run 45612538665 on 95d3f58) and on every run
after it, the Rust analysis printed "Total number of Rust files that were
extracted with errors | 1": crates/vcon-server/src/api/listen.rs:186:25,
"macro expansion failed for '$crate::select_priv_declare_output_enum'" (a
tokio `select!`). No step read that number.

The rule: after the Rust Analyze step, CI's CodeQL job decodes the summary
query's result in the database the step made
(`results/**/NumberOfFilesExtractedWithErrors.bqrs`, the metric
rust/summary/number-of-files-extracted-with-errors) and fails unless the
number is RECORDED: a rise is a file CodeQL no longer reads whole; a fall
lowers RECORDED in the same change, so the record stays exact. It prints
each extraction error the database holds (ExtractionErrors.bqrs) on a
failure. `log` applies the same rule to the metric table line of a job's
log that has ended (`gh api .../actions/jobs/<id>/logs`).

Run with: python3 -I scripts/codeql_extraction.py check --codeql <codeql> --database <database directory>
          python3 -I scripts/codeql_extraction.py log <job log file>
"""

import json
import pathlib
import re
import subprocess
import sys

sys.dont_write_bytecode = True

# The number of Rust files the pinned CodeQL extracts with errors, and
# where each one is. 0 since the select! at api/listen.rs:186 moved into a
# function of its own (fix/defects-2026-11-26).
RECORDED = 0
COUNT_FILE = "NumberOfFilesExtractedWithErrors.bqrs"
ERRORS_FILE = "ExtractionErrors.bqrs"
DECODE_DEADLINE = 120
USAGE = (
    "usage: codeql_extraction.py check --codeql <codeql> --database <database directory>"
    " | log <job log file>"
)
# The Analyze step's metric table line, after the log's timestamp.
LOG_LINE = re.compile(r"^(?:\S+ )?\|\s*Total number of Rust files that were extracted with errors\s*\|\s*(\S+)\s*\|\s*$", re.MULTILINE)


def find(database, name):
    """The one `name` under the database's results, or a ValueError."""
    found = sorted(pathlib.Path(database).joinpath("results").rglob(name))
    if not found:
        raise ValueError(f"{database}/results holds no {name}: the analysis did not run the summary queries")
    if len(found) > 1:
        raise ValueError(f"{database}/results holds {len(found)} files named {name}: {', '.join(map(str, found))}")
    return found[0]


def decode(codeql, path, run=subprocess.run):
    """The JSON `codeql bqrs decode` writes for `path`, parsed."""
    done = run(
        [str(codeql), "bqrs", "decode", "--format=json", str(path)],
        capture_output=True, text=True, timeout=DECODE_DEADLINE, check=False, stdin=subprocess.DEVNULL,
    )
    if done.returncode != 0:
        raise ValueError(f"codeql bqrs decode {path} failed (exit {done.returncode}): {done.stderr.strip()[-500:]}")
    try:
        return json.loads(done.stdout)
    except json.JSONDecodeError as error:
        raise ValueError(f"codeql bqrs decode {path} wrote no JSON: {error}") from error


def count(decoded):
    """The one integer of a metric query's result set."""
    tuples = (decoded.get("#select") or {}).get("tuples") if isinstance(decoded, dict) else None
    if not isinstance(tuples, list) or len(tuples) != 1 or not isinstance(tuples[0], list) or len(tuples[0]) != 1:
        raise ValueError(f"the summary query's result is not one number: {json.dumps(decoded)[:300]}")
    value = tuples[0][0]
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise ValueError(f"the summary query's result is not a count: {value!r}")
    return value


def error_lines(decoded):
    """One line per row of the extraction errors' result set."""
    tuples = (decoded.get("#select") or {}).get("tuples") if isinstance(decoded, dict) else None
    if not isinstance(tuples, list):
        return [f"(the extraction errors' result is not a result set: {json.dumps(decoded)[:300]})"]
    return [" | ".join(json.dumps(cell)[:300] if not isinstance(cell, str) else cell for cell in row) for row in tuples]


def count_in_log(text):
    """The number in the Analyze step's metric table line for the files
    extracted with errors, from a job's log text."""
    found = [m.group(1) for m in LOG_LINE.finditer(text)]
    if not found:
        raise ValueError("the log has no line `| Total number of Rust files that were extracted with errors | N |`")
    if len(found) > 1:
        raise ValueError(f"the log has {len(found)} lines of the files extracted with errors")
    if not found[0].isdigit():
        raise ValueError(f"the count of files extracted with errors is no number: {found[0]!r}")
    return int(found[0])


def judge(found, recorded=RECORDED):
    """None when `found` is the record, else the failure's text."""
    if found > recorded:
        return (
            f"{found} Rust file(s) were extracted with errors, more than the {recorded} recorded in "
            "scripts/codeql_extraction.py: CodeQL analyzes part of each, and reports no finding in the rest"
        )
    if found < recorded:
        return f"{found} Rust file(s) were extracted with errors, fewer than the {recorded} recorded: set RECORDED to {found}"
    return None


def check(codeql, database, run=subprocess.run):
    """(exit status, lines to print)."""
    try:
        found = count(decode(codeql, find(database, COUNT_FILE), run))
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        return 1, [f"codeql_extraction: {error}"]
    failure = judge(found)
    if failure is None:
        return 0, [f"codeql_extraction: ok ({found} Rust file(s) extracted with errors, as recorded)"]
    lines = [f"codeql_extraction: {failure}"]
    try:
        lines += [f"codeql_extraction: {line}" for line in error_lines(decode(codeql, find(database, ERRORS_FILE), run))]
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        lines.append(f"codeql_extraction: the errors cannot be listed: {error}")
    return 1, lines


def parse(argv):
    """(codeql, database) from the command line, or None."""
    if len(argv) != 5 or argv[0] != "check":
        return None
    options = {argv[i]: argv[i + 1] for i in range(1, len(argv), 2)}
    if set(options) != {"--codeql", "--database"} or not all(options.values()):
        return None
    return options["--codeql"], options["--database"]


def check_log(path):
    """(exit status, lines to print) for a job log that has ended."""
    try:
        found = count_in_log(pathlib.Path(path).read_text(encoding="utf-8", errors="replace"))
    except (OSError, ValueError) as error:
        return 1, [f"codeql_extraction: {error}"]
    failure = judge(found)
    if failure is None:
        return 0, [f"codeql_extraction: ok ({found} Rust file(s) extracted with errors, as recorded)"]
    return 1, [f"codeql_extraction: {failure}"]


def main(argv, run=subprocess.run, out=sys.stdout, err=sys.stderr):
    if len(argv) == 2 and argv[0] == "log":
        code, lines = check_log(argv[1])
        for line in lines:
            print(line, file=err if code else out)
        return code
    parsed = parse(argv)
    if parsed is None:
        print(USAGE, file=err)
        return 2
    code, lines = check(*parsed, run=run)
    for line in lines:
        print(line, file=err if code else out)
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
