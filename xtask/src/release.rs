// SPDX-License-Identifier: LGPL-3.0-or-later

//! The two halves of a release (docs/releasing.md): `release` opens the pull request that
//! sets the new version, `tag` tags `main` once it is merged, which starts the `Release`
//! workflow.
//!
//! Both read `main` from the public repository over HTTPS, so they need no SSH key; `tag`
//! creates the tag through the GitHub API with the `gh` login, because tags pushed by a
//! workflow's own token would not start other workflows.

use std::fs;
use std::process::Command;

use crate::{Result, cargo, root, run, run_output};

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

type Version = (u64, u64, u64);

/// `cargo xtask release <X.Y.Z|major|minor|patch>`
pub fn release(args: &[String]) -> Result<()> {
    let [wanted] = args else {
        return Err("usage: cargo xtask release <X.Y.Z|major|minor|patch>".into());
    };
    if !git(&["status", "--porcelain"])?.trim().is_empty() {
        return Err("the checkout has uncommitted changes; commit or stash them first".into());
    }
    let (main, current) = upstream_main()?;
    let new = match wanted.as_str() {
        "major" => (current.0 + 1, 0, 0),
        "minor" => (current.0, current.1 + 1, 0),
        "patch" => (current.0, current.1, current.2 + 1),
        v => parse(v.strip_prefix('v').unwrap_or(v))
            .ok_or_else(|| format!("{v} is neither X.Y.Z nor major, minor or patch"))?,
    };
    if new <= current {
        return Err(format!(
            "{} is not newer than main's version {}",
            show(new),
            show(current)
        ));
    }
    let version = show(new);
    let tag = format!("v{version}");
    if tag_exists(&tag)? {
        return Err(format!("{tag} is already tagged"));
    }

    let branch = format!("release-{tag}");
    git(&["switch", "--quiet", "--create", &branch, &main])?;
    let manifest = root().join("Cargo.toml");
    let text = fs::read_to_string(&manifest).map_err(|e| format!("Cargo.toml: {e}"))?;
    fs::write(&manifest, set_version(&text, &version)?).map_err(|e| format!("Cargo.toml: {e}"))?;
    // only the workspace's own packages change in Cargo.lock
    run(Command::new(cargo())
        .args(["update", "--workspace", "--offline", "--quiet"])
        .current_dir(root()))?;
    git(&[
        "commit",
        "--quiet",
        "--all",
        "--message",
        &format!("Release {version}"),
    ])?;
    println!("Committed version {version} on {branch}.");

    run(Command::new("git")
        .args(["push", "--quiet", "--set-upstream", "origin", &branch])
        .current_dir(root()))
    .map_err(|e| {
        format!("{e}\nretry with: git push -u origin {branch}, then open the pull request")
    })?;
    run(Command::new("gh")
        .args(["pr", "create", "--base", "main", "--head", &branch])
        .args(["--title", &format!("Release {version}")])
        .arg("--body")
        .arg(format!(
            "Sets the workspace version to {version}. Once this is merged, `cargo xtask tag` \
             tags main `{tag}`, which builds the draft release (docs/releasing.md)."
        ))
        .current_dir(root()))?;
    println!("Check the README's status list, merge the pull request, then run `cargo xtask tag`.");
    Ok(())
}

/// `cargo xtask tag`: tags upstream `main` with its version, if not tagged yet.
pub fn tag(args: &[String]) -> Result<()> {
    if !args.is_empty() {
        return Err("usage: cargo xtask tag".into());
    }
    let (main, version) = upstream_main()?;
    let tag = format!("v{}", show(version));
    if tag_exists(&tag)? {
        return Err(format!(
            "{tag} is already tagged; merge a release pull request (cargo xtask release) first"
        ));
    }
    let repo = REPOSITORY
        .strip_prefix("https://github.com/")
        .ok_or("the workspace repository is not on GitHub")?;
    run_output(
        Command::new("gh")
            .args(["api", &format!("repos/{repo}/git/refs")])
            .args([
                "-f",
                &format!("ref=refs/tags/{tag}"),
                "-f",
                &format!("sha={main}"),
            ]),
    )?;
    // the local tag, for `must update` and git log; the remote one is what counts
    let _ = git(&["tag", &tag, &main]);
    println!(
        "Tagged {} as {tag}. The Release workflow now builds the draft release \
         (gh run list -w Release); publish it once it is done.",
        &main[..12.min(main.len())]
    );
    Ok(())
}

/// The commit at the tip of upstream's `main` and the version it sets.
fn upstream_main() -> Result<(String, Version)> {
    git(&["fetch", "--quiet", "--no-tags", &upstream(), "main"])?;
    let main = git(&["rev-parse", "FETCH_HEAD"])?.trim().to_owned();
    let manifest = git(&["show", &format!("{main}:Cargo.toml")])?;
    let version = workspace_version(&manifest)
        .and_then(parse)
        .ok_or("no X.Y.Z version in [workspace.package] of main's Cargo.toml")?;
    Ok((main, version))
}

fn tag_exists(tag: &str) -> Result<bool> {
    let refs = git(&[
        "ls-remote",
        "--tags",
        &upstream(),
        &format!("refs/tags/{tag}"),
    ])?;
    Ok(!refs.trim().is_empty())
}

fn upstream() -> String {
    format!("{REPOSITORY}.git")
}

fn git(args: &[&str]) -> Result<String> {
    run_output(
        Command::new("git")
            .args(args)
            .current_dir(root())
            // fail instead of asking for credentials the public repository does not need
            .env("GIT_TERMINAL_PROMPT", "0"),
    )
}

fn parse(s: &str) -> Option<Version> {
    let mut parts = s.split('.').map(|p| p.parse().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

fn show((major, minor, patch): Version) -> String {
    format!("{major}.{minor}.{patch}")
}

/// The `version` of `[workspace.package]`, read as the Release workflow reads it.
fn workspace_version(manifest: &str) -> Option<&str> {
    let line = version_line(manifest)?;
    manifest
        .lines()
        .nth(line)?
        .strip_prefix("version = \"")?
        .strip_suffix('"')
}

fn set_version(manifest: &str, version: &str) -> Result<String> {
    let line = version_line(manifest).ok_or("no version in [workspace.package] of Cargo.toml")?;
    let mut out: Vec<String> = manifest.lines().map(str::to_owned).collect();
    out[line] = format!("version = \"{version}\"");
    let mut out = out.join("\n");
    if manifest.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

/// Index of the `version = ` line within `[workspace.package]`.
fn version_line(manifest: &str) -> Option<usize> {
    let mut in_package = false;
    for (i, line) in manifest.lines().enumerate() {
        if line.starts_with('[') {
            in_package = line.trim() == "[workspace.package]";
        } else if in_package && line.starts_with("version = ") {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = "\
[workspace]
members = [\"crates/*\"]

[workspace.package]
version = \"0.2.1\"
edition = \"2024\"

[workspace.dependencies]
cc = { version = \"1\" }
";

    #[test]
    fn reads_and_sets_the_workspace_version() {
        assert_eq!(workspace_version(MANIFEST), Some("0.2.1"));
        let set = set_version(MANIFEST, "0.3.0").unwrap();
        assert_eq!(workspace_version(&set), Some("0.3.0"));
        assert_eq!(set, MANIFEST.replace("0.2.1", "0.3.0"));
        assert!(set_version("[package]\nversion = \"1.0.0\"\n", "1.0.1").is_err());
    }

    #[test]
    fn parses_versions() {
        assert_eq!(parse("0.10.2"), Some((0, 10, 2)));
        assert_eq!(parse("1.2"), None);
        assert_eq!(parse("1.2.3.4"), None);
        assert_eq!(parse("1.2.3-rc1"), None);
    }
}
