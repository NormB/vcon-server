#!/usr/bin/env python3
"""Classify the changes between two versions of vcon-server's HTTP contract.

website/content/docs/stability.md states what vcon-server keeps compatible.
This script compares an OLD and a NEW version of one surface and says which
changes break a client written against OLD:

    contract_diff.py --kind openapi  OLD.json NEW.json  # OpenAPI documents
    contract_diff.py --kind problems OLD_DIR  NEW_DIR   # problem-type pages
    contract_diff.py --against REF                      # this tree against REF

`--against` reads each surface at REF (`git show`, `git ls-tree`) and in
the work tree: crates/vcon-server/openapi.json and the pages of
website/content/problems/. A surface REF does not hold is reported as "no
baseline" and passes.

THE RULE. A schema is read from the side of the program that handles the
value:

- reader rule, for what the server sends (response bodies): breaking when a
  property disappears or stops being required; when a value can take a type
  the old schema did not allow (`null` included; no `type` allows every
  type); when a closed enum gains a value or loses its list; when a `const`
  changes. A new property is additive.
- writer rule, for what a client sends (request bodies, parameters):
  breaking when the server stops accepting a value it accepted: a type
  dropped, an enum value removed, an enum or a `const` where there was none,
  a bound tightened, a pattern or format changed, unknown members newly
  refused, a property removed or newly required. A changed or removed
  `default` is breaking too (a client that leaves the value out gets other
  behavior). A new optional property, a new enum value and a wider bound
  are additive.
- the document: breaking when an operation, a response status, a response
  media type or header, a parameter or a request media type disappears;
  when a parameter or the request body becomes required; when a security
  requirement an old client met is no longer accepted. New operations,
  statuses, optional parameters and headers are additive. `info` (whose
  `version` changes every release), descriptions and examples are not
  compared.
- problem types: a removed page of website/content/problems/ is breaking;
  a new one is additive.

A `oneOf`, `anyOf`, `allOf` or `not` that changes is reported as breaking,
because this script does not compare them, and a part it cannot compare is
never passed as unchanged. An enum that may grow says so with
`"x-vcon-server-open-enum": true` in both versions. Only local references
(`#/...`) are followed; any other is an error.

Prints one line per change, `breaking <where>: <what>` or `additive <where>:
<what>`, then a count (with `--against`, each line starts with the surface).
Exit 0: no breaking change. 1: at least one. 2: usage, read, git or
reference error; nothing was decided.
"""

from __future__ import annotations

import json
import os
import pathlib
import re
import subprocess
import sys
from dataclasses import dataclass

METHODS = ("get", "put", "post", "delete", "options", "head", "patch", "trace")
ALL_TYPES = frozenset({"string", "number", "integer", "boolean", "object", "array", "null"})
OPEN_ENUM = "x-vcon-server-open-enum"
COMPOSITION = ("allOf", "anyOf", "oneOf", "not")
# Bounds a writer meets: a larger lower bound or a smaller upper bound
# refuses a value the old schema accepted.
LOWER = ("minimum", "exclusiveMinimum", "minLength", "minItems", "minProperties")
UPPER = ("maximum", "exclusiveMaximum", "maxLength", "maxItems", "maxProperties")
OPENAPI_PATH = "crates/vcon-server/openapi.json"
PROBLEMS_PATH = "website/content/problems"
UNSET = object()


class ContractError(Exception):
    """A snapshot this script cannot read as a contract."""


@dataclass(frozen=True)
class Change:
    kind: str  # "breaking" or "additive"
    path: str
    message: str


def _resolve(node, root):
    """`node` with every top-level local `$ref` followed, and the refs followed."""
    refs = []
    while isinstance(node, dict) and "$ref" in node:
        ref = node["$ref"]
        if not isinstance(ref, str) or not ref.startswith("#"):
            raise ContractError(f"reference {ref!r} is not local to the document")
        if ref in refs:
            return node, tuple(refs)
        refs.append(ref)
        target = root
        for part in ref[1:].split("/")[1:]:
            part = part.replace("~1", "/").replace("~0", "~")
            if not isinstance(target, dict) or part not in target:
                raise ContractError(f"reference {ref!r} does not resolve")
            target = target[part]
        node = target
    return node, tuple(refs)


def _value_type(v) -> str:
    if v is None:
        return "null"
    if isinstance(v, bool):
        return "boolean"
    if isinstance(v, int):
        return "integer"
    if isinstance(v, float):
        return "number"
    if isinstance(v, str):
        return "string"
    return "array" if isinstance(v, list) else "object"


def _types(s) -> frozenset:
    """The JSON types a value of schema `s` can have."""
    if s is True or not isinstance(s, dict):
        return ALL_TYPES
    t = s.get("type")
    if t is None:
        # Without `type`, a value list still limits the types to its values'.
        if "const" in s:
            return frozenset({_value_type(s["const"])})
        if "enum" in s:
            return frozenset(_value_type(v) for v in s["enum"])
        return ALL_TYPES
    ts = frozenset([t] if isinstance(t, str) else t)
    # Every integer is a number.
    return ts | ({"integer"} if "number" in ts else set())


def _canon(v) -> str:
    return json.dumps(v, sort_keys=True)


def _reader(o: dict, n: dict, path: str, out: list) -> None:
    """The checks of the reader rule on one node (not its children)."""
    added = sorted(_types(n) - _types(o))
    if added:
        out.append(Change("breaking", path, f"may now be of type {', '.join(added)}"))
    if "const" in o and n.get("const", UNSET) != o["const"]:
        out.append(Change("breaking", path, f"const {o['const']!r} changed"))
    if "enum" in o:
        if "enum" not in n:
            out.append(Change("breaking", path, "enum list removed"))
        else:
            grown = [v for v in n["enum"] if v not in o["enum"]]
            if grown and not (o.get(OPEN_ENUM) is True and n.get(OPEN_ENUM) is True):
                out.append(Change("breaking", path, f"closed enum gained {grown!r}"))


def _writer(o: dict, n: dict, path: str, out: list) -> None:
    """The checks of the writer rule on one node (not its children)."""
    dropped = sorted(_types(o) - _types(n))
    if dropped:
        out.append(Change("breaking", path, f"no longer accepts {', '.join(dropped)}"))
    if "const" in n and o.get("const", UNSET) != n["const"]:
        out.append(Change("breaking", path, f"now accepts only {n['const']!r}"))
    if "enum" in n:
        if "enum" not in o:
            out.append(Change("breaking", path, f"now accepts only {n['enum']!r}"))
        else:
            lost = [v for v in o["enum"] if v not in n["enum"]]
            grown = [v for v in n["enum"] if v not in o["enum"]]
            if lost:
                out.append(Change("breaking", path, f"no longer accepts {lost!r}"))
            if grown:
                out.append(Change("additive", path, f"accepts more values: {grown!r}"))
    for kw in LOWER:
        if kw in n and (kw not in o or n[kw] > o[kw]):
            was = f" from {o[kw]!r}" if kw in o else ""
            out.append(Change("breaking", path, f"{kw} raised{was} to {n[kw]!r}"))
    for kw in UPPER:
        if kw in n and (kw not in o or n[kw] < o[kw]):
            was = f" from {o[kw]!r}" if kw in o else ""
            out.append(Change("breaking", path, f"{kw} lowered{was} to {n[kw]!r}"))
    for kw in ("pattern", "format"):
        if kw in n and o.get(kw, UNSET) != n[kw]:
            out.append(Change("breaking", path, f"{kw} changed"))
    if n.get("uniqueItems") is True and o.get("uniqueItems") is not True:
        out.append(Change("breaking", path, "items must now be unique"))
    if n.get("additionalProperties") is False and o.get("additionalProperties", True) is not False:
        out.append(Change("breaking", path, "unknown members are now refused"))
    if "default" in o:
        if "default" not in n:
            out.append(Change("breaking", path, f"default {o['default']!r} removed"))
        elif n["default"] != o["default"]:
            out.append(Change("breaking", path, f"default changed from {o['default']!r} to {n['default']!r}"))
    elif "default" in n:
        out.append(Change("additive", path, f"default stated: {n['default']!r}"))


def schema_changes(old, new, old_root, new_root, path="$", side="reader", _seen=frozenset()) -> list[Change]:
    """Every change from `old` to `new`, read by `side`: "reader" for a value
    the server sends, "writer" for a value a client sends."""
    o, o_refs = _resolve(old, old_root)
    n, n_refs = _resolve(new, new_root)
    seen = _seen
    if o_refs or n_refs:
        # `seen` holds the reference pairs of this node's ancestors only, so a
        # pair met again is a cycle and ends here, while two sibling
        # properties that use one definition are both compared.
        pair = (o_refs[-1:], n_refs[-1:])
        if pair in seen:
            return []
        seen = seen | {pair}
    out: list[Change] = []
    if not (isinstance(o, dict) and isinstance(n, dict)):
        if _types(n) != _types(o):
            (_reader if side == "reader" else _writer)(o if isinstance(o, dict) else {},
                                                      n if isinstance(n, dict) else {}, path, out)
        return out
    (_reader if side == "reader" else _writer)(o, n, path, out)
    for kw in COMPOSITION:
        if _canon(o.get(kw)) != _canon(n.get(kw)):
            out.append(Change("breaking", path, f"{kw} changed, and it is not compared"))

    o_props = o.get("properties") or {}
    n_props = n.get("properties") or {}
    o_req = set(o.get("required") or [])
    n_req = set(n.get("required") or [])
    for name in sorted(set(o_props) | set(n_props)):
        sub = f"{path}.{name}"
        if name not in n_props:
            out.append(Change("breaking", sub, "property removed"))
        elif name not in o_props:
            if side == "writer" and name in n_req:
                out.append(Change("breaking", sub, "required property added"))
            else:
                out.append(Change("additive", sub, "optional property added" if side == "writer" else "property added"))
        else:
            if side == "reader" and name in o_req and name not in n_req:
                out.append(Change("breaking", sub, "no longer required, so it may be absent"))
            if side == "writer" and name in n_req and name not in o_req:
                out.append(Change("breaking", sub, "became required"))
            out.extend(schema_changes(o_props[name], n_props[name], old_root, new_root, sub, side, seen))

    if "items" in o and "items" in n:
        out.extend(schema_changes(o["items"], n["items"], old_root, new_root, f"{path}[]", side, seen))
    o_add, n_add = o.get("additionalProperties"), n.get("additionalProperties")
    if isinstance(o_add, dict) and isinstance(n_add, dict):
        out.extend(schema_changes(o_add, n_add, old_root, new_root, f"{path}.*", side, seen))
    return out


def _template(path: str) -> tuple[str, list[str]]:
    """The path with each `{name}` as `{}`, and the names in order."""
    names = re.findall(r"\{([^}]*)\}", path)
    return re.sub(r"\{[^}]*\}", "{}", path), names


def _params(item: dict, operation: dict, path: str, root: dict) -> dict:
    """The operation's parameters, path-level ones included (an operation's
    own replaces one of the path's with the same key), keyed (in, name).
    A header name has no case; a path parameter is keyed by its place in
    the template, so a renamed template variable is the same parameter."""
    _, names = _template(path)
    out = {}
    for p in list(item.get("parameters") or []) + list(operation.get("parameters") or []):
        p, _ = _resolve(p, root)
        where, name = p.get("in"), str(p.get("name"))
        if where == "header":
            name = name.lower()
        key = (where, f"#{names.index(name)}" if where == "path" and name in names else name)
        out[key] = (name, where == "path" or bool(p.get("required")), p.get("schema"))
    return out


def _alternatives(requirements) -> dict:
    """Each security alternative, keyed by a canonical form, with its name."""
    reqs = requirements if requirements else [{}]
    return {_canon(r): (" + ".join(sorted(r)) or "none") for r in reqs}


def _media(container: dict) -> dict:
    return (container or {}).get("content") or {}


def _headers(response: dict) -> dict:
    return {str(k).lower(): v for k, v in (response.get("headers") or {}).items()}


def _body_changes(o_body, n_body, old, new, where, out):
    o_body = _resolve(o_body, old)[0] if o_body is not None else None
    n_body = _resolve(n_body, new)[0] if n_body is not None else None
    bwhere = f"{where} body"
    if o_body is None and n_body is None:
        return
    if n_body is None:
        out.append(Change("breaking", bwhere, "request body removed"))
        return
    if o_body is None:
        if n_body.get("required"):
            out.append(Change("breaking", bwhere, "required request body added"))
        else:
            out.append(Change("additive", bwhere, "optional request body added"))
        return
    if n_body.get("required") and not o_body.get("required"):
        out.append(Change("breaking", bwhere, "request body became required"))
    o_m, n_m = _media(o_body), _media(n_body)
    for media in sorted(set(o_m) | set(n_m)):
        mwhere = f"{bwhere} {media}"
        if media not in n_m:
            out.append(Change("breaking", mwhere, "media type removed"))
        elif media not in o_m:
            out.append(Change("additive", mwhere, "media type added"))
        elif "schema" in o_m[media] and "schema" in n_m[media]:
            out.extend(schema_changes(o_m[media]["schema"], n_m[media]["schema"], old, new,
                                      f"{mwhere} $", "writer"))


def _response_changes(o_res, n_res, old, new, where, out):
    o_res, n_res = _resolve(o_res, old)[0], _resolve(n_res, new)[0]
    o_m, n_m = _media(o_res), _media(n_res)
    for media in sorted(set(o_m) | set(n_m)):
        mwhere = f"{where} {media}"
        if media not in n_m:
            out.append(Change("breaking", mwhere, "media type removed"))
        elif media not in o_m:
            out.append(Change("additive", mwhere, "media type added"))
        elif "schema" in o_m[media] and "schema" in n_m[media]:
            out.extend(schema_changes(o_m[media]["schema"], n_m[media]["schema"], old, new,
                                      f"{mwhere} $", "reader"))
        elif "schema" in o_m[media]:
            out.append(Change("breaking", mwhere, "body schema removed"))
    o_h, n_h = _headers(o_res), _headers(n_res)
    for name in sorted(set(o_h) | set(n_h)):
        hwhere = f"{where} header {name}"
        if name not in n_h:
            out.append(Change("breaking", hwhere, "header removed"))
        elif name not in o_h:
            out.append(Change("additive", hwhere, "header added"))
        else:
            o_s = _resolve(o_h[name], old)[0].get("schema")
            n_s = _resolve(n_h[name], new)[0].get("schema")
            if o_s is not None and n_s is not None:
                out.extend(schema_changes(o_s, n_s, old, new, f"{hwhere} $", "reader"))


def openapi_changes(old: dict, new: dict) -> list[Change]:
    """Every change from `old` to `new`, read as the API a client calls."""
    for d in (old, new):
        if not isinstance(d, dict) or not isinstance(d.get("paths"), dict):
            raise ContractError("an OpenAPI document has no `paths` object")
    out: list[Change] = []

    def ops(d):
        found = {}
        for p, item in d["paths"].items():
            item = _resolve(item, d)[0]
            for m in METHODS:
                if m in item:
                    found[(m.upper(), _template(p)[0])] = (p, item, item[m])
        return found

    o_ops, n_ops = ops(old), ops(new)
    for key in sorted(set(o_ops) | set(n_ops), key=lambda k: (k[1], k[0])):
        shown = (n_ops.get(key) or o_ops[key])[0]
        where = f"{key[0]} {shown}"
        if key not in n_ops:
            out.append(Change("breaking", where, "operation removed"))
            continue
        if key not in o_ops:
            out.append(Change("additive", where, "operation added"))
            continue
        (o_path, o_item, o_op), (n_path, n_item, n_op) = o_ops[key], n_ops[key]

        o_sec = _alternatives(o_op["security"] if "security" in o_op else old.get("security"))
        n_sec = _alternatives(n_op["security"] if "security" in n_op else new.get("security"))
        for k in sorted(set(o_sec) - set(n_sec)):
            out.append(Change("breaking", where, f"security requirement {o_sec[k]} no longer accepted"))
        for k in sorted(set(n_sec) - set(o_sec)):
            out.append(Change("additive", where, f"security requirement {n_sec[k]} accepted"))

        o_par, n_par = _params(o_item, o_op, o_path, old), _params(n_item, n_op, n_path, new)
        for pk in sorted(set(o_par) | set(n_par), key=lambda k: (str(k[0]), str(k[1]))):
            name, required, schema = n_par.get(pk) or o_par[pk]
            pwhere = f"{where} {pk[0]} {name}"
            if pk not in n_par:
                out.append(Change("breaking", pwhere, "parameter removed"))
            elif pk not in o_par:
                if required:
                    out.append(Change("breaking", pwhere, "required parameter added"))
                else:
                    out.append(Change("additive", pwhere, "optional parameter added"))
            else:
                if required and not o_par[pk][1]:
                    out.append(Change("breaking", pwhere, "parameter became required"))
                o_s = o_par[pk][2]
                if o_s is not None and schema is not None:
                    out.extend(schema_changes(o_s, schema, old, new, f"{pwhere} $", "writer"))

        _body_changes(o_op.get("requestBody"), n_op.get("requestBody"), old, new, where, out)

        o_res, n_res = o_op.get("responses") or {}, n_op.get("responses") or {}
        for status in sorted(set(o_res) | set(n_res)):
            swhere = f"{where} {status}"
            if status not in n_res:
                out.append(Change("breaking", swhere, "response removed"))
            elif status not in o_res:
                out.append(Change("additive", swhere, "response added"))
            else:
                _response_changes(o_res[status], n_res[status], old, new, swhere, out)
    return out


def problem_slugs(names) -> set:
    """The problem types a directory listing of website/content/problems/
    holds: each `<slug>.md` but the section's `_index.md`."""
    return {n[:-3] for n in names if n.endswith(".md") and not n.startswith("_")}


def problem_changes(old: set, new: set) -> list[Change]:
    out = [Change("breaking", f"problems/{s}", "problem type removed") for s in sorted(set(old) - set(new))]
    out += [Change("additive", f"problems/{s}", "problem type added") for s in sorted(set(new) - set(old))]
    return out


def git_runner(root, env=None):
    """A function that runs `git <args>` in `root`: (exit code, stdout)."""
    def run(args):
        r = subprocess.run(["git", *args], cwd=root, env=env, capture_output=True, text=True, timeout=120)
        return r.returncode, r.stdout
    return run


def _report(label: str, changes: list[Change]) -> int:
    prefix = f"{label}: " if label else ""
    for c in changes:
        print(f"{prefix}{c.kind} {c.path}: {c.message}")
    n = sum(c.kind == "breaking" for c in changes)
    print(f"{prefix or 'contract_diff: '}{len(changes)} change(s), {n} breaking")
    return n


def against(ref: str, root, git) -> int:
    """Compares the work tree at `root` with `ref`, each surface in turn,
    reading `ref` through `git(args) -> (code, stdout)`. Returns the exit
    status: 0, 1 or 2."""
    root = pathlib.Path(root)
    rc, _ = git(["rev-parse", "--verify", f"{ref}^{{commit}}"])
    if rc != 0:
        print(f"contract_diff: {ref!r} names no commit; nothing was decided", file=sys.stderr)
        return 2
    broken = 0
    try:
        rc, text = git(["show", f"{ref}:{OPENAPI_PATH}"])
        if rc != 0:
            print(f"openapi: no baseline at {ref}")
        else:
            new = json.loads((root / OPENAPI_PATH).read_text(encoding="utf-8"))
            broken += _report("openapi", openapi_changes(json.loads(text), new))
        rc, text = git(["ls-tree", "--name-only", ref, "--", PROBLEMS_PATH + "/"])
        old_names = [line.rsplit("/", 1)[-1] for line in text.splitlines() if line]
        if rc != 0 or not old_names:
            print(f"problems: no baseline at {ref}")
        else:
            new_names = os.listdir(root / PROBLEMS_PATH)
            broken += _report("problems", problem_changes(problem_slugs(old_names), problem_slugs(new_names)))
    except (OSError, ValueError, ContractError) as e:
        print(f"contract_diff: {e}; nothing was decided", file=sys.stderr)
        return 2
    return 1 if broken else 0


USAGE = ("usage: contract_diff.py --kind openapi OLD.json NEW.json\n"
         "       contract_diff.py --kind problems OLD_DIR NEW_DIR\n"
         "       contract_diff.py --against REF")


KINDS = ("openapi", "problems")


def parse(argv: list[str]):
    """("against", REF) or (KIND, OLD, NEW) for a usage main takes, else None."""
    if len(argv) == 2 and argv[0] == "--against":
        return ("against", argv[1])
    if len(argv) == 4 and argv[0] == "--kind" and argv[1] in KINDS:
        return tuple(argv[1:])
    return None


def main(argv: list[str]) -> int:
    args = parse(argv)
    if args is None:
        print(USAGE, file=sys.stderr)
        return 2
    if args[0] == "against":
        root = pathlib.Path(__file__).resolve().parent.parent
        return against(args[1], root, git_runner(root))
    try:
        if args[0] == "openapi":
            docs = []
            for path in argv[2:]:
                with open(path, encoding="utf-8") as f:
                    docs.append(json.load(f))
            changes = openapi_changes(*docs)
        else:
            changes = problem_changes(problem_slugs(os.listdir(argv[2])), problem_slugs(os.listdir(argv[3])))
    except (OSError, ValueError, ContractError) as e:
        print(f"contract_diff: {e}; nothing was decided", file=sys.stderr)
        return 2
    return 1 if _report("", changes) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
