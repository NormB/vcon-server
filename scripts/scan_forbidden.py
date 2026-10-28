"""Finds text that must never be committed.

Reads lines of the form "<path>\t<text>" on standard input and prints one
line per finding. Exits 1 if anything was found, else 0. The same rules
scan staged lines, staged paths and commit messages, so they live only here.

Patterns that name forbidden words are written so that this file does not
match itself, for example "(c)laude".
"""

import ipaddress
import re
import sys

AI_ATTRIBUTION = [
    ("AI tool name", re.compile(r"\b(c)laude\b", re.I)),
    ("AI company name", re.compile(r"\ba(n)thropic\b", re.I)),
    ("robot emoji", re.compile("\U0001F916")),
    ("generated-with footer", re.compile(r"\bgenerated (w)ith\b.*\b(ai|llm|assistant)\b", re.I)),
]

OWNER = [("owner's name", re.compile(r"\b(b)randinger\b", re.I))]

# The owner asked to be credited, as on sipnab.com, in the README and in
# the site's configuration, which the page footer reads. Nowhere else.
OWNER_CREDIT_PATHS = {"README.md", "website/config.toml"}

# Every id takes any number of digits: the documents keep growing, and an id
# past the first phases or past 99 is as private as the first ones.
PRIVATE_DOC_IDS = [
    # Any case, joined by - or _, so an id inside a snake_case name is
    # caught too; \b would not split _ from a letter, so the edges are
    # spelled out.
    (
        "design rule id",
        re.compile(r"(?<![A-Za-z0-9])(?:FMT|ENV|STO|ACC|AUD|API|OPS|TEN|BKD)[-_][0-9]+(?![0-9])", re.I),
    ),
    ("design decision id", re.compile(r"\bCD[0-9]+\b")),
    ("scope decision id", re.compile(r"\bD[1-9][0-9]*\b")),
    ("plan task id", re.compile(r"\bP[0-9]+\.[0-9]+\b")),
    ("private draft number", re.compile(r"\bdraft\s+[0-9]+\.[0-9]+\b", re.I)),
]

EMAIL = re.compile(r"[A-Za-z0-9._%+-]+@([A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+)")
RESERVED_DOMAINS = ("example.com", "example.net", "example.org")
RESERVED_TLDS = (".example", ".invalid", ".test", ".localhost")

PHONE_NANP = re.compile(r"(?<![\d.])(?:\+?1[\s.-]?)?\(?([2-9]\d{2})\)?[\s.-](\d{3})[\s.-](\d{4})(?![\d.])")
PHONE_E164 = re.compile(r"(?<![\w+])\+([1-9]\d{7,14})\b")

IPV4 = re.compile(r"(?<![\d.])(\d{1,3}(?:\.\d{1,3}){3})(?![\d.])")
IPV6 = re.compile(r"(?<![0-9A-Za-z:])((?:[0-9A-Fa-f]{0,4}:){2,7}[0-9A-Fa-f]{0,4})(?![0-9A-Za-z:])")
DOC_NETS_V4 = [ipaddress.ip_network(n) for n in ("192.0.2.0/24", "198.51.100.0/24", "203.0.113.0/24", "127.0.0.0/8")]
DOC_NET_V6 = ipaddress.ip_network("2001:db8::/32")


def reserved_domain(domain):
    d = domain.lower()
    return d in RESERVED_DOMAINS or d.endswith(RESERVED_TLDS) or any(d.endswith("." + r) for r in RESERVED_DOMAINS)


# The two illustrative numbers in the IETF vCon core draft's examples, which
# are kept byte for byte as test fixtures. Allowed only in those fixtures.
IETF_EXAMPLE_PHONES = {"+1234567" "8901", "+1987654" "3210"}


def is_ietf_fixture(path):
    return "/tests/fixtures/ietf/" in "/" + path


def reserved_phone(exchange, line):
    return exchange == "555" and line.startswith("01")


def personal_data(text, path=""):
    for m in EMAIL.finditer(text):
        if not reserved_domain(m.group(1)):
            yield "email address", m.group(0)
    for m in PHONE_NANP.finditer(text):
        if not reserved_phone(m.group(2), m.group(3)):
            yield "phone number", m.group(0)
    for m in PHONE_E164.finditer(text):
        digits = m.group(1)
        if is_ietf_fixture(path) and m.group(0) in IETF_EXAMPLE_PHONES:
            continue
        if not (len(digits) == 11 and digits[0] == "1" and digits[4:7] == "555" and digits[7:9] == "01"):
            yield "phone number", m.group(0)
    for m in IPV4.finditer(text):
        try:
            ip = ipaddress.IPv4Address(m.group(1))
        except ValueError:
            continue
        if str(ip) == "0.0.0.0" or any(ip in n for n in DOC_NETS_V4):
            continue
        yield "IP address", m.group(1)
    for m in IPV6.finditer(text):
        candidate = m.group(1)
        if candidate.count(":") < 2 or not re.search(r"[0-9]", candidate):
            continue
        try:
            ip = ipaddress.IPv6Address(candidate)
        except ValueError:
            continue
        if ip.is_loopback or ip in DOC_NET_V6:
            continue
        yield "IP address", candidate


def is_fixture(path):
    return "/fixtures/" in "/" + path


def scan(path, text):
    rules = AI_ATTRIBUTION if path in OWNER_CREDIT_PATHS else AI_ATTRIBUTION + OWNER
    for reason, pattern in rules:
        if pattern.search(text):
            yield reason, pattern.search(text).group(0)
    yield from personal_data(text, path)
    if not is_fixture(path):
        for reason, pattern in PRIVATE_DOC_IDS:
            m = pattern.search(text)
            if m:
                yield reason, m.group(0)


def main():
    found = False
    for raw in sys.stdin:
        path, _, text = raw.rstrip("\n").partition("\t")
        for reason, hit in scan(path, text):
            found = True
            print(f"{path}: {reason}: {hit!r}")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
