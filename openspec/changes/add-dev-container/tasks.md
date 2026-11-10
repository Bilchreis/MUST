# Tasks

## 1. Image

- [ ] 1.1 Write `.devcontainer/Dockerfile` on `rust:1.99-bookworm` with the system packages, Node.js LTS, the agent CLIs, `openspec`, `cargo-deny`, `mdbook` 0.5.4, the T3 runtime at the `T3_VERSION` build argument, the user `dev` (UID/GID 1000) and `ENV MUST_TUI=0`. Verify that `docker build` and `podman build` succeed, and that in the image `rustc --version` shows 1.99 and `t3 --version`, `claude --version`, `codex --version`, `openspec --version`, `gh --version`, `cargo deny --version` and `mdbook --version` all run as `dev`.

## 2. Container, T3 server and pairing

- [ ] 2.1 Write `.devcontainer/entrypoint.sh` (clone `MUST_REPO` into an empty `/workspace/must`, `gh auth setup-git` when `GH_TOKEN` is set, then `exec t3 serve --host 0.0.0.0 --port 3773 --no-browser /workspace/must`). Also write `compose.yaml` with the bridge network, `127.0.0.1:${T3_PORT:-3773}:3773`, the `workspace`, `cargo` and `home` volumes, `T3CODE_HOME` and `env_file: .env` (optional), plus `.env.example`, and git-ignore `.devcontainer/.env`. Verify that `docker compose -p must-a up -d` clones the repository on first start and not on restart, and that the T3 server answers on the published port.
- [ ] 2.2 Pair the host T3 Code with the container (`t3 pair` through `docker compose exec`, then link from the host), start a thread in the container environment and have it run `git status` and `cargo --version` there. Verify that the thread reports the container's clone and toolchain, and that the pairing survives `docker compose down && up -d`. Then have two threads build in two worktrees at once, and verify that each builds into its own worktree's `target/` without `Blocking waiting for file lock`.
- [ ] 2.3 Add `.devcontainer/devcontainer.json` pointing at the compose service, the `dev` user and `/workspace/must`. Verify that `devcontainer up --workspace-folder .` (via `npx @devcontainers/cli`) attaches to the same service.

## 3. Builds, tests and isolation

- [ ] 3.1 In the container, run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo doc --workspace --no-deps`, `mdbook build docs` and `cargo deny check licenses`. Verify that all pass with `RUSTFLAGS=-D warnings` as in CI.
- [ ] 3.2 Run `xvfb-run cargo test --workspace` in the container on the bridge network. Verify that it passes with tests actually run (`test result: ok. N passed`, N > 0), including the discovery tests in `must-server` and the xterm launcher tests. If a discovery test fails only because of the bridge network, record why in `docs/dev-container.md` and decide with the user before changing any test.
- [ ] 3.3 Start a second container (`T3_PORT=3774 docker compose -p must-b up -d`) and run `cargo test --workspace` in both at once. Verify that both pass, that each has its own `workspace`, `cargo` and `home` volumes (`docker volume ls`), and that both T3 servers can be paired from the host at the same time.
- [ ] 3.4 Add `compose.host-network.yaml` (`network_mode: host`, `t3 serve --host 127.0.0.1 --port ${T3_PORT}`). Verify that a container started with `-f compose.yaml -f compose.host-network.yaml` serves T3 on `T3_PORT` and that `simple_node` running in it is discovered by a `must` process on the host.

## 4. Documentation

- [ ] 4.1 Write `docs/dev-container.md`: prerequisites, `.env` setup, starting a container, pairing with host T3 Code, logging agents in, running a second container, running the TUI tests under `xvfb-run`, the host-network override, why Windows targets are left to CI, and that `down -v` deletes unpushed work. Add it to `docs/SUMMARY.md` and link it from the README's *Building from source*. Verify that `mdbook build docs` passes and that a fresh container set up by following only the guide reaches a paired, working thread.

## Workflow follow-up

- Archive the change once the dev container is merged and has been used for a real change.
