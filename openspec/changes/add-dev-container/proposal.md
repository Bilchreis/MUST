# Proposal

## Why

Agents work on MUST directly on the developer's laptop, in bypass-permissions mode, with the host's toolchain, credentials and network. A mistaken command can touch anything on the machine, and several agents working at once share one set of ports and one network. A dev container gives each line of agentic work its own isolated machine with the exact toolchain CI uses, while the developer keeps driving everything from T3 Code on the host.

## What Changes

- **A dev container definition** in `.devcontainer/`: an image with Rust 1.99 (clippy, rustfmt) as pinned by `rust-toolchain.toml`, the C/C++ compilers `must-ffi-ctests` needs, `xvfb` and `xterm` for the status TUI launch tests, `gh`, `git`, Node.js (agent CLIs, the `openspec` CLI) and the T3 Code runtime. `MUST_TUI=0` is set as in CI. The definition is a `devcontainer.json` plus a compose file and works with both Docker and Podman.
- **Agents run inside the container, orchestrated from the host.** The container's main process is a headless T3 server (`t3 serve --no-browser`). The host's T3 Code links to it once as a remote environment by pairing with a token from `t3 pair` inside the container. Threads, delegates and worktrees then run in the container. The link can be revoked under Settings → Connections.
- **Several containers side by side.** Bridge networking is the default: each container has its own network stack, and only its T3 port is published, on a host port the developer chooses per container. MUST's UDP discovery (port 10767, shared with `SO_REUSEPORT`) and every test port therefore stay inside one container. Host networking is an opt-in override, only for reaching real SECoP hardware on the lab network.
- **The container's own clone.** The repository is cloned into a named volume inside the container rather than bind-mounting the host checkout, because git records worktrees by absolute path and the agents create worktrees. Work leaves the container as pushed branches and pull requests. Each worktree builds into its own `target/`, as cargo does by default, and the cargo registry lives in a named volume.
- **Credentials at runtime only.** Agent authentication (Claude Code, Codex), the GitHub token and the git identity are passed in when the container starts or set up by logging in inside it, and persist in a named volume. Nothing is baked into the image, and the host's `~/.claude` is not mounted.
- **Documentation:** `docs/dev-container.md` in the guide covers starting a container, pairing it with host T3 Code, running a second one, and opting in to host networking.

Windows targets are not built in the container. The `*-pc-windows-msvc` targets need MSVC to link, and CI already builds and tests them on Windows runners.

## Capabilities

### New Capabilities

None. This is developer tooling: it changes no behaviour of the libraries, `must` or `must-tui`, so the change sets `skip_specs: true`.

### Modified Capabilities

None.

## Impact

- New files: `.devcontainer/` (Dockerfile, `devcontainer.json`, compose file, entrypoint), `docs/dev-container.md`, plus an entry in `docs/SUMMARY.md` and a pointer in the README's *Building from source* section.
- No changes to crates, CI workflows or release tooling.
- Developers need Docker or Podman on the host. The `devcontainer` CLI and VS Code are optional.
