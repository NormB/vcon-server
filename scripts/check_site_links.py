"""Checks the links of a built site without fetching anything.

Every internal link (relative, root-relative, or an absolute URL on the
site's own domain) in an href or src must name a file the build wrote, and
every #fragment must name an id (or an <a name>) on the page it points to.
The problem types that openapi.json lists must be exactly the built pages
under problems/, each at <base>/problems/<name>.

`zola check` covers the Markdown links written as @/ paths; this covers what
it cannot see: links typed into templates or as URLs, and every anchor.
Links to other sites are left alone, so no third-party page decides whether
a change passes.

Run with: python3 -I scripts/check_site_links.py <built site> <base URL>
Exits 1 and lists each broken link when there is one.
"""

import html.parser
import json
import pathlib
import posixpath
import sys
import urllib.parse

# The attributes that load or link to another resource.
LINK_ATTRIBUTES = {"href", "src"}


class Page(html.parser.HTMLParser):
    """The ids an HTML page defines and the links it holds."""

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.ids = set()
        self.links = []

    def handle_starttag(self, tag, attrs):
        for name, value in attrs:
            if value is None:
                continue
            if name == "id" or (tag == "a" and name == "name"):
                self.ids.add(value)
            elif name in LINK_ATTRIBUTES:
                self.links.append(value)

    handle_startendtag = handle_starttag


def read_page(path):
    parser = Page()
    parser.feed(path.read_text(encoding="utf-8"))
    parser.close()
    return parser


def site_path(base, page_rel, link):
    """The site-relative path a link names and its fragment, or None for
    a link to another site or by another scheme. The path is "" for the
    page itself and may begin with "../" when it leaves the site."""
    parts = urllib.parse.urlsplit(link)
    if parts.scheme or parts.netloc:
        own = urllib.parse.urlsplit(base)
        if (parts.scheme or own.scheme, parts.netloc) != (own.scheme, own.netloc):
            return None
    path = urllib.parse.unquote(parts.path)
    fragment = urllib.parse.unquote(parts.fragment)
    if not path:
        return page_rel, fragment
    if path.startswith("/"):
        joined = path.lstrip("/")
    else:
        joined = posixpath.join(posixpath.dirname(page_rel), path)
    trailing = path.endswith("/")
    normal = posixpath.normpath(joined) if joined else "."
    if normal == ".":
        normal = ""
    if trailing and normal:
        normal += "/"
    return normal, fragment


def target_file(site, rel):
    """The built file a site-relative path is served from, or None."""
    if rel.startswith("../") or rel == "..":
        return None
    candidate = site / rel
    if rel == "" or rel.endswith("/") or candidate.is_dir():
        candidate = candidate / "index.html"
    return candidate if candidate.is_file() else None


def check_links(site, base, counts):
    problems = []
    pages = {}

    def page_of(path):
        if path not in pages:
            pages[path] = read_page(path)
        return pages[path]

    for html_path in sorted(site.rglob("*.html")):
        page_rel = html_path.relative_to(site).as_posix()
        for link in page_of(html_path).links:
            resolved = site_path(base, page_rel, link)
            if resolved is None:
                continue
            rel, fragment = resolved
            counts["links"] += 1
            target = html_path if rel == page_rel else target_file(site, rel)
            if target is None:
                problems.append(f"{page_rel}: {link}: no such file in the build")
                continue
            if not fragment:
                continue
            counts["fragments"] += 1
            if target.suffix != ".html":
                problems.append(f"{page_rel}: {link}: a fragment on a file that is not a page")
            elif fragment not in page_of(target).ids:
                where = target.relative_to(site).as_posix()
                problems.append(f"{page_rel}: {link}: no id {fragment!r} on {where}")
    return problems


def check_problem_types(site, base, counts):
    openapi = site / "openapi.json"
    try:
        types = json.loads(openapi.read_text(encoding="utf-8"))["components"]["schemas"]["Problem"]["properties"]["type"]["enum"]
    except (OSError, ValueError, KeyError, TypeError) as e:
        return [f"openapi.json: no problem type list ({type(e).__name__}: {e})"]
    problems = []
    prefix = base.rstrip("/") + "/problems/"
    typed = set()
    for url in types:
        name = url[len(prefix):] if isinstance(url, str) and url.startswith(prefix) else ""
        if not name or "/" in name:
            problems.append(f"openapi.json: problem type {url} is not a page under {prefix}")
            continue
        typed.add(name)
        counts["types"] += 1
        if not (site / "problems" / name / "index.html").is_file():
            problems.append(f"openapi.json: problem type {url} has no built page problems/{name}/index.html")
    built = {p.parent.name for p in (site / "problems").glob("*/index.html")}
    for name in sorted(built - typed):
        problems.append(f"problems/{name}/index.html: a problem page whose type openapi.json does not list")
    return problems


def check(site, base, counts=None):
    """Every problem found, as one line each. `counts`, if given, gets
    the number of internal links, fragments and problem types checked."""
    site = pathlib.Path(site)
    if counts is None:
        counts = {}
    for key in ("links", "fragments", "types"):
        counts.setdefault(key, 0)
    return check_links(site, base, counts) + check_problem_types(site, base, counts)


def main(argv):
    if len(argv) != 3:
        print("usage: check_site_links.py <built site> <base URL>", file=sys.stderr)
        return 2
    counts = {}
    problems = check(argv[1], argv[2], counts)
    for problem in problems:
        print(f"check_site_links: {problem}", file=sys.stderr)
    if problems:
        return 1
    print(f"check_site_links: ok ({counts['links']} internal links, {counts['fragments']} with a fragment, {counts['types']} problem types)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
