# SPDX-License-Identifier: LGPL-3.0-or-later
"""Share UDP port 10767 with a Frappy node, and be found by Frappy's frappy-scan.

Usage: python3 frappy_discovery.py <simple_node binary> <frappy checkout>

Starts must node A, Frappy's demo cryo node and must node B on one host, so that each
joins a port that the other implementation already holds. The dependencies of
frappy-server must be installed (the checkout's requirements.txt), and the checkout needs
a git tag or a frappy/RELEASE-VERSION file for Frappy's version. Needs a host where
broadcasts work, UDP port 10767 not held by another user, and TCP port 10769 free. Other
nodes on the network may answer too; they are ignored. Exits non-zero on the first
failed check.
"""

import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

# from the node section of Frappy's cfg/cryo_cfg.py
FRAPPY_ID = "cryo_7.frappy.demo"
FRAPPY_PORT = 10769
MUST_ID = "must-example"
ANSWER = re.compile(r"Found (\S+) at \S+:\n  Port: (\d+)\n  Firmware: (.*)\n")


def wait_for(predicate, what, timeout=20.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(0.2)
    raise AssertionError(f"timed out waiting for {what}")


class Interop:
    def __init__(self, node_binary, frappy, scratch):
        self.node_binary = node_binary
        self.frappy = frappy
        self.scratch = scratch
        self.processes = []

    def start_must_node(self):
        # to a file: the node logs to stdout too, and a pipe nobody reads would block it
        path = self.scratch / f"must-{len(self.processes)}.out"
        with path.open("w") as out:
            process = subprocess.Popen([self.node_binary, "0.0.0.0:0"], stdout=out)
        self.processes.append(process)
        match = wait_for(
            lambda: re.search(r"^listening on \S+:(\d+)$", path.read_text(), re.MULTILINE),
            "the node to report its TCP port",
        )
        return process, int(match[1])

    def start_frappy_node(self):
        env = dict(os.environ)
        for name in ("CONFDIR", "LOGDIR", "PIDDIR"):
            env[f"FRAPPY_{name}"] = str(self.scratch)
        self.processes.append(
            subprocess.Popen(
                [
                    sys.executable,
                    str(self.frappy / "bin" / "frappy-server"),
                    "-c",
                    str(self.frappy / "cfg" / "cryo_cfg.py"),
                    "cryo",
                ],
                env=env,
                cwd=self.scratch,
            )
        )

    def scan(self):
        """What frappy-scan finds, as {(equipment_id, port): firmware}."""
        output = subprocess.run(
            [sys.executable, str(self.frappy / "bin" / "frappy-scan")],
            capture_output=True,
            text=True,
            check=True,
            timeout=30,
        ).stdout
        return {(m[1], int(m[2])): m[3] for m in ANSWER.finditer(output)}

    def start_listener(self):
        """Starts frappy-scan --listen; returns the file that it writes its findings to."""
        path = self.scratch / "listen.out"
        with path.open("w") as out:
            self.processes.append(
                subprocess.Popen(
                    [sys.executable, "-u", str(self.frappy / "bin" / "frappy-scan"), "--listen"],
                    stdout=out,
                )
            )
        # it scans for one second before it starts to listen
        time.sleep(2)
        return path

    def stop_all(self):
        for process in self.processes:
            process.terminate()
        for process in self.processes:
            process.wait(timeout=10)


def run(interop):
    node_a, port_a = interop.start_must_node()
    print("must node A on TCP port", port_a)
    wait_for(lambda: (MUST_ID, port_a) in interop.scan(), "frappy-scan to find must node A")

    # Frappy joins the port that must holds. Without sharing, its bind would fail.
    interop.start_frappy_node()
    wait_for(
        lambda: (FRAPPY_ID, FRAPPY_PORT) in interop.scan(), "frappy-scan to find the Frappy node"
    )
    print("Frappy node joined")

    # must joins the port that Frappy holds, and announces itself to a listener that was
    # there first: frappy-scan's scan has ended, so only the announcement can report it.
    listened = interop.start_listener()
    _, port_b = interop.start_must_node()
    print("must node B on TCP port", port_b)
    announced = f"{MUST_ID} "
    wait_for(
        lambda: any(
            line.startswith(announced) and line.endswith(f":{port_b}")
            for line in listened.read_text().splitlines()
        ),
        "frappy-scan --listen to report must node B's announcement",
    )
    print("announcement of must node B seen")

    # everyone answers a single broadcast
    found = wait_for(
        lambda: (
            lambda scan: scan
            if {(MUST_ID, port_a), (MUST_ID, port_b), (FRAPPY_ID, FRAPPY_PORT)} <= scan.keys()
            else None
        )(interop.scan()),
        "one scan to find all three nodes",
    )
    assert found[MUST_ID, port_a].startswith("must-"), found
    assert found[MUST_ID, port_b].startswith("must-"), found
    assert found[FRAPPY_ID, FRAPPY_PORT].startswith("FRAPPY"), found
    print("one scan found all three nodes")

    # a node that stops leaves the others in the group answering
    node_a.terminate()
    node_a.wait(timeout=10)
    found = interop.scan()
    assert (MUST_ID, port_a) not in found, found
    assert (MUST_ID, port_b) in found, found
    assert (FRAPPY_ID, FRAPPY_PORT) in found, found
    print("frappy discovery interop: OK")


def main(node_binary, frappy):
    with tempfile.TemporaryDirectory() as scratch:
        interop = Interop(str(Path(node_binary).resolve()), Path(frappy).resolve(), Path(scratch))
        try:
            run(interop)
        finally:
            interop.stop_all()


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
