# SPDX-License-Identifier: LGPL-3.0-or-later
"""Check a running node's description with secop-checker.

Usage: python3 secop_check.py <host:port>

secop-checker (https://github.com/SampleEnvironment/secop-checker) must be importable,
e.g. via PYTHONPATH pointing at a checkout. Prints every diagnostic and exits non-zero
if any has severity ERROR or worse; the checker's own command line exits zero for those.
"""

import sys

from secop_check import Severity, load_from_node
from secop_check.checker import Checker


def main(addr):
    version, describing = load_from_node(addr)
    print(f"node {addr} reports SECoP version {version}")
    checker = Checker(version, [], "none")
    checker.check(describing)
    diags = checker.get_diags()
    for d in diags:
        path = " / ".join(str(item) for item in d.ctx.path)
        print(f"{d.severity.name}: [{d.step}] {path}: {d.msg}")
    failed = [d for d in diags if d.severity >= Severity.ERROR]
    if failed:
        print(f"secop-checker: {len(failed)} error(s)")
        sys.exit(1)
    print(f"secop-checker: OK ({len(diags)} hints or warnings)")


if __name__ == "__main__":
    main(sys.argv[1])
