# Benchmarks

Latency and throughput measurements of must nodes, mainly through SHALL's C ABI used the way a C or LabVIEW program uses it. They are not part of CI.

- `bench_host.c`: a program that builds one node `bench` (module `m`, readable doubles `p0`…`p9` and `value`, `status`, writable `w`) through `SECoP_S_*`, in callback mode, in polling mode with a LabVIEW-style loop, or pushing values in a tight loop. See the comment at its top.
- `loadgen/`: a SECoP load client (its own Cargo project, not a workspace member).
- `run.sh`: builds both against the current checkout and runs a suite.

Linux only (`bench_host.c` uses POSIX timing, `run.sh` links `libSHALLserver64.so`). Needs a C compiler and a Rust toolchain.

## Running

```bash
bench/run.sh polling      # polling-mode latency, both loop styles (about 30 s)
bench/run.sh stall        # client latency across a 60 s slow-tier poll tick (about 2.5 min)
bench/run.sh throughput   # reads and pushes per second, log files and monitoring on and off
bench/run.sh all
```

Build output and the nodes' log files go to `bench/target/`. The polling host answers every `read` with 1.5 and every `change` with the requested value, at once. "50 ms loop" means: sleep 50 ms, then call GET CMD (`SECoP_S_getStoredCommand2`) either until it returns no data or once.

## Baseline

Measured on `audit-perf-compat` before `concurrent-polled-requests` (4 cores, release build). Use them to compare after changes to request handling; absolute numbers depend on the machine.

`polling`, median latency:

| Scenario | GET CMD until no data | GET CMD once per pass |
|---|---|---|
| 1 client, one read at a time | 50 ms | 50 ms |
| 4 clients, different parameters | 200 ms | 200 ms |
| 8 clients, same parameter | 400 ms | 400 ms |
| 1 client, same read twice in a row | 100 ms | 100 ms |

With `concurrent-polled-requests`, every row of "until no data" is expected to drop to about 50 ms. For "once per pass" the second row stays at about 200 ms, as with SHALL, because the program fetches one request per pass; the third and fourth rows drop to about 50 ms, because the reads are merged into one request.

`stall`, one client reading `m:p0` for 75 s:

| | p99 | max |
|---|---|---|
| default polling (`value`/`status` 5 s, others 60 s) | 114 ms | 728 ms |
| `pollinterval` 0 | 53 ms | 114 ms |

`throughput`:

| | Log files and monitoring on | Off |
|---|---|---|
| Rust example node, 16 clients × 8 in flight | 43k replies/s | 89k replies/s |
| C callbacks, 16 clients × 8 in flight, one module | 10.8k replies/s | 21k replies/s |
| one `SECoP_S_updateParameter2` call | 7.9 µs | 1.2 µs |

See issue #16 for the CPU hotspots behind the difference.

After issue #16, on another machine (node pinned to 4 threads, load client to 4 others; median of 5 alternating runs), before → after:

| | Log files and monitoring on | Off |
|---|---|---|
| Rust example node, 16 clients × 8 in flight | 143k → 166k replies/s | 240k → 283k replies/s |
| C callbacks, 16 clients × 8 in flight, one module | 101k → 114k replies/s | 181k → 193k replies/s |
| one `SECoP_S_updateParameter2` call | 3.6 → 2.7 µs | 0.99 → 0.42 µs |

Instructions counted with callgrind, log files and monitoring on: 66k → 41k per read reply of the Rust node, 12.9k → 8.5k per update call (including start-up and idle work, so approximate).

The fixes also help a node with both off, so on stays at about 60% of off under full load. What is left is the work of logging itself: every request makes three events, which two tasks follow and a thread writes as three lines of text, and an update wakes those tasks.
