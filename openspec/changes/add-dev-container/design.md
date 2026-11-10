# Design

## Context

See proposal.md for motivation. Relevant facts about the repository and the tools:

- `rust-toolchain.toml` pins Rust 1.99 with clippy and rustfmt, and lists the Windows MSVC targets. `rustup` installs whatever the file lists, so the Windows targets get installed in the container too, but they cannot link there.
- `must-ffi-ctests` compiles C and C++ through `cc`. The launcher tests open `must-tui` in `xterm`, which CI runs under Xvfb. CI sets `MUST_TUI=0` so other tests don't open real terminals.
- `must-server` discovery binds UDP 10767 on all interfaces with `SO_REUSEPORT`/`SO_REUSEADDR` and broadcasts on every interface. Several integration tests (`events.rs`, `polling.rs`, `slow_clients.rs`) exercise it.
- The T3 runtime (`t3`, currently 0.1.6) runs headless with `t3 serve --host <addr> --port <p> --no-browser [<cwd>]`. It keeps its state under a base directory (`--base-dir`, or `T3CODE_HOME`). `t3 pair` mints a short-lived pairing token for a running server, and `t3 environment link` (or Settings → Connections in the app) links another environment.

## Goals / Non-Goals

**Goals:**
- One command starts a container whose T3 server the host T3 Code can pair with.
- Several containers run at once without sharing ports, discovery traffic, clones or build output.
- `cargo build`, `cargo test --workspace`, `cargo clippy`, `cargo fmt` and `cargo doc` behave as on the Linux CI runner.
- Rebuilding the image keeps the clone, the build cache, credentials and the T3 pairing.

**Non-Goals:**
- Building or testing the Windows targets (left to CI).
- The interop job (Frappy, secop-checker) and `cargo xtask dist`/`check` inside the container. They should work, but are not part of the acceptance checks.
- Editing from VS Code through the Dev Containers extension. `devcontainer.json` keeps that possible, but the supported workflow is T3 Code.
- Publishing a prebuilt image.

## Decisions

### Agents and T3 server inside the container
The container's main process is `t3 serve --host 0.0.0.0 --port 3773 --no-browser /workspace/must`, and the agent CLIs are installed in the image. Shell commands and file edits therefore happen in the container.
*Alternative:* agents on the host running commands through `docker exec`. That is simpler, but edits still land on the host checkout and the agents keep full access to the laptop, which removes the main reason for the container.

### Bridge networking with one published port per container
Compose uses the default bridge network and publishes only `127.0.0.1:${T3_PORT:-3773}:3773`. Binding the published port to loopback keeps the T3 server unreachable from the LAN. A second container is started with a different compose project name and `T3_PORT`, e.g. `T3_PORT=3774 docker compose -p must-b up -d`.
Discovery and test ports stay inside the container's network namespace. Broadcast inside a bridge network still reaches the container's own sockets, so the discovery tests should pass. Task 3.2 checks this.
*Alternative:* host networking by default. Every container would then need distinct T3 ports, and MUST's discovery would cross between containers and the host, because `SO_REUSEPORT` lets every container bind 10767. This is kept as an opt-in override file, `compose.host-network.yaml`, which sets `network_mode: host` and passes `--port ${T3_PORT}` to `t3 serve`.

### Own clone per container in named volumes
Compose declares these named volumes, scoped by the compose project name:
- `workspace`: the clone at `/workspace/must`
- `cargo`: `CARGO_HOME`, the registry and git checkouts
- `home`: the user's home directory, holding T3 state (`T3CODE_HOME`), T3-made worktrees, agent logins and `gh` auth

On first start the entrypoint clones `${MUST_REPO:-https://github.com/Bilchreis/MUST.git}` into `/workspace/must` if it is empty. After that it never touches the clone.
*Alternatives:* bind-mounting the host checkout. Git records worktrees by absolute path in both `.git/worktrees/*/gitdir` and each worktree's `.git` file, so worktrees made on one side break on the other unless both use the same path. Bind mounts are also slow for `target/` under rootless Podman. Mounting at the identical host path was rejected because it ties the image to one developer's directory layout.

### One shared `CARGO_TARGET_DIR`
`CARGO_TARGET_DIR` is not set, so every worktree builds into its own `target/`. Clones and worktrees live in the `workspace` and `home` volumes, so builds don't go through a bind mount.
*Alternative:* one `CARGO_TARGET_DIR` for all worktrees in a container, so dependencies compile once. This was rejected for three reasons:
- Cargo locks the build directory for the whole build, so agents in different worktrees would wait on each other.
- Cargo names workspace crates' output by their path relative to the workspace root. Worktrees with different code would therefore overwrite each other's `must-server`, `must-tui` and so on, and keep recompiling them.
- The launcher finds `must-tui` next to the running executable (`launcher.rs`). A test in one worktree could then launch a `must-tui` built from another worktree's code.

The cost is a full first build per worktree, in time and disk. If that hurts, `sccache` (a `RUSTC_WRAPPER` caching compiled dependencies by content) can be added later without changing the layout.

### Credentials at runtime
- `GH_TOKEN`, `GIT_AUTHOR_NAME`/`GIT_AUTHOR_EMAIL` (and the committer equivalents) and optional `ANTHROPIC_API_KEY`/`OPENAI_API_KEY` come from a git-ignored `.devcontainer/.env`, read by compose.
- The entrypoint runs `gh auth setup-git` so HTTPS pushes use `GH_TOKEN`.
- Interactive logins (`claude`, `codex login`) can also be done once through `docker compose exec`. They persist in the `home` volume.
- Nothing goes into the image, and the host `~/.claude` is not mounted.

### Image contents
The base is `rust:1.99-bookworm`, so rustup and the toolchain from `rust-toolchain.toml` are already present. The image adds:
- `build-essential`, `clang`, `xvfb`, `xterm`, `git`, `gh`, `nc`, `python3`
- Node.js LTS
- `@anthropic-ai/claude-code`, `@openai/codex` and `@fission-ai/openspec`, installed globally
- `cargo-deny`, and `mdbook` 0.5.4 to match CI
- the T3 runtime at a pinned version, `T3_VERSION` as a build argument

The image runs as a non-root user `dev` with UID/GID 1000, which keeps file ownership simple on both Docker and rootless Podman. `ENV MUST_TUI=0`. Xvfb is not started globally; the launcher tests that need it are run under `xvfb-run`, as documented.

### Compose plus `devcontainer.json`
`compose.yaml` is the primary entry point (`docker compose` or `podman compose`). `devcontainer.json` points at the same compose service, so editors that understand dev containers can attach to the same container.

## Risks / Trade-offs

- [T3's headless install and pairing across a container boundary is new territory, and T3 is at 0.1.x] → Pin `T3_VERSION`. Task 2.1 checks install, serve and pair before anything builds on them. If pairing needs a reachable URL other than `localhost:<port>`, document that in the guide.
- [The T3 server has full control of the container, and a published port invites remote access] → Publish on `127.0.0.1` only. Pairing tokens are short-lived (5 min default), and the link can be revoked under Connections.
- [Work exists only in the container until pushed] → The guide says to push branches, and `docker compose down -v` is called out as destroying unpushed work.
- [Each container clones, and each worktree builds from scratch] → The first build of a worktree is slow and takes several GB. Removing finished worktrees, or recreating the container, reclaims the space. `sccache` is the follow-up if build time hurts.
- [Discovery tests may rely on a real broadcast-capable interface] → The bridge interface supports broadcast. Task 3.2 runs them, and the host-network override is the fallback.

## Open Questions

- Where T3 publishes its Linux runtime for unattended installs (an install script or a release archive). This only changes the install step in the Dockerfile.
