# Releasing

Releases carry the SHALL-named libraries for every platform as downloadable archives.
They are built by the `Release` workflow (`.github/workflows/release.yml`) when a version
tag is pushed, and land in a **draft** release that a maintainer reviews and publishes.

## Steps

1. Set the new version in `[workspace.package]` of the root `Cargo.toml` (e.g.
   `version = "0.2.0"`), update the README's status list if needed, and merge that to
   `main`.
2. Tag the merge commit with the same version, prefixed with `v`, and push the tag:

   ```bash
   git checkout main && git pull
   git tag v0.2.0
   git push origin v0.2.0
   ```

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
