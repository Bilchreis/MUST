---
name: release-tag
description: Tag a merged MUST release pull request and see the Release workflow through to the draft GitHub release. Use after a "Release X.Y.Z" pull request is merged, when the user asks to tag a release, or to continue the release skill.
compatibility: Requires git, cargo and the GitHub CLI (gh) logged in.
---

The second half of a release (`docs/releasing.md`), after the release pull request is
merged.

## 1. Tag

```bash
cargo xtask tag
```

It tags the tip of upstream's `main` with the version of its `Cargo.toml` (`v<version>`)
through the GitHub API, which starts the `Release` workflow. Do not push the tag over
SSH instead: the API needs no SSH key, and a tag created with the `gh` login reliably
starts the workflow.

- "`v<version>` is already tagged": `main`'s version was released already. Check that
  the release pull request really is merged (`gh pr list --state merged --limit 3`);
  if it is, the tag exists and only the workflow (step 2) remains.
- Anything merged after the release pull request ends up in the tag too. If something
  other than the release pull request was merged since, tell the user before tagging.

## 2. Wait for the workflow

```bash
gh run list --workflow Release --limit 1 --json databaseId,status,headBranch
```

Wait for that run in the background until it completes, rather than polling in the
foreground. It takes a few minutes: builds and checks for Linux x86_64 and Windows
x86_64 and i686, then the draft release.

- Success: `gh release view v<version> --json isDraft,assets` should show a draft with
  three archives, `must-shall-<version>-<target>.zip`/`.tar.gz`.
- Failure: show the failing job's log (`gh run view <id> --log-failed | tail -50`) and
  stop. A fix goes into a new pull request; re-running the workflow for the same tag
  then replaces the draft's archives (`gh run rerun <id>`), as long as the release is
  not published.

## 3. Hand back

Do not publish the draft: the maintainer reviews the notes and the files and publishes
it. Report the tag, its commit, the archives and the draft's URL
(`gh release view v<version> --json url`).
