"""Tests scripts/contract_diff.py: the classifier that tells a breaking
change to the HTTP API from an additive one.

Run with: python3 -I scripts/test_contract_diff.py
"""

import contextlib
import copy
import io
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE))
import importlib.util  # noqa: E402

spec = importlib.util.spec_from_file_location("contract_diff", HERE / "contract_diff.py")
cd = importlib.util.module_from_spec(spec)
# A dataclass with postponed annotations finds its module in sys.modules.
sys.modules["contract_diff"] = cd
spec.loader.exec_module(cd)

OPENAPI = ROOT / "crates/vcon-server/openapi.json"
PROBLEMS = ROOT / "website/content/problems"
STABILITY = ROOT / "website/content/docs/stability.md"


def obj(props, required=(), **more):
    """An object schema with these properties."""
    out = {"type": "object", "properties": props}
    if required:
        out["required"] = list(required)
    out.update(more)
    return out


def doc(paths, schemas=None, parameters=None, security=None, **more):
    """An OpenAPI document with these paths and components."""
    d = {"openapi": "3.1.1", "info": {"title": "t", "version": "1.0.0"}, "paths": paths}
    comps = {}
    if schemas is not None:
        comps["schemas"] = schemas
    if parameters is not None:
        comps["parameters"] = parameters
    comps.update(more.pop("components", {}))
    if comps:
        d["components"] = comps
    if security is not None:
        d["security"] = security
    d.update(more)
    return d


def resp(schema=None, media="application/json", headers=None):
    """A response with one media type (none when `schema` is None and
    `media` is None)."""
    out = {"description": "d"}
    if media is not None:
        out["content"] = {media: {} if schema is None else {"schema": schema}}
    if headers:
        out["headers"] = {h: {"schema": {"type": "string"}} for h in headers}
    return out


def op(responses=None, parameters=None, body=None, **more):
    out = {"responses": responses if responses is not None else {"200": resp({"type": "object"})}}
    if parameters is not None:
        out["parameters"] = parameters
    if body is not None:
        out["requestBody"] = body
    out.update(more)
    return out


def body(schema, required=True, media="application/json"):
    return {"required": required, "content": {media: {"schema": schema}}}


def query(name, schema=None, required=False):
    return {"name": name, "in": "query", "required": required, "schema": schema or {"type": "string"}}


def kinds(changes):
    return [(c.kind, c.path, c.message) for c in changes]


def breaking(changes):
    return [c for c in changes if c.kind == "breaking"]


def additive(changes):
    return [c for c in changes if c.kind == "additive"]


def post_body(schema):
    """A one-operation document whose POST /x takes `schema` as its body."""
    return doc({"/x": {"post": op(body=body(schema))}})


def get_returning(schema):
    """A one-operation document whose GET /x answers `schema`."""
    return doc({"/x": {"get": op({"200": resp(schema)})}})


def run_main(argv, files=None):
    """main's exit status, standard output and standard error. `files` maps
    a placeholder in argv to JSON (a dict) or a list of file names (a
    directory of empty files)."""
    out, err = io.StringIO(), io.StringIO()
    with tempfile.TemporaryDirectory() as tmp:
        args = []
        for a in argv:
            if files and a in files:
                value = files[a]
                path = pathlib.Path(tmp, a)
                if isinstance(value, list):
                    path.mkdir()
                    for name in value:
                        (path / name).write_text("x\n", encoding="utf-8")
                elif isinstance(value, str):
                    path.write_text(value, encoding="utf-8")
                else:
                    path.write_text(json.dumps(value), encoding="utf-8")
                a = str(path)
            args.append(a)
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = cd.main(args)
    return rc, out.getvalue(), err.getvalue()


def clean_git_env():
    """The environment without the variables a hook sets for its own
    repository, so a git command reaches the repository it is run in."""
    return {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}


def git(repo, *args):
    return subprocess.run(
        ["git", "-c", "commit.gpgsign=false", "-c", "user.name=t", "-c", "user.email=t@example.com",
         "-c", "init.defaultBranch=main", *args],
        cwd=repo, env=clean_git_env(), capture_output=True, text=True, check=True, timeout=60)


class FakeGit:
    """A git reader that answers from a dict: (ref, path) -> text for a file,
    (ref, directory) -> list of names for a directory."""

    def __init__(self, files, refs=("v1",)):
        self.files, self.refs = files, set(refs)

    def __call__(self, args):
        if args[:2] == ["rev-parse", "--verify"]:
            ref = args[2].split("^")[0]
            return (0, ref + "\n") if ref in self.refs else (128, "")
        if args[0] == "show":
            ref, path = args[1].split(":", 1)
            text = self.files.get((ref, path))
            return (0, text) if isinstance(text, str) else (128, "")
        if args[0] == "ls-tree":
            ref, path = args[2], args[4].rstrip("/")
            names = self.files.get((ref, path))
            if not isinstance(names, list):
                return (0, "")
            return (0, "".join(f"{path}/{n}\n" for n in names))
        raise AssertionError(f"unexpected git call {args}")


def tree(tmp, openapi=None, problems=None, cli=None, config=None):
    """A work tree under `tmp` with the two HTTP surfaces and, when given,
    the command line's and the configuration's snapshots."""
    root = pathlib.Path(tmp)
    for name, snapshot in (("cli", cli), ("config", config)):
        if snapshot is not None:
            p = root / f"contract/{name}.json"
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(json.dumps(snapshot), encoding="utf-8")
    if openapi is not None:
        p = root / "crates/vcon-server/openapi.json"
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(json.dumps(openapi), encoding="utf-8")
    if problems is not None:
        d = root / "website/content/problems"
        d.mkdir(parents=True, exist_ok=True)
        for name in problems:
            (d / name).write_text("x\n", encoding="utf-8")
    return root


BASE = doc(
    {
        "/vcons/{uuid}": {
            "parameters": [{"$ref": "#/components/parameters/uuid"}],
            "get": op({"200": resp({"$ref": "#/components/schemas/Vcon"}),
                       "404": resp({"$ref": "#/components/schemas/Problem"}, "application/problem+json")},
                      parameters=[{"$ref": "#/components/parameters/limit"}]),
        },
        "/keys": {"post": op({"201": resp({"type": "object"})},
                             body=body({"$ref": "#/components/schemas/NewKey"}))},
    },
    schemas={
        "Vcon": obj({"uuid": {"type": "string"}, "parties": {"type": "array", "items": {"type": "object"}}},
                    ["uuid"]),
        "Problem": obj({"type": {"type": "string", "enum": ["a", "b"], cd.OPEN_ENUM: True}}, ["type"]),
        "NewKey": obj({"label": {"type": "string"},
                       "scopes": {"type": "array", "items": {"type": "string", "enum": ["read", "write"]}},
                       "days": {"type": "integer", "minimum": 1, "maximum": 365, "default": 90}},
                      ["label"], additionalProperties=False),
    },
    parameters={
        "uuid": {"name": "uuid", "in": "path", "required": True, "schema": {"type": "string"}},
        "limit": {"name": "limit", "in": "query", "schema": {"type": "integer", "default": 100}},
    },
    security=[{"bearer": []}],
)


def changed(edit):
    """A deep copy of BASE after `edit(copy)`."""
    d = copy.deepcopy(BASE)
    edit(d)
    return d


class Passes(unittest.TestCase):
    def test_identical_documents_have_no_change(self):
        self.assertEqual(cd.openapi_changes(BASE, copy.deepcopy(BASE)), [], "a document equals itself")

    def test_the_published_document_is_unchanged_against_itself(self):
        d = json.loads(OPENAPI.read_text(encoding="utf-8"))
        self.assertEqual(cd.openapi_changes(d, copy.deepcopy(d)), [], "openapi.json compared with itself")

    def test_the_published_document_reads_without_an_error(self):
        d = json.loads(OPENAPI.read_text(encoding="utf-8"))
        bare = copy.deepcopy(d)
        bare["paths"] = {}
        found = cd.openapi_changes(d, bare)
        self.assertTrue(found and all(c.message == "operation removed" for c in found),
                        "every operation of openapi.json is read and its removal named")

    def test_info_version_is_ignored(self):
        new = changed(lambda d: d["info"].update(version="9.9.9", title="other", description="x"))
        self.assertEqual(cd.openapi_changes(BASE, new), [], "info changes every release and is not compared")

    def test_a_new_operation_is_additive(self):
        new = changed(lambda d: d["paths"].update({"/health": {"get": op()}}))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)), [("additive", "GET /health", "operation added")],
                         "a new path is additive")

    def test_a_new_method_on_a_path_is_additive(self):
        new = changed(lambda d: d["paths"]["/keys"].update(get=op()))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)), [("additive", "GET /keys", "operation added")],
                         "a new method is additive")

    def test_a_new_optional_query_parameter_is_additive(self):
        new = changed(lambda d: d["paths"]["/keys"]["post"].update(parameters=[query("dry")]))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)),
                         [("additive", "POST /keys query dry", "optional parameter added")],
                         "an optional parameter is additive")

    def test_a_required_parameter_made_optional_is_no_break(self):
        old = changed(lambda d: d["paths"]["/keys"]["post"].update(parameters=[query("dry", required=True)]))
        new = changed(lambda d: d["paths"]["/keys"]["post"].update(parameters=[query("dry")]))
        self.assertEqual(breaking(cd.openapi_changes(old, new)), [], "a client that sends it still may")

    def test_a_new_response_status_is_additive(self):
        new = changed(lambda d: d["paths"]["/keys"]["post"]["responses"].update({"409": resp()}))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)), [("additive", "POST /keys 409", "response added")],
                         "a new status is additive")

    def test_a_new_optional_response_member_is_additive(self):
        new = changed(lambda d: d["components"]["schemas"]["Vcon"]["properties"].update(subject={"type": "string"}))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)),
                         [("additive", "GET /vcons/{uuid} 200 application/json $.subject", "property added")],
                         "a reader ignores a member it does not know")

    def test_a_new_optional_request_member_is_additive(self):
        new = changed(lambda d: d["components"]["schemas"]["NewKey"]["properties"].update(note={"type": "string"}))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)),
                         [("additive", "POST /keys body application/json $.note", "optional property added")],
                         "a writer that does not send it is still accepted")

    def test_a_request_member_made_optional_is_no_break(self):
        new = changed(lambda d: d["components"]["schemas"]["NewKey"].update(required=[]))
        self.assertEqual(breaking(cd.openapi_changes(BASE, new)), [], "a writer that sends it is still accepted")

    def test_a_request_enum_that_gains_a_value_is_additive(self):
        new = changed(lambda d: d["components"]["schemas"]["NewKey"]["properties"]["scopes"]["items"]
                      .update(enum=["read", "write", "audit"]))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)),
                         [("additive", "POST /keys body application/json $.scopes[]", "accepts more values: ['audit']")],
                         "the server accepts more")

    def test_a_request_type_widened_is_no_break(self):
        new = changed(lambda d: d["components"]["schemas"]["NewKey"]["properties"]["label"]
                      .update(type=["string", "null"]))
        self.assertEqual(breaking(cd.openapi_changes(BASE, new)), [], "the server accepts more types")

    def test_a_request_bound_relaxed_is_no_break(self):
        new = changed(lambda d: d["components"]["schemas"]["NewKey"]["properties"]["days"]
                      .update(minimum=0, maximum=3650))
        self.assertEqual(breaking(cd.openapi_changes(BASE, new)), [], "wider bounds accept every old value")

    def test_a_response_type_narrowed_is_no_break(self):
        old = get_returning(obj({"n": {"type": ["integer", "null"]}}))
        new = get_returning(obj({"n": {"type": "integer"}}))
        self.assertEqual(cd.openapi_changes(old, new), [], "a reader of integer-or-null reads an integer")

    def test_an_open_response_enum_may_gain_a_value(self):
        new = changed(lambda d: d["components"]["schemas"]["Problem"]["properties"]["type"]
                      .update(enum=["a", "b", "c"]))
        self.assertEqual(breaking(cd.openapi_changes(BASE, new)), [], "the open-enum marker allows new values")

    def test_a_description_change_is_no_change(self):
        new = changed(lambda d: d["components"]["schemas"]["Vcon"].update(description="new words"))
        self.assertEqual(cd.openapi_changes(BASE, new), [], "prose is not part of the contract")

    def test_a_new_problem_type_is_additive(self):
        self.assertEqual(kinds(cd.problem_changes({"bad-body"}, {"bad-body", "gone"})),
                         [("additive", "problems/gone", "problem type added")], "a new page is additive")

    def test_the_real_problem_pages_are_read_as_slugs(self):
        slugs = cd.problem_slugs(p.name for p in PROBLEMS.iterdir())
        self.assertIn("bad-body", slugs, "bad-body.md is a problem type")
        self.assertNotIn("_index", slugs, "the section index is not a problem type")

    def test_main_exits_0_when_only_additive(self):
        new = changed(lambda d: d["paths"].update({"/health": {"get": op()}}))
        rc, out, _ = run_main(["--kind", "openapi", "OLD", "NEW"], {"OLD": BASE, "NEW": new})
        self.assertEqual(rc, 0, out)
        self.assertIn("additive GET /health: operation added", out, "each change is printed")
        self.assertIn("1 change(s), 0 breaking", out, "the count is printed")

    def test_main_reads_problem_directories(self):
        rc, out, _ = run_main(["--kind", "problems", "OLD", "NEW"],
                              {"OLD": ["a.md", "_index.md"], "NEW": ["a.md", "b.md", "_index.md"]})
        self.assertEqual((rc, "additive problems/b: problem type added" in out), (0, True), out)

    def test_against_a_ref_with_no_change_exits_0(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = tree(tmp, BASE, ["a.md"])
            fake = FakeGit({("v1", cd.OPENAPI_PATH): json.dumps(BASE), ("v1", cd.PROBLEMS_PATH): ["a.md"]})
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                rc = cd.against("v1", root, fake)
        self.assertEqual(rc, 0, out.getvalue())
        self.assertIn("openapi: 0 change(s), 0 breaking", out.getvalue(), "each surface reports its count")
        self.assertIn("problems: 0 change(s), 0 breaking", out.getvalue(), "each surface reports its count")

    def test_the_published_problem_type_is_an_open_list(self):
        d = json.loads(OPENAPI.read_text(encoding="utf-8"))
        self.assertIs(d["components"]["schemas"]["Problem"]["properties"]["type"].get(cd.OPEN_ENUM), True,
                      "stability.md says a client must handle a problem type it does not know")

    def test_every_command_on_the_stability_page_parses(self):
        lines = [l for l in STABILITY.read_text(encoding="utf-8").splitlines() if l.startswith("python3 -I scripts/contract_diff.py")]
        self.assertEqual(len(lines), 4, "the page shows the --against, --kind openapi, --kind cli and --kind config forms")
        for line in lines:
            self.assertIsNotNone(cd.parse(line.split()[3:]), f"the page's command {line!r} is a usage main takes")

    def test_the_page_names_every_kind_the_script_takes(self):
        text = STABILITY.read_text(encoding="utf-8")
        for kind in cd.KINDS:
            self.assertIn(f"--kind {kind}", text, f"stability.md shows --kind {kind}")

    def test_against_a_real_tag_in_a_real_repository(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = tree(tmp, BASE, ["a.md"])
            git(root, "init", "-q")
            git(root, "add", ".")
            git(root, "commit", "-q", "-m", "one")
            git(root, "tag", "v1")
            (root / "website/content/problems/b.md").write_text("x\n", encoding="utf-8")
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                rc = cd.against("v1", root, cd.git_runner(root, clean_git_env()))
        self.assertEqual(rc, 0, out.getvalue())
        self.assertIn("problems: additive problems/b: problem type added", out.getvalue(), out.getvalue())


class Fails(unittest.TestCase):
    def test_a_removed_operation_is_breaking(self):
        new = changed(lambda d: d["paths"]["/keys"].pop("post"))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)), [("breaking", "POST /keys", "operation removed")],
                         "a client that calls it gets 404 or 405")

    def test_a_removed_path_is_breaking(self):
        new = changed(lambda d: d["paths"].pop("/vcons/{uuid}"))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)),
                         [("breaking", "GET /vcons/{uuid}", "operation removed")], "every operation of it goes")

    def test_a_new_required_parameter_is_breaking(self):
        new = changed(lambda d: d["paths"]["/keys"]["post"].update(parameters=[query("dry", required=True)]))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)),
                         [("breaking", "POST /keys query dry", "required parameter added")],
                         "an old client does not send it")

    def test_an_optional_parameter_made_required_is_breaking(self):
        new = changed(lambda d: d["components"]["parameters"]["limit"].update(required=True))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)),
                         [("breaking", "GET /vcons/{uuid} query limit", "parameter became required")],
                         "a parameter behind a $ref is read")

    def test_a_removed_parameter_is_breaking(self):
        new = changed(lambda d: d["paths"]["/vcons/{uuid}"]["get"].pop("parameters"))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)),
                         [("breaking", "GET /vcons/{uuid} query limit", "parameter removed")],
                         "a client that sends it is refused")

    def test_a_changed_query_default_is_breaking(self):
        new = changed(lambda d: d["components"]["parameters"]["limit"]["schema"].update(default=50))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)),
                         [("breaking", "GET /vcons/{uuid} query limit $", "default changed from 100 to 50")],
                         "a changed default is breaking (decision vii)")

    def test_a_narrowed_parameter_type_is_breaking(self):
        new = changed(lambda d: d["components"]["parameters"]["limit"]["schema"].update(type="boolean"))
        found = kinds(cd.openapi_changes(BASE, new))
        self.assertIn(("breaking", "GET /vcons/{uuid} query limit $", "no longer accepts integer"), found,
                      "a value an old client sends is refused")

    def test_a_removed_response_status_is_breaking(self):
        new = changed(lambda d: d["paths"]["/vcons/{uuid}"]["get"]["responses"].pop("404"))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)),
                         [("breaking", "GET /vcons/{uuid} 404", "response removed")], "a reader waits for it")

    def test_a_removed_response_member_is_breaking_through_components(self):
        new = changed(lambda d: d["components"]["schemas"]["Vcon"]["properties"].pop("parties"))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)),
                         [("breaking", "GET /vcons/{uuid} 200 application/json $.parties", "property removed")],
                         "a reader of parties finds none")

    def test_a_response_member_no_longer_required_is_breaking(self):
        new = changed(lambda d: d["components"]["schemas"]["Vcon"].update(required=[]))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)),
                         [("breaking", "GET /vcons/{uuid} 200 application/json $.uuid",
                           "no longer required, so it may be absent")], "a reader may find it absent")

    def test_a_response_that_may_be_null_is_breaking(self):
        new = changed(lambda d: d["components"]["schemas"]["Vcon"]["properties"]["uuid"]
                      .update(type=["string", "null"]))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)),
                         [("breaking", "GET /vcons/{uuid} 200 application/json $.uuid", "may now be of type null")],
                         "a 3.1 type array with null widens the output")

    def test_a_closed_response_enum_that_gains_a_value_is_breaking(self):
        old = get_returning(obj({"s": {"type": "string", "enum": ["x"]}}))
        new = get_returning(obj({"s": {"type": "string", "enum": ["x", "y"]}}))
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("breaking", "GET /x 200 application/json $.s", "closed enum gained ['y']")],
                         "a reader's switch has no case for it")

    def test_a_changed_response_const_is_breaking(self):
        old = get_returning(obj({"v": {"const": 1}}))
        new = get_returning(obj({"v": {"const": 2}}))
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("breaking", "GET /x 200 application/json $.v", "const 1 changed")],
                         "a reader that checks the value finds another")

    def test_a_response_enum_list_removed_is_breaking(self):
        old = get_returning(obj({"s": {"type": "string", "enum": ["x"]}}))
        new = get_returning(obj({"s": {"type": "string"}}))
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("breaking", "GET /x 200 application/json $.s", "enum list removed")],
                         "a reader's switch may meet any string")

    def test_a_removed_response_media_type_is_breaking(self):
        new = changed(lambda d: d["paths"]["/vcons/{uuid}"]["get"]["responses"].update(
            {"200": resp({"$ref": "#/components/schemas/Vcon"}, "application/vcon")}))
        found = kinds(cd.openapi_changes(BASE, new))
        self.assertIn(("breaking", "GET /vcons/{uuid} 200 application/json", "media type removed"), found,
                      "a reader that asks for it gets another")

    def test_a_removed_response_header_is_breaking(self):
        old = doc({"/x": {"get": op({"200": resp({"type": "object"}, headers=["ETag"])})}})
        new = doc({"/x": {"get": op({"200": resp({"type": "object"})})}})
        self.assertEqual(kinds(cd.openapi_changes(old, new)), [("breaking", "GET /x 200 header etag", "header removed")],
                         "a reader that reads the ETag finds none")

    def test_a_newly_required_request_body_is_breaking(self):
        old = doc({"/x": {"post": op(body=body({"type": "object"}, required=False))}})
        new = doc({"/x": {"post": op(body=body({"type": "object"}))}})
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("breaking", "POST /x body", "request body became required")], "an old client sends none")

    def test_a_new_required_request_member_is_breaking(self):
        new = changed(lambda d: (d["components"]["schemas"]["NewKey"]["properties"].update(tenant={"type": "string"}),
                                 d["components"]["schemas"]["NewKey"]["required"].append("tenant")))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)),
                         [("breaking", "POST /keys body application/json $.tenant", "required property added")],
                         "the writer rule: an old client does not send it")

    def test_a_request_member_made_required_is_breaking(self):
        new = changed(lambda d: d["components"]["schemas"]["NewKey"]["required"].append("days"))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)),
                         [("breaking", "POST /keys body application/json $.days", "became required")],
                         "an old client may leave it out")

    def test_a_removed_request_member_is_breaking(self):
        new = changed(lambda d: d["components"]["schemas"]["NewKey"]["properties"].pop("days"))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)),
                         [("breaking", "POST /keys body application/json $.days", "property removed")],
                         "the server refuses a member it does not know")

    def test_a_narrowed_request_type_is_breaking(self):
        new = changed(lambda d: d["components"]["schemas"]["NewKey"]["properties"]["days"].update(type="string"))
        found = kinds(cd.openapi_changes(BASE, new))
        self.assertIn(("breaking", "POST /keys body application/json $.days", "no longer accepts integer"), found,
                      "an old client's integer is refused")

    def test_a_removed_request_enum_value_is_breaking(self):
        new = changed(lambda d: d["components"]["schemas"]["NewKey"]["properties"]["scopes"]["items"]
                      .update(enum=["read"]))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)),
                         [("breaking", "POST /keys body application/json $.scopes[]", "no longer accepts ['write']")],
                         "an old client's value is refused")

    def test_a_tightened_request_bound_is_breaking(self):
        new = changed(lambda d: d["components"]["schemas"]["NewKey"]["properties"]["days"].update(maximum=30))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)),
                         [("breaking", "POST /keys body application/json $.days", "maximum lowered from 365 to 30")],
                         "an old client's 365 is refused")

    def test_a_changed_request_default_is_breaking(self):
        new = changed(lambda d: d["components"]["schemas"]["NewKey"]["properties"]["days"].update(default=30))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)),
                         [("breaking", "POST /keys body application/json $.days", "default changed from 90 to 30")],
                         "a client that leaves it out gets other behavior")

    def test_a_request_that_starts_refusing_unknown_members_is_breaking(self):
        old = post_body(obj({"a": {"type": "string"}}))
        new = post_body(obj({"a": {"type": "string"}}, additionalProperties=False))
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("breaking", "POST /x body application/json $", "unknown members are now refused")],
                         "an old client's extra member is refused")

    def test_a_removed_request_media_type_is_breaking(self):
        old = doc({"/x": {"post": op(body={"required": True, "content": {
            "application/json": {"schema": {"type": "object"}}, "application/vcon": {"schema": {"type": "string"}}}})}})
        new = post_body({"type": "object"})
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("breaking", "POST /x body application/vcon", "media type removed")],
                         "an old client's media type is refused")

    def test_a_removed_security_alternative_is_breaking(self):
        old = doc({"/x": {"get": op(security=[{"bearer": []}, {"session": []}])}})
        new = doc({"/x": {"get": op(security=[{"bearer": []}])}})
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("breaking", "GET /x", "security requirement session no longer accepted")],
                         "a session client is refused")

    def test_a_route_that_starts_needing_a_key_is_breaking(self):
        old = doc({"/x": {"get": op(security=[])}})
        new = doc({"/x": {"get": op()}}, security=[{"bearer": []}])
        self.assertEqual(kinds(breaking(cd.openapi_changes(old, new))),
                         [("breaking", "GET /x", "security requirement none no longer accepted")],
                         "a client that sent no key is refused")

    def test_a_removed_problem_type_is_breaking(self):
        self.assertEqual(kinds(cd.problem_changes({"bad-body", "gone"}, {"bad-body"})),
                         [("breaking", "problems/gone", "problem type removed")], "a client matches on its type")

    def test_main_exits_1_on_a_breaking_change(self):
        new = changed(lambda d: d["paths"]["/keys"].pop("post"))
        rc, out, _ = run_main(["--kind", "openapi", "OLD", "NEW"], {"OLD": BASE, "NEW": new})
        self.assertEqual(rc, 1, out)
        self.assertIn("breaking POST /keys: operation removed", out, "the change is named")
        self.assertIn("1 change(s), 1 breaking", out, "the count is printed")

    def test_against_exits_1_and_names_the_surface(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = tree(tmp, BASE, ["a.md"])
            fake = FakeGit({("v1", cd.OPENAPI_PATH): json.dumps(BASE), ("v1", cd.PROBLEMS_PATH): ["a.md", "b.md"]})
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                rc = cd.against("v1", root, fake)
        self.assertEqual(rc, 1, out.getvalue())
        self.assertIn("problems: breaking problems/b: problem type removed", out.getvalue(), out.getvalue())

    def test_parse_refuses_a_kind_the_script_does_not_take(self):
        self.assertIsNone(cd.parse(["--kind", "nonesuch", "a", "b"]), "nonesuch is not a surface")

    def test_parse_refuses_against_without_exactly_one_ref(self):
        self.assertEqual((cd.parse(["--against"]), cd.parse(["--against", "v1", "v2"])), (None, None),
                         "--against takes one ref")

    def test_the_page_names_each_error_the_script_prints(self):
        text = " ".join(STABILITY.read_text(encoding="utf-8").split())
        for message in ("names no commit; nothing was decided", "is not local to the document; nothing was decided",
                        "an OpenAPI document has no `paths` object; nothing was decided", "usage:"):
            self.assertIn(message, text, f"stability.md says what to do when the script prints {message!r}")

    def test_main_against_an_unknown_ref_is_an_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = tree(tmp, BASE, ["a.md"])
            out, err = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                rc = cd.against("v9", root, FakeGit({}))
        self.assertEqual(rc, 2, err.getvalue())
        self.assertIn("v9", err.getvalue(), "the ref is named")


class Edges(unittest.TestCase):
    def test_a_ref_cycle_ends(self):
        node = {"$ref": "#/components/schemas/Node"}
        d = doc({"/x": {"get": op({"200": resp(node)})}},
                schemas={"Node": obj({"next": node, "v": {"type": "string"}})})
        self.assertEqual(cd.openapi_changes(d, copy.deepcopy(d)), [], "a self-referencing schema ends")

    def test_a_change_inside_a_ref_cycle_is_found(self):
        node = {"$ref": "#/components/schemas/Node"}
        old = doc({"/x": {"get": op({"200": resp(node)})}}, schemas={"Node": obj({"next": node, "v": {"type": "string"}})})
        new = doc({"/x": {"get": op({"200": resp(node)})}}, schemas={"Node": obj({"next": node})})
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("breaking", "GET /x 200 application/json $.v", "property removed")], "the first level is read")

    def test_two_uses_of_one_schema_are_both_compared(self):
        ref = {"$ref": "#/components/schemas/P"}
        old = get_returning(obj({"a": ref, "b": ref}))
        old["components"] = {"schemas": {"P": obj({"n": {"type": "string"}})}}
        new = copy.deepcopy(old)
        new["components"]["schemas"]["P"]["properties"].pop("n")
        paths = [c.path for c in cd.openapi_changes(old, new)]
        self.assertEqual(paths, ["GET /x 200 application/json $.a.n", "GET /x 200 application/json $.b.n"],
                         "sibling uses are not a cycle")

    def test_a_remote_ref_is_an_error(self):
        new = get_returning({"$ref": "other.json#/X"})
        with self.assertRaises(cd.ContractError, msg="a schema this script cannot read is refused"):
            cd.openapi_changes(new, new)

    def test_an_unresolvable_ref_is_an_error(self):
        new = get_returning({"$ref": "#/components/schemas/Nope"})
        with self.assertRaises(cd.ContractError, msg="a dangling reference is refused"):
            cd.openapi_changes(new, new)

    def test_a_ref_with_an_escaped_pointer_resolves(self):
        d = get_returning({"$ref": "#/components/schemas/a~1b"})
        d["components"] = {"schemas": {"a/b": {"type": "string"}}}
        self.assertEqual(cd.openapi_changes(d, copy.deepcopy(d)), [], "~1 stands for /")

    def test_a_document_without_paths_is_an_error(self):
        with self.assertRaises(cd.ContractError, msg="a document with no paths object is refused"):
            cd.openapi_changes({"openapi": "3.1.1"}, BASE)

    def test_type_lists_compare_as_sets(self):
        old = get_returning(obj({"n": {"type": ["null", "integer"]}}))
        new = get_returning(obj({"n": {"type": ["integer", "null"]}}))
        self.assertEqual(cd.openapi_changes(old, new), [], "order of a type list is not a change")

    def test_number_to_integer_in_a_request_is_breaking(self):
        old = post_body(obj({"n": {"type": "number"}}))
        new = post_body(obj({"n": {"type": "integer"}}))
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("breaking", "POST /x body application/json $.n", "no longer accepts number")],
                         "a writer's 1.5 is refused")

    def test_integer_to_number_in_a_request_is_no_break(self):
        old = post_body(obj({"n": {"type": "integer"}}))
        new = post_body(obj({"n": {"type": "number"}}))
        self.assertEqual(breaking(cd.openapi_changes(old, new)), [], "every integer is a number")

    def test_integer_to_number_in_a_response_is_breaking(self):
        old = get_returning(obj({"n": {"type": "integer"}}))
        new = get_returning(obj({"n": {"type": "number"}}))
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("breaking", "GET /x 200 application/json $.n", "may now be of type number")],
                         "a reader of integer may get 1.5")

    def test_the_rule_depends_on_the_side_of_one_shared_schema(self):
        shared = {"$ref": "#/components/schemas/S"}
        old = doc({"/x": {"post": op({"200": resp(shared)}, body=body(shared))}},
                  schemas={"S": obj({"a": {"type": "string"}})})
        new = copy.deepcopy(old)
        new["components"]["schemas"]["S"]["required"] = ["a"]
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("breaking", "POST /x body application/json $.a", "became required")],
                         "required is breaking for the request and no change for the response")

    def test_a_request_const_added_is_breaking(self):
        old = post_body(obj({"v": {"type": "integer"}}))
        new = post_body(obj({"v": {"type": "integer", "const": 1}}))
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("breaking", "POST /x body application/json $.v", "now accepts only 1")],
                         "a const narrows a request")

    def test_a_request_enum_added_where_there_was_none_is_breaking(self):
        old = post_body(obj({"v": {"type": "string"}}))
        new = post_body(obj({"v": {"type": "string", "enum": ["a"]}}))
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("breaking", "POST /x body application/json $.v", "now accepts only ['a']")],
                         "an enum narrows a request")

    def test_a_request_pattern_change_is_breaking(self):
        old = post_body(obj({"v": {"type": "string", "pattern": "^[a-z]+$"}}))
        new = post_body(obj({"v": {"type": "string", "pattern": "^[a-f]+$"}}))
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("breaking", "POST /x body application/json $.v", "pattern changed")],
                         "two patterns cannot be compared, so a change is breaking")

    def test_a_request_default_first_stated_is_additive(self):
        old = post_body(obj({"v": {"type": "integer"}}))
        new = post_body(obj({"v": {"type": "integer", "default": 5}}))
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("additive", "POST /x body application/json $.v", "default stated: 5")],
                         "a default stated for the first time changes no promise")

    def test_a_request_default_removed_is_breaking(self):
        old = post_body(obj({"v": {"type": "integer", "default": 5}}))
        new = post_body(obj({"v": {"type": "integer"}}))
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("breaking", "POST /x body application/json $.v", "default 5 removed")],
                         "a client that leaves it out no longer knows what it gets")

    def test_request_items_that_must_be_unique_are_breaking(self):
        old = post_body(obj({"v": {"type": "array", "items": {"type": "string"}}}))
        new = post_body(obj({"v": {"type": "array", "items": {"type": "string"}, "uniqueItems": True}}))
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("breaking", "POST /x body application/json $.v", "items must now be unique")],
                         "an old client's repeated item is refused")

    def test_a_request_format_change_is_breaking(self):
        old = post_body(obj({"v": {"type": "string", "format": "uuid"}}))
        new = post_body(obj({"v": {"type": "string", "format": "date-time"}}))
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("breaking", "POST /x body application/json $.v", "format changed")],
                         "an old client's uuid is refused")

    def test_a_map_value_schema_is_compared(self):
        old = get_returning({"type": "object", "additionalProperties": {"type": "string"}})
        new = get_returning({"type": "object", "additionalProperties": {"type": ["string", "null"]}})
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("breaking", "GET /x 200 application/json $.*", "may now be of type null")],
                         "the values of a map are read as any other value")

    def test_a_composition_change_is_breaking(self):
        old = get_returning({"oneOf": [{"type": "string"}, {"type": "integer"}]})
        new = get_returning({"oneOf": [{"type": "string"}]})
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("breaking", "GET /x 200 application/json $", "oneOf changed, and it is not compared")],
                         "a part the script does not compare is never passed as unchanged")

    def test_header_parameters_match_without_case(self):
        old = doc({"/x": {"get": op(parameters=[{"name": "If-Match", "in": "header", "required": True}])}})
        new = doc({"/x": {"get": op(parameters=[{"name": "if-match", "in": "header", "required": True}])}})
        self.assertEqual(cd.openapi_changes(old, new), [], "HTTP header names have no case")

    def test_a_renamed_path_template_parameter_is_no_change(self):
        old = doc({"/k/{id}": {"get": op(parameters=[{"name": "id", "in": "path", "required": True}])}})
        new = doc({"/k/{key}": {"get": op(parameters=[{"name": "key", "in": "path", "required": True}])}})
        self.assertEqual(cd.openapi_changes(old, new), [], "the URL a client sends is the same")

    def test_path_level_parameters_apply_to_every_operation(self):
        new = changed(lambda d: d["paths"]["/vcons/{uuid}"]["parameters"].append(query("tenant", required=True)))
        self.assertEqual(kinds(cd.openapi_changes(BASE, new)),
                         [("breaking", "GET /vcons/{uuid} query tenant", "required parameter added")],
                         "a path-level parameter is the operation's")

    def test_a_path_parameter_counts_as_required_even_unmarked(self):
        old = doc({"/k/{id}": {"get": op()}})
        new = doc({"/k/{id}": {"get": op(parameters=[{"name": "id", "in": "path"}])}})
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("breaking", "GET /k/{id} path id", "required parameter added")],
                         "a path parameter is always required")

    def test_non_method_keys_of_a_path_item_are_not_operations(self):
        new = changed(lambda d: d["paths"]["/keys"].update(summary="s", description="d", servers=[]))
        self.assertEqual(cd.openapi_changes(BASE, new), [], "summary and servers are not methods")

    def test_a_response_without_a_schema_compares_status_only(self):
        old = doc({"/x": {"get": op({"204": resp(media=None)})}})
        new = doc({"/x": {"get": op({"204": resp(media=None, headers=["Retry-After"])})}})
        self.assertEqual(kinds(cd.openapi_changes(old, new)),
                         [("additive", "GET /x 204 header retry-after", "header added")], "only the header is new")

    def test_changes_come_in_a_stable_order(self):
        old = doc({"/a": {"post": op()}, "/b": {"get": op(), "delete": op()}})
        new = doc({})
        paths = [c.path for c in cd.openapi_changes(old, new)]
        self.assertEqual(paths, ["POST /a", "DELETE /b", "GET /b"], "sorted by path, then method")

    def test_problem_slugs_skip_the_index_and_other_files(self):
        self.assertEqual(cd.problem_slugs(["a.md", "_index.md", "notes.txt", "b.md"]), {"a", "b"},
                         "only the .md pages that are not the index")

    def test_against_a_ref_without_a_surface_reports_no_baseline(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = tree(tmp, BASE, ["a.md"])
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                rc = cd.against("v1", root, FakeGit({}))
        self.assertEqual(rc, 0, out.getvalue())
        self.assertIn("openapi: no baseline at v1", out.getvalue(), "a surface new since the ref passes")
        self.assertIn("problems: no baseline at v1", out.getvalue(), "a surface new since the ref passes")

    def test_main_with_a_missing_file_is_an_error(self):
        rc, _, err = run_main(["--kind", "openapi", "/nonexistent/old.json", "/nonexistent/new.json"])
        self.assertEqual(rc, 2, err)
        self.assertIn("nothing was decided", err, "the error says no verdict was reached")

    def test_main_with_malformed_json_is_an_error(self):
        rc, _, err = run_main(["--kind", "openapi", "OLD", "NEW"], {"OLD": "{not json", "NEW": BASE})
        self.assertEqual(rc, 2, err)

    def test_main_with_an_unknown_kind_is_a_usage_error(self):
        rc, _, err = run_main(["--kind", "schema", "a", "b"])
        self.assertEqual(rc, 2, err)
        self.assertIn("usage:", err, "the usage is printed")

    def test_main_with_too_few_arguments_is_a_usage_error(self):
        rc, _, err = run_main(["--kind", "openapi", "a"])
        self.assertEqual((rc, "usage:" in err), (2, True), err)


# The command line: contract/cli.json, as tests/cli_contract.rs writes it.
CLI_SNAPSHOT = ROOT / "contract/cli.json"


def flag(short=None, value=None, required=False, repeated=False, default=None, values=None):
    return {"short": short, "value": value, "required": required, "repeated": repeated,
            "default": default, "values": values}


def arg(name, required=True, repeated=False, default=None, values=None):
    return {"name": name, "required": required, "repeated": repeated, "default": default, "values": values}


CLI_BASE = {
    "commands": {
        "vcon-server": {"arguments": [], "flags": {"--help": flag("-h"), "--version": flag("-V")}},
        "vcon-server key create": {"arguments": [], "flags": {
            "--config": flag(value="<CONFIG>", required=True),
            "--label": flag(value="<LABEL>", required=True),
            "--scope": flag(value="<SCOPES>", repeated=True, values=["read", "write"]),
            "--expires-days": flag(value="<DAYS>", default="90"),
            "--tenant": flag("-t", value="<TENANT>"),
            "--no-expiry": flag(),
            "--help": flag("-h"),
        }},
        "vcon-server tenant remove": {"arguments": [arg("ID")], "flags": {
            "--config": flag(value="<CONFIG>", required=True),
            "--force": flag(),
            "--help": flag("-h"),
        }},
    },
    "exit_codes": {"0": "done", "1": "refused or failed", "2": "the command line does not parse"},
}
KEY = "vcon-server key create"
REMOVE = "vcon-server tenant remove"


def cli_changed(edit):
    """A deep copy of CLI_BASE after `edit(copy)`."""
    d = copy.deepcopy(CLI_BASE)
    edit(d)
    return d


def key_flag(name, **values):
    """An edit that sets members of one flag of `key create`."""
    return lambda d: d["commands"][KEY]["flags"][name].update(values)


class CliPasses(unittest.TestCase):
    def test_identical_snapshots_have_no_change(self):
        self.assertEqual(cd.cli_changes(CLI_BASE, copy.deepcopy(CLI_BASE)), [], "a snapshot equals itself")

    def test_the_snapshot_in_the_tree_is_unchanged_against_itself(self):
        d = json.loads(CLI_SNAPSHOT.read_text(encoding="utf-8"))
        self.assertEqual(cd.cli_changes(d, copy.deepcopy(d)), [], "contract/cli.json compared with itself")

    def test_the_snapshot_in_the_tree_holds_the_three_exit_codes(self):
        d = json.loads(CLI_SNAPSHOT.read_text(encoding="utf-8"))
        self.assertEqual(sorted(d["exit_codes"]), ["0", "1", "2"], "command-line.md lists 0, 1 and 2")

    def test_the_snapshot_in_the_tree_reads_every_command(self):
        d = json.loads(CLI_SNAPSHOT.read_text(encoding="utf-8"))
        found = cd.cli_changes(d, {"commands": {}, "exit_codes": d["exit_codes"]})
        self.assertEqual(len(found), len(d["commands"]), "each command's removal is one change")
        self.assertTrue(all(c.message == "command removed" for c in found), found)

    def test_an_optional_flag_added_is_additive(self):
        d = cli_changed(lambda d: d["commands"][KEY]["flags"].update({"--dry-run": flag()}))
        self.assertEqual(kinds(cd.cli_changes(CLI_BASE, d)),
                         [("additive", f"{KEY} --dry-run", "optional flag added")], "an old command line still parses")

    def test_a_command_added_is_additive(self):
        d = cli_changed(lambda d: d["commands"].update({"vcon-server key rotate": {"arguments": [], "flags": {}}}))
        self.assertEqual(kinds(cd.cli_changes(CLI_BASE, d)),
                         [("additive", "vcon-server key rotate", "command added")], "a new command breaks no script")

    def test_a_flag_no_longer_required_is_additive(self):
        found = cd.cli_changes(CLI_BASE, cli_changed(key_flag("--label", required=False)))
        self.assertEqual(kinds(found), [("additive", f"{KEY} --label", "no longer required")], found)

    def test_an_optional_argument_added_is_additive(self):
        d = cli_changed(lambda d: d["commands"][REMOVE]["arguments"].append(arg("REASON", required=False)))
        self.assertEqual(kinds(cd.cli_changes(CLI_BASE, d)),
                         [("additive", f"{REMOVE} <2>", "optional argument added")], "an old command line still parses")

    def test_an_argument_no_longer_required_is_additive(self):
        d = cli_changed(lambda d: d["commands"][REMOVE]["arguments"][0].update(required=False))
        self.assertEqual(kinds(cd.cli_changes(CLI_BASE, d)), [("additive", f"{REMOVE} <1>", "no longer required")],
                         "a command line that gives it still parses")

    def test_a_default_stated_where_there_was_none_is_additive(self):
        found = cd.cli_changes(CLI_BASE, cli_changed(key_flag("--tenant", default="t1")))
        self.assertEqual(kinds(found), [("additive", f"{KEY} --tenant", "default stated: 't1'")], found)

    def test_a_possible_value_added_is_additive(self):
        found = cd.cli_changes(CLI_BASE, cli_changed(key_flag("--scope", values=["read", "write", "audit"])))
        self.assertEqual(kinds(found), [("additive", f"{KEY} --scope", "accepts more values: ['audit']")], found)

    def test_a_list_of_possible_values_removed_is_additive(self):
        found = cd.cli_changes(CLI_BASE, cli_changed(key_flag("--scope", values=None)))
        self.assertEqual(kinds(found), [("additive", f"{KEY} --scope", "accepts any value")], found)

    def test_a_flag_that_may_now_be_repeated_is_additive(self):
        found = cd.cli_changes(CLI_BASE, cli_changed(key_flag("--label", repeated=True)))
        self.assertEqual(kinds(found), [("additive", f"{KEY} --label", "may now be repeated")], found)

    def test_a_short_name_added_is_additive(self):
        found = cd.cli_changes(CLI_BASE, cli_changed(key_flag("--label", short="-l")))
        self.assertEqual(kinds(found), [("additive", f"{KEY} --label", "short name -l added")], found)

    def test_a_value_that_may_now_be_left_out_is_additive(self):
        found = cd.cli_changes(CLI_BASE, cli_changed(key_flag("--expires-days", value="[<DAYS>]")))
        self.assertEqual(kinds(found), [("additive", f"{KEY} --expires-days", "its value may now be left out")], found)

    def test_a_renamed_value_is_no_change(self):
        found = cd.cli_changes(CLI_BASE, cli_changed(key_flag("--scope", value="<SCOPE>")))
        self.assertEqual(found, [], "the value's name is --help text, not part of what a script types")

    def test_main_compares_two_cli_files(self):
        d = cli_changed(lambda d: d["commands"][KEY]["flags"].update({"--dry-run": flag()}))
        rc, out, err = run_main(["--kind", "cli", "OLD", "NEW"], {"OLD": CLI_BASE, "NEW": d})
        self.assertEqual(rc, 0, err)
        self.assertIn(f"additive {KEY} --dry-run: optional flag added", out, out)
        self.assertIn("contract_diff: 1 change(s), 0 breaking", out, out)

    def test_against_compares_the_cli_snapshot_at_the_ref(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = tree(tmp, BASE, ["a.md"], CLI_BASE)
            fake = FakeGit({("v1", cd.OPENAPI_PATH): json.dumps(BASE), ("v1", cd.PROBLEMS_PATH): ["a.md"],
                            ("v1", cd.CLI_PATH): json.dumps(CLI_BASE)})
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                rc = cd.against("v1", root, fake)
        self.assertEqual(rc, 0, out.getvalue())
        self.assertIn("cli: 0 change(s), 0 breaking", out.getvalue(), "the cli surface reports its count")

    def test_against_a_ref_without_a_cli_snapshot_is_no_baseline(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = tree(tmp, BASE, ["a.md"], CLI_BASE)
            fake = FakeGit({("v1", cd.OPENAPI_PATH): json.dumps(BASE), ("v1", cd.PROBLEMS_PATH): ["a.md"]})
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                rc = cd.against("v1", root, fake)
        self.assertEqual(rc, 0, out.getvalue())
        self.assertIn("cli: no baseline at v1", out.getvalue(), "a release before contract/cli.json passes")

    def test_parse_takes_the_cli_kind(self):
        self.assertEqual(cd.parse(["--kind", "cli", "a", "b"]), ("cli", "a", "b"), "cli is a surface")


class CliFails(unittest.TestCase):
    def one(self, edit, where, message):
        found = cd.cli_changes(CLI_BASE, cli_changed(edit))
        self.assertEqual(kinds(found), [("breaking", where, message)], found)

    def test_a_command_removed_is_breaking(self):
        self.one(lambda d: d["commands"].pop(REMOVE), REMOVE, "command removed")

    def test_a_flag_removed_is_breaking(self):
        self.one(lambda d: d["commands"][REMOVE]["flags"].pop("--force"), f"{REMOVE} --force", "flag removed")

    def test_a_required_flag_added_is_breaking(self):
        self.one(lambda d: d["commands"][REMOVE]["flags"].update({"--reason": flag(value="<R>", required=True)}),
                 f"{REMOVE} --reason", "required flag added")

    def test_a_flag_that_became_required_is_breaking(self):
        self.one(key_flag("--tenant", required=True), f"{KEY} --tenant", "became required")

    def test_a_flag_that_stops_taking_a_value_is_breaking(self):
        self.one(key_flag("--tenant", value=None), f"{KEY} --tenant", "no longer takes a value")

    def test_a_flag_that_starts_taking_a_value_is_breaking(self):
        self.one(key_flag("--no-expiry", value="<WHEN>"), f"{KEY} --no-expiry", "now takes a value")

    def test_a_value_that_may_no_longer_be_left_out_is_breaking(self):
        d = cli_changed(key_flag("--expires-days", value="[<DAYS>]"))
        found = cd.cli_changes(d, CLI_BASE)
        self.assertEqual(kinds(found), [("breaking", f"{KEY} --expires-days", "its value may no longer be left out")],
                         found)

    def test_a_changed_default_is_breaking(self):
        self.one(key_flag("--expires-days", default="30"), f"{KEY} --expires-days",
                 "default changed from '90' to '30'")

    def test_a_removed_default_is_breaking(self):
        self.one(key_flag("--expires-days", default=None), f"{KEY} --expires-days", "default '90' removed")

    def test_a_possible_value_removed_is_breaking(self):
        self.one(key_flag("--scope", values=["read"]), f"{KEY} --scope", "no longer accepts ['write']")

    def test_a_list_of_possible_values_set_where_there_was_none_is_breaking(self):
        self.one(key_flag("--tenant", values=["a", "b"]), f"{KEY} --tenant", "now accepts only ['a', 'b']")

    def test_a_flag_that_may_no_longer_be_repeated_is_breaking(self):
        self.one(key_flag("--scope", repeated=False), f"{KEY} --scope", "may no longer be repeated")

    def test_a_short_name_removed_is_breaking(self):
        self.one(key_flag("--tenant", short=None), f"{KEY} --tenant", "short name -t removed")

    def test_a_short_name_changed_is_breaking(self):
        self.one(key_flag("--tenant", short="-T"), f"{KEY} --tenant", "short name -t changed to -T")

    def test_an_argument_removed_is_breaking(self):
        self.one(lambda d: d["commands"][REMOVE]["arguments"].clear(), f"{REMOVE} <1>", "argument removed")

    def test_a_required_argument_added_is_breaking(self):
        self.one(lambda d: d["commands"][REMOVE]["arguments"].append(arg("REASON")), f"{REMOVE} <2>",
                 "required argument added")

    def test_an_argument_that_became_required_is_breaking(self):
        d = cli_changed(lambda d: d["commands"][REMOVE]["arguments"][0].update(required=False))
        found = cd.cli_changes(d, CLI_BASE)
        self.assertEqual(kinds(found), [("breaking", f"{REMOVE} <1>", "became required")], found)

    def test_an_argument_whose_default_changed_is_breaking(self):
        old = cli_changed(lambda d: d["commands"][REMOVE]["arguments"][0].update(default="a"))
        new = cli_changed(lambda d: d["commands"][REMOVE]["arguments"][0].update(default="b"))
        self.assertEqual(kinds(cd.cli_changes(old, new)),
                         [("breaking", f"{REMOVE} <1>", "default changed from 'a' to 'b'")], "an argument's default")

    def test_an_argument_that_lost_a_possible_value_is_breaking(self):
        old = cli_changed(lambda d: d["commands"][REMOVE]["arguments"][0].update(values=["none", "full"]))
        new = cli_changed(lambda d: d["commands"][REMOVE]["arguments"][0].update(values=["none"]))
        self.assertEqual(kinds(cd.cli_changes(old, new)),
                         [("breaking", f"{REMOVE} <1>", "no longer accepts ['full']")], "an argument's values")

    def test_an_exit_code_removed_is_breaking(self):
        self.one(lambda d: d["exit_codes"].pop("2"), "exit 2", "exit code removed")

    def test_an_exit_code_whose_meaning_changed_is_breaking(self):
        self.one(lambda d: d["exit_codes"].update({"1": "refused"}), "exit 1",
                 "meaning changed from 'refused or failed' to 'refused'")

    def test_an_exit_code_added_is_breaking(self):
        self.one(lambda d: d["exit_codes"].update({"3": "busy"}), "exit 3",
                 "exit code added, which a script written for the older release does not know")

    def test_main_exits_1_on_a_breaking_cli_change(self):
        d = cli_changed(lambda d: d["commands"].pop(REMOVE))
        rc, out, err = run_main(["--kind", "cli", "OLD", "NEW"], {"OLD": CLI_BASE, "NEW": d})
        self.assertEqual(rc, 1, err)
        self.assertIn(f"breaking {REMOVE}: command removed", out, out)

    def test_against_exits_1_on_a_breaking_cli_change(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = tree(tmp, BASE, ["a.md"], cli_changed(lambda d: d["commands"].pop(REMOVE)))
            fake = FakeGit({("v1", cd.OPENAPI_PATH): json.dumps(BASE), ("v1", cd.PROBLEMS_PATH): ["a.md"],
                            ("v1", cd.CLI_PATH): json.dumps(CLI_BASE)})
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                rc = cd.against("v1", root, fake)
        self.assertEqual(rc, 1, out.getvalue())
        self.assertIn(f"cli: breaking {REMOVE}: command removed", out.getvalue(), out.getvalue())

    def test_against_without_the_snapshot_in_the_work_tree_is_an_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = tree(tmp, BASE, ["a.md"])
            fake = FakeGit({("v1", cd.OPENAPI_PATH): json.dumps(BASE), ("v1", cd.PROBLEMS_PATH): ["a.md"],
                            ("v1", cd.CLI_PATH): json.dumps(CLI_BASE)})
            out, err = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                rc = cd.against("v1", root, fake)
        self.assertEqual(rc, 2, err.getvalue())
        self.assertIn("nothing was decided", err.getvalue(), err.getvalue())

    def test_the_page_names_each_cli_error_the_script_prints(self):
        text = " ".join(STABILITY.read_text(encoding="utf-8").split())
        self.assertIn("a command line snapshot has no `commands` object; nothing was decided", text,
                      "stability.md says what to do when a cli file is not a snapshot")


class CliEdges(unittest.TestCase):
    def test_a_group_removed_names_each_command_under_it(self):
        d = cli_changed(lambda d: [d["commands"].pop(k) for k in list(d["commands"]) if k.startswith("vcon-server ")])
        found = cd.cli_changes(CLI_BASE, d)
        self.assertEqual([c.path for c in breaking(found)], [KEY, REMOVE], "each command is named once, in order")

    def test_a_snapshot_without_commands_is_an_error(self):
        for bad in ({"exit_codes": {}}, {"commands": [], "exit_codes": {}}, [], "x"):
            with self.assertRaises(cd.ContractError, msg=repr(bad)):
                cd.cli_changes(CLI_BASE, bad)

    def test_a_snapshot_without_exit_codes_is_an_error(self):
        for bad in ({"commands": {}}, {"commands": {}, "exit_codes": []}):
            with self.assertRaises(cd.ContractError, msg=repr(bad)):
                cd.cli_changes(bad, CLI_BASE)

    def test_a_command_without_flags_or_arguments_is_an_error(self):
        for bad in ({}, {"flags": {}}, {"arguments": []}, {"flags": [], "arguments": []}):
            d = cli_changed(lambda d, bad=bad: d["commands"].update({REMOVE: bad}))
            with self.assertRaises(cd.ContractError, msg=repr(bad)):
                cd.cli_changes(CLI_BASE, d)

    def test_main_on_a_file_that_is_not_a_snapshot_is_an_error(self):
        rc, _, err = run_main(["--kind", "cli", "OLD", "NEW"], {"OLD": CLI_BASE, "NEW": {"exit_codes": {}}})
        self.assertEqual(rc, 2, err)
        self.assertIn("a command line snapshot has no `commands` object; nothing was decided", err, err)

    def test_main_on_a_file_that_is_not_json_is_an_error(self):
        rc, _, err = run_main(["--kind", "cli", "OLD", "NEW"], {"OLD": "{", "NEW": CLI_BASE})
        self.assertEqual(rc, 2, err)

    def test_against_with_a_baseline_that_is_not_json_is_an_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = tree(tmp, BASE, ["a.md"], CLI_BASE)
            fake = FakeGit({("v1", cd.OPENAPI_PATH): json.dumps(BASE), ("v1", cd.PROBLEMS_PATH): ["a.md"],
                            ("v1", cd.CLI_PATH): "{"})
            out, err = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                rc = cd.against("v1", root, fake)
        self.assertEqual(rc, 2, err.getvalue())

    def test_a_meaning_that_differs_only_in_spacing_and_case_is_no_change(self):
        found = cd.cli_changes(CLI_BASE, cli_changed(lambda d: d["exit_codes"].update({"1": "Refused  or\nfailed"})))
        self.assertEqual(found, [], "a meaning rewrapped on the page is the same meaning")

    def test_possible_values_in_another_order_are_no_change(self):
        found = cd.cli_changes(CLI_BASE, cli_changed(key_flag("--scope", values=["write", "read"])))
        self.assertEqual(found, [], "the order --help lists the values in is not compared")

    def test_flags_in_another_order_are_no_change(self):
        d = cli_changed(lambda d: d["commands"][KEY].update(
            flags=dict(reversed(list(d["commands"][KEY]["flags"].items())))))
        self.assertEqual(cd.cli_changes(CLI_BASE, d), [], "flags are compared by name")

    def test_a_renamed_argument_is_no_change(self):
        d = cli_changed(lambda d: d["commands"][REMOVE]["arguments"][0].update(name="TENANT"))
        self.assertEqual(cd.cli_changes(CLI_BASE, d), [], "an argument is its place; its name is --help text")

    def test_a_second_argument_removed_is_named_by_its_place(self):
        two = cli_changed(lambda d: d["commands"][REMOVE]["arguments"].append(arg("REASON", required=False)))
        found = cd.cli_changes(two, CLI_BASE)
        self.assertEqual(kinds(found), [("breaking", f"{REMOVE} <2>", "argument removed")], found)

    def test_a_short_only_flag_is_compared_by_its_key(self):
        old = cli_changed(lambda d: d["commands"][REMOVE]["flags"].update({"-q": flag("-q")}))
        found = cd.cli_changes(old, CLI_BASE)
        self.assertEqual(kinds(found), [("breaking", f"{REMOVE} -q", "flag removed")], found)

    def test_an_empty_default_is_a_default(self):
        old = cli_changed(key_flag("--tenant", default=""))
        found = cd.cli_changes(old, CLI_BASE)
        self.assertEqual(kinds(found), [("breaking", f"{KEY} --tenant", "default '' removed")], found)

    def test_the_root_help_flag_is_part_of_the_contract(self):
        found = cd.cli_changes(CLI_BASE, cli_changed(lambda d: d["commands"]["vcon-server"]["flags"].pop("--help")))
        self.assertEqual(kinds(found), [("breaking", "vcon-server --help", "flag removed")], found)

    def test_a_command_that_becomes_a_group_reports_both_sides(self):
        new = cli_changed(lambda d: d["commands"].update({
            REMOVE: {"arguments": [], "flags": {"--help": flag("-h")}},
            f"{REMOVE} now": {"arguments": [arg("ID")], "flags": {"--config": flag(value="<CONFIG>", required=True)}},
        }))
        found = kinds(cd.cli_changes(CLI_BASE, new))
        self.assertIn(("breaking", f"{REMOVE} <1>", "argument removed"), found, found)
        self.assertIn(("breaking", f"{REMOVE} --config", "flag removed"), found, found)
        self.assertIn(("additive", f"{REMOVE} now", "command added"), found, found)

    def test_several_changes_to_one_flag_are_each_named(self):
        found = cd.cli_changes(CLI_BASE, cli_changed(key_flag("--tenant", required=True, short=None)))
        self.assertEqual(sorted(c.message for c in found), ["became required", "short name -t removed"], found)

    def test_changes_come_in_the_order_of_the_commands_then_exit_codes(self):
        d = cli_changed(lambda d: (d["exit_codes"].pop("0"), d["commands"].pop(KEY)))
        found = cd.cli_changes(CLI_BASE, d)
        self.assertEqual([c.path for c in found], [KEY, "exit 0"], "commands first, then exit codes")


CONFIG_SNAPSHOT = ROOT / "contract/config.json"


def key(type_="integer", required="no", default="`5`"):
    """One key as contract/config.json holds it: a required key has no default."""
    return {"default": default if required == "no" else None, "required": required, "type": type_}


CONFIG_BASE = {"keys": {
    "listen": key("string", "always"),
    "purge_interval": key(default="`3600`"),
    "storage.objects": key("string", default="`local`"),
    "storage.s3.region": key("string", "with its table"),
    "storage.s3.timeout": key(default="`30`"),
    "supported_critical": key("array of string", default="none"),
}}


def config_changed(edit):
    """A deep copy of CONFIG_BASE after `edit(copy)`."""
    d = copy.deepcopy(CONFIG_BASE)
    edit(d)
    return d


def set_key(name, **fields):
    """An edit that sets `fields` on the key `name`."""
    return lambda d: d["keys"][name].update(fields)


def add_key(name, entry):
    return lambda d: d["keys"].update({name: entry})


class ConfigPasses(unittest.TestCase):
    def test_identical_snapshots_have_no_change(self):
        self.assertEqual(cd.config_changes(CONFIG_BASE, copy.deepcopy(CONFIG_BASE)), [], "a snapshot equals itself")

    def test_the_snapshot_in_the_tree_is_unchanged_against_itself(self):
        d = json.loads(CONFIG_SNAPSHOT.read_text(encoding="utf-8"))
        self.assertEqual(cd.config_changes(d, copy.deepcopy(d)), [], "contract/config.json compared with itself")

    def test_the_snapshot_in_the_tree_holds_the_64_keys_of_the_full_file(self):
        d = json.loads(CONFIG_SNAPSHOT.read_text(encoding="utf-8"))
        self.assertEqual(len(d["keys"]), 64, "config.rs FULL sets 64 keys")

    def test_the_snapshot_in_the_tree_reads_every_key(self):
        d = json.loads(CONFIG_SNAPSHOT.read_text(encoding="utf-8"))
        found = cd.config_changes(d, {"keys": {}})
        self.assertEqual(len(found), 64, "each key's removal is one change")
        self.assertTrue(all(c.kind == "breaking" for c in found), found)

    def test_an_optional_key_added_is_additive(self):
        found = cd.config_changes(CONFIG_BASE, config_changed(add_key("limits.body", key(default="`16`"))))
        self.assertEqual(kinds(found), [("additive", "limits.body", "optional key added")], "an old file still loads")

    def test_a_required_key_in_a_new_table_is_additive(self):
        found = cd.config_changes(CONFIG_BASE, config_changed(add_key("export.s3.region", key("string", "with its table"))))
        self.assertEqual(kinds(found), [("additive", "export.s3.region", "key added in a new table")],
                         "no old file has the table: deny_unknown_fields refused it")

    def test_a_key_no_longer_required_is_additive(self):
        found = cd.config_changes(CONFIG_BASE, config_changed(set_key("listen", required="no", default="`0.0.0.0:443`")))
        self.assertEqual(kinds(found), [("additive", "listen", "no longer required")], found)

    def test_a_key_required_only_with_its_table_from_always_is_additive(self):
        new = config_changed(set_key("listen", required="with its table"))
        self.assertEqual(kinds(cd.config_changes(CONFIG_BASE, new)), [("additive", "listen", "no longer required")],
                         "a file without the table may now leave it out")

    def test_a_default_stated_where_there_was_none_is_additive(self):
        old = config_changed(set_key("purge_interval", default=None))
        found = cd.config_changes(old, CONFIG_BASE)
        self.assertEqual(kinds(found), [("additive", "purge_interval", "default stated: '`3600`'")], found)

    def test_main_with_kind_config_exits_0_on_additive_changes(self):
        new = config_changed(add_key("limits.body", key()))
        rc, out, err = run_main(["--kind", "config", "OLD", "NEW"], {"OLD": CONFIG_BASE, "NEW": new})
        self.assertEqual(rc, 0, err)
        self.assertIn("additive limits.body: optional key added", out, out)
        self.assertIn("contract_diff: 1 change(s), 0 breaking", out, out)

    def test_against_a_ref_without_config_json_reports_no_baseline(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = tree(tmp, BASE, ["a.md"], CLI_BASE, CONFIG_BASE)
            fake = FakeGit({("v1", cd.OPENAPI_PATH): json.dumps(BASE), ("v1", cd.PROBLEMS_PATH): ["a.md"],
                            ("v1", cd.CLI_PATH): json.dumps(CLI_BASE)})
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                rc = cd.against("v1", root, fake)
        self.assertEqual(rc, 0, out.getvalue())
        self.assertIn("config: no baseline at v1", out.getvalue(), out.getvalue())

    def test_against_a_ref_with_the_same_config_json_counts_no_change(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = tree(tmp, BASE, ["a.md"], CLI_BASE, CONFIG_BASE)
            fake = FakeGit({("v1", cd.OPENAPI_PATH): json.dumps(BASE), ("v1", cd.PROBLEMS_PATH): ["a.md"],
                            ("v1", cd.CLI_PATH): json.dumps(CLI_BASE), ("v1", cd.CONFIG_PATH): json.dumps(CONFIG_BASE)})
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                rc = cd.against("v1", root, fake)
        self.assertEqual(rc, 0, out.getvalue())
        self.assertIn("config: 0 change(s), 0 breaking", out.getvalue(), out.getvalue())

    def test_the_pages_example_output_is_what_the_script_prints(self):
        old = config_changed(add_key("limits.body", key(default="`16777216`")))
        new = config_changed(lambda d: d["keys"].update({"limits.body": key(default="`8388608`"),
                                                          "storage.s3.sse": key("string", default="none")}))
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            cd._report("", cd.config_changes(old, new))
        text = STABILITY.read_text(encoding="utf-8")
        self.assertIn(out.getvalue(), text, "stability.md shows the script's own lines for that change")

    def test_the_page_names_the_command_that_writes_the_snapshot(self):
        text = STABILITY.read_text(encoding="utf-8")
        self.assertIn("VCON_SERVER_WRITE_CONTRACT=1 cargo test -p vcon-server --lib config::tests::contract", text,
                      "stability.md names the one command the test's message names")


class ConfigFails(unittest.TestCase):
    def one(self, edit, where, message, old=None):
        found = cd.config_changes(old or CONFIG_BASE, config_changed(edit))
        self.assertEqual(kinds(found), [("breaking", where, message)], found)

    def test_a_key_removed_is_breaking(self):
        self.one(lambda d: d["keys"].pop("purge_interval"), "purge_interval",
                 "key removed: a file that sets it no longer loads")

    def test_a_required_key_added_is_breaking(self):
        self.one(add_key("data_dir", key("string", "always")), "data_dir", "required key added")

    def test_a_required_key_added_to_a_table_that_exists_is_breaking(self):
        self.one(add_key("storage.s3.bucket", key("string", "with its table")), "storage.s3.bucket",
                 "required key added to a table an older file may have")

    def test_a_key_that_became_required_is_breaking(self):
        self.one(set_key("purge_interval", required="always", default=None), "purge_interval", "became required")

    def test_a_key_that_became_required_with_its_table_is_breaking(self):
        self.one(set_key("storage.s3.timeout", required="with its table", default=None), "storage.s3.timeout",
                 "became required")

    def test_a_key_required_even_without_its_table_is_breaking(self):
        self.one(set_key("storage.s3.region", required="always"), "storage.s3.region",
                 "now required in a file without its table")

    def test_a_changed_type_is_breaking(self):
        self.one(set_key("purge_interval", type="string"), "purge_interval",
                 "type changed from 'integer' to 'string'")

    def test_a_changed_default_is_breaking(self):
        self.one(set_key("purge_interval", default="`1800`"), "purge_interval",
                 "default changed from '`3600`' to '`1800`'")

    def test_a_removed_default_is_breaking(self):
        self.one(set_key("storage.objects", default=None), "storage.objects", "default '`local`' removed")

    def test_main_with_kind_config_exits_1_on_a_breaking_change(self):
        d = config_changed(lambda d: d["keys"].pop("listen"))
        rc, out, err = run_main(["--kind", "config", "OLD", "NEW"], {"OLD": CONFIG_BASE, "NEW": d})
        self.assertEqual(rc, 1, err)
        self.assertIn("breaking listen: key removed: a file that sets it no longer loads", out, out)

    def test_against_exits_1_on_a_breaking_config_change(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = tree(tmp, BASE, ["a.md"], CLI_BASE, config_changed(set_key("purge_interval", type="string")))
            fake = FakeGit({("v1", cd.OPENAPI_PATH): json.dumps(BASE), ("v1", cd.PROBLEMS_PATH): ["a.md"],
                            ("v1", cd.CLI_PATH): json.dumps(CLI_BASE), ("v1", cd.CONFIG_PATH): json.dumps(CONFIG_BASE)})
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                rc = cd.against("v1", root, fake)
        self.assertEqual(rc, 1, out.getvalue())
        self.assertIn("config: breaking purge_interval: type changed", out.getvalue(), out.getvalue())

    def test_against_without_config_json_in_the_work_tree_is_an_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = tree(tmp, BASE, ["a.md"], CLI_BASE)
            fake = FakeGit({("v1", cd.OPENAPI_PATH): json.dumps(BASE), ("v1", cd.PROBLEMS_PATH): ["a.md"],
                            ("v1", cd.CLI_PATH): json.dumps(CLI_BASE), ("v1", cd.CONFIG_PATH): json.dumps(CONFIG_BASE)})
            out, err = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                rc = cd.against("v1", root, fake)
        self.assertEqual(rc, 2, err.getvalue())
        self.assertIn("nothing was decided", err.getvalue(), err.getvalue())

    def test_the_page_names_each_config_error_the_script_prints(self):
        text = " ".join(STABILITY.read_text(encoding="utf-8").split())
        self.assertIn("a configuration snapshot has no `keys` object; nothing was decided", text,
                      "stability.md says what to do when a config file is not a snapshot")
        self.assertIn("contract/config.json is not what the configuration code reads", text,
                      "stability.md names the lib test's message")

    def test_a_required_key_moved_to_another_table_is_breaking_at_both_names(self):
        new = config_changed(lambda d: (d["keys"].pop("storage.s3.region"),
                                        d["keys"].update({"storage.s3.endpoint.region": key("string", "always")})))
        found = kinds(cd.config_changes(CONFIG_BASE, new))
        self.assertEqual(found, [("breaking", "storage.s3.endpoint.region", "required key added"),
                                 ("breaking", "storage.s3.region", "key removed: a file that sets it no longer loads")],
                         "a move is a removal and an addition")


class ConfigEdges(unittest.TestCase):
    def test_a_snapshot_without_keys_is_an_error(self):
        for bad in ({}, {"keys": []}, [], "x"):
            with self.assertRaises(cd.ContractError, msg=repr(bad)):
                cd.config_changes(CONFIG_BASE, bad)

    def test_a_key_without_a_type_or_with_an_unknown_required_state_is_an_error(self):
        for bad in ({}, {"type": "string", "required": "maybe", "default": None},
                    {"type": 1, "required": "no", "default": None}, {"type": "string", "required": "no", "default": 5},
                    "x"):
            d = config_changed(add_key("listen", bad))
            with self.assertRaises(cd.ContractError, msg=repr(bad)):
                cd.config_changes(CONFIG_BASE, d)

    def test_main_on_a_file_that_is_not_a_snapshot_is_an_error(self):
        rc, _, err = run_main(["--kind", "config", "OLD", "NEW"], {"OLD": CONFIG_BASE, "NEW": {"commands": {}}})
        self.assertEqual(rc, 2, err)
        self.assertIn("a configuration snapshot has no `keys` object; nothing was decided", err, err)

    def test_main_on_a_file_that_is_not_json_is_an_error(self):
        rc, _, err = run_main(["--kind", "config", "OLD", "NEW"], {"OLD": CONFIG_BASE, "NEW": "{"})
        self.assertEqual(rc, 2, err)

    def test_a_required_key_added_at_the_top_with_its_table_is_breaking(self):
        found = cd.config_changes(CONFIG_BASE, config_changed(add_key("data_dir", key("string", "with its table"))))
        self.assertEqual(kinds(found), [("breaking", "data_dir", "required key added to a table an older file may have")],
                         "the top level is a table every file has")

    def test_a_required_key_in_a_new_table_under_one_that_exists_is_additive(self):
        found = cd.config_changes(CONFIG_BASE, config_changed(add_key("storage.s3.sse.key", key("string", "with its table"))))
        self.assertEqual(kinds(found), [("additive", "storage.s3.sse.key", "key added in a new table")],
                         "[storage.s3.sse] is new though [storage.s3] is not")

    def test_a_table_named_like_a_prefix_of_another_is_its_own_table(self):
        found = cd.config_changes(CONFIG_BASE, config_changed(add_key("storage.s.x", key("string", "with its table"))))
        self.assertEqual(kinds(found), [("additive", "storage.s.x", "key added in a new table")],
                         "storage.s is not storage.s3")

    def test_a_required_key_added_to_the_table_of_a_removed_key_is_breaking(self):
        old = CONFIG_BASE
        new = config_changed(lambda d: (d["keys"].pop("storage.s3.timeout"), d["keys"].pop("storage.s3.region"),
                                        d["keys"].update({"storage.s3.zone": key("string", "with its table")})))
        found = kinds(cd.config_changes(old, new))
        self.assertIn(("breaking", "storage.s3.zone", "required key added to a table an older file may have"), found,
                      found)

    def test_a_required_key_that_changes_its_default_text_is_no_change(self):
        new = config_changed(set_key("listen", default="required"))
        self.assertEqual(cd.config_changes(CONFIG_BASE, new), [], "a required key's default is not compared")

    def test_a_key_that_became_required_reports_no_default_change(self):
        found = cd.config_changes(CONFIG_BASE, config_changed(set_key("purge_interval", required="always", default=None)))
        self.assertEqual([c.message for c in found], ["became required"], "the default goes with the requirement")

    def test_a_type_and_a_default_change_are_each_named(self):
        found = cd.config_changes(CONFIG_BASE, config_changed(set_key("purge_interval", type="string", default="`1h`")))
        self.assertEqual(sorted(c.message for c in found),
                         ["default changed from '`3600`' to '`1h`'", "type changed from 'integer' to 'string'"], found)

    def test_keys_in_another_order_are_no_change(self):
        new = {"keys": dict(reversed(list(CONFIG_BASE["keys"].items())))}
        self.assertEqual(cd.config_changes(CONFIG_BASE, new), [], "keys are compared by name")

    def test_changes_come_in_key_name_order(self):
        new = config_changed(lambda d: (d["keys"].pop("supported_critical"), d["keys"].pop("listen")))
        self.assertEqual([c.path for c in cd.config_changes(CONFIG_BASE, new)], ["listen", "supported_critical"],
                         "name order")

    def test_an_empty_default_text_is_a_default(self):
        old = config_changed(set_key("purge_interval", default=""))
        found = cd.config_changes(old, config_changed(set_key("purge_interval", default=None)))
        self.assertEqual(kinds(found), [("breaking", "purge_interval", "default '' removed")], found)


if __name__ == "__main__":
    unittest.main()
