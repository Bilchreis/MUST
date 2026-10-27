# SPDX-License-Identifier: LGPL-3.0-or-later
"""Drive the must example node with the Frappy SECoP client.

Usage: python3 frappy_client.py <host:port>

Frappy (https://github.com/SampleEnvironment/frappy) must be importable, e.g. via
PYTHONPATH pointing at a checkout. Exits non-zero on the first failed check.
"""

import sys
import time

from frappy.client import SecopClient
from frappy.errors import RangeError


def wait_for(predicate, what, timeout=10.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.1)
    raise AssertionError(f"timed out waiting for {what}")


def main(uri):
    client = SecopClient(uri)
    client.connect(try_period=10)
    print("connected:", client.nodename)

    # the description was understood
    assert "cryo" in client.modules, client.modules.keys()
    params = client.modules["cryo"]["parameters"]
    for name in ("value", "status", "target", "target_max", "ramp"):
        assert name in params, f"missing parameter {name}"
    assert "stop" in client.modules["cryo"]["commands"]
    assert client.properties["equipment_id"] == "must-example"

    # read
    value, timestamp, readerror = client.getParameter("cryo", "value")
    assert readerror is None and 0 <= value <= 400 and timestamp, (value, timestamp, readerror)
    print("value:", value)

    # change, then follow the updates until the target is reached
    target = round(value - 0.5, 3)
    client.setParameter("cryo", "target", target)
    wait_for(lambda: abs(client.cache["cryo", "value"][0] - target) < 0.02, "value to reach target")
    wait_for(lambda: client.cache["cryo", "status"][0][0] == 100, "status IDLE")
    print("reached", client.cache["cryo", "value"][0])

    # the dynamic limit (target_max = 300) is enforced
    try:
        client.setParameter("cryo", "target", 350)
    except RangeError as e:
        print("rejected as expected:", e)
    else:
        raise AssertionError("target 350 accepted despite target_max 300")

    # commands
    result, qualifiers = client.execCommand("cryo", "stop")
    assert result is None and "t" in qualifiers, (result, qualifiers)

    client.disconnect()
    print("frappy interop: OK")


if __name__ == "__main__":
    main(sys.argv[1])
