---
name: release
description: Make a new must release, from choosing the version to the draft GitHub release. Use when the user asks to make, cut or prepare a release, or to bump the version. Opens the release pull request with `cargo xtask release`; once it is merged, continues with the release-tag skill.
compatibility: Requires git, cargo and the GitHub CLI (gh) logged in.
---

Make a release as described in `docs/releasing.md`. This skill covers the first half,
the release pull request; the `release-tag` skill covers tagging and the draft release.

## 1. Choose the version

Unless the user named one, look at what `main` has since the last release:

```bash
git fetch --quiet https://github.com/Bilchreis/MUST.git main
git log --oneline --no-merges "$(git describe --tags --abbrev=0 FETCH_HEAD)"..FETCH_HEAD
```

- Nothing since the last tag: say so and stop; there is nothing to release.
- New features or changed behaviour (new SECoP messages, data types, commands, C API
  functions): `minor`. Before 1.0 this also covers breaking changes.
- Only fixes, docs or tooling: `patch`.

Tell the user the version and the reason in one line, then go on.

## 2. Open the pull request

The checkout must be clean (`git status --porcelain` empty); if it is not, ask the user
what to do with the changes instead of stashing or discarding them.

```bash
cargo xtask release <patch|minor|major|X.Y.Z>
```

It branches `release-v<version>` off upstream's `main`, bumps `[workspace.package]` and
`Cargo.lock`, commits, pushes to `origin` and opens the pull request. If only the push
failed (GitHub's SSH auth fails now and then), retry `git push -u origin
release-v<version>` a few times, then open the pull request with `gh pr create --base main
--title "Release <version>"`.

## 3. Update the README's status list

The `## Status` list in `README.md` names the OpenSpec changes and whether each is done.
Compare it with `openspec/changes/` (open) and `openspec/changes/archive/` (done), and
with the open changes' `tasks.md` (all boxes ticked: done). If it is out of date, fix it
on the release branch, amend or add a commit, and push.

## 4. Merge

Wait for CI on the pull request. When all checks pass, merge it if the user asked for
the release to be made; if they only asked to prepare it, stop and hand back the link.
If they asked for the release to be made and CI fails, report the failure and stop.

```bash
gh pr merge <number> --merge --delete-branch
```

If `gh` reports an error such as 502 or "Merge already in progress", check `gh pr view
<number> --json state` before retrying; the merge may have gone through.

Then follow the `release-tag` skill.
