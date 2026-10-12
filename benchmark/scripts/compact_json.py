"""Write a JSON document without its optional whitespace.

Usage: compact_json.py INPUT OUTPUT

Removes every space, tab, carriage return and newline that lies outside a
string, and nothing else: string contents and the spelling of every number
are copied byte for byte, so a document that is already compact comes out
identical. A JSON parser and serialiser (``jq -c``, ``json.dumps``) would also
drop the whitespace, but it rewrites numbers on the way (``1e-10`` becomes
``1E-10`` in jq 1.7, ``1.50`` becomes ``1.5`` in Python), so the measured
document would differ from the published one by more than its whitespace.

The input is assumed to be valid JSON; the prepare chain checks the result
with ``jq`` afterwards.
"""

import re
import sys

# A whole string token (kept), or a run of insignificant whitespace (dropped).
# Bytes, not text: a UTF-8 continuation byte is never a quote or a backslash,
# so multi-byte characters inside strings pass through untouched.
_TOKEN = re.compile(rb'("(?:[^"\\]|\\.)*")|[ \t\n\r]+', re.DOTALL)


def compact(data: bytes) -> bytes:
    """Return ``data`` with the whitespace outside strings removed."""
    return _TOKEN.sub(rb"\1", data)


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print("usage: compact_json.py INPUT OUTPUT", file=sys.stderr)
        return 2
    with open(argv[1], "rb") as source:
        data = source.read()
    with open(argv[2], "wb") as target:
        target.write(compact(data))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
