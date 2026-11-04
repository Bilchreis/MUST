# Releasing

Releases carry the SHALL-named libraries for every platform as downloadable archives.
They are built by the `Release` workflow (`.github/workflows/release.yml`) when a version
tag is pushed, and land in a **draft** release that a maintainer reviews and publishes.

## Steps

Both `cargo xtask` steps need a clean checkout and the [GitHub CLI](https://cli.github.com)
(`gh`) logged in. They read `main` from the public repository over HTTPS, not from your
remote, so the release is always made from upstream's latest `main`.

1. Open the release pull request:

   ```bash
   cargo xtask release patch      # or minor, major, or a version such as 0.3.0
   ```

   This creates the branch `release-v<version>` from upstream's `main`, sets the version
   in `[workspace.package]` of the root `Cargo.toml`, updates `Cargo.lock`, commits,
   pushes the branch to `origin` and opens the pull request. It refuses versions that are
   not newer than `main`'s or that are already tagged. Update the README's status list
   on that branch if needed.
2. Once CI passes, merge the pull request, then tag the merge commit:

   ```bash
   cargo xtask tag
   ```

   This tags the tip of upstream's `main` with its version, prefixed with `v` (e.g.
   `v0.2.1`). It creates the tag through the GitHub API with your `gh` login, so it needs
   no SSH key and starts the workflow. Pushing the tag by hand (`git tag v0.2.1 &&
   git push origin v0.2.1`) works too.
3. The workflow
   - checks that the tag matches the version in `Cargo.toml` (and stops otherwise),
   - runs `cargo xtask dist` and `cargo xtask check` for `x86_64-unknown-linux-gnu`,
     `x86_64-pc-windows-msvc` and `i686-pc-windows-msvc`,
   - packs each `dist/<target>/` folder with the README, `COMPATIBILITY.md` and the
     license files into `must-shall-<version>-<target>.zip` (Windows) or `.tar.gz`
     (Linux),
   - creates a draft release `must <version>` with notes generated from the merged pull
     requests and attaches the three archives.
4. Open the draft on the repository's Releases page, check the notes and the files,
   and publish it.

## Re-running

Re-running the workflow for the same tag replaces the archives of the draft. Once a
release is published, the workflow refuses to change its files; make a new version
instead.
