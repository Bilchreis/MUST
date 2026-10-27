// SPDX-License-Identifier: LGPL-3.0-or-later

//! Build tasks for must.
//!
//! ```text
//! cargo xtask dist  [--target TRIPLE] [--debug]   build the SHALL-named libraries into dist/TRIPLE
//! cargo xtask check [--target TRIPLE]             check their exports and dependencies
//! cargo xtask exports                             list the functions declared by include/
//! ```

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

type Result<T> = std::result::Result<T, String>;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("dist") => parse_options(&args[1..]).and_then(|o| dist(&o)),
        Some("check") => parse_options(&args[1..]).and_then(|o| check(&o)),
        Some("exports") => exports().map(|(server, variant)| {
            println!(
                "SHALLserver: {}",
                server.into_iter().collect::<Vec<_>>().join(" ")
            );
            println!(
                "SHALLvariant: {}",
                variant.into_iter().collect::<Vec<_>>().join(" ")
            );
        }),
        _ => Err("usage: cargo xtask dist|check [--target TRIPLE] [--debug] | exports".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

struct Options {
    target: String,
    release: bool,
}

fn parse_options(args: &[String]) -> Result<Options> {
    let mut options = Options {
        target: host_target()?,
        release: true,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--target" => options
                .target
                .clone_from(it.next().ok_or("--target needs a value")?),
            "--debug" => options.release = false,
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(options)
}

fn host_target() -> Result<String> {
    let out = run_output(
        Command::new(cargo())
            .args(["rustc", "-q", "--", "-vV"])
            .current_dir(root()),
    )
    .or_else(|_| run_output(Command::new("rustc").arg("-vV")))?;
    out.lines()
        .find_map(|l| l.strip_prefix("host: "))
        .map(str::to_owned)
        .ok_or_else(|| "cannot determine the host target".into())
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace")
        .to_owned()
}

fn cargo() -> String {
    env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

/// Pointer width of a target: SHALL's libraries are named `...32` or `...64`.
fn bits(target: &str) -> Result<u32> {
    let arch = target.split('-').next().unwrap_or_default();
    match arch {
        "x86_64" | "aarch64" => Ok(64),
        "i686" | "i586" => Ok(32),
        _ => Err(format!("unsupported target {target}")),
    }
}

/// The libraries: (cargo artifact name, SHALL name).
const LIBRARIES: [(&str, &str); 2] = [
    ("must_ffi", "SHALLserver"),
    ("must_ffi_variant", "SHALLvariant"),
];

fn dist(o: &Options) -> Result<()> {
    let bits = bits(&o.target)?;
    let windows = o.target.contains("windows");
    let mut rustflags = env::var("RUSTFLAGS").unwrap_or_default();
    if windows {
        // no dependency on the Visual C++ runtime DLLs
        rustflags.push_str(" -C target-feature=+crt-static");
    }
    let mut build = Command::new(cargo());
    build
        .args([
            "build",
            "-p",
            "must-ffi",
            "-p",
            "must-ffi-variant",
            "-p",
            "must-tui",
            "--target",
            &o.target,
        ])
        .env("RUSTFLAGS", rustflags.trim())
        .current_dir(root());
    if o.release {
        build.arg("--release");
    }
    run(&mut build)?;

    let artifacts =
        root()
            .join("target")
            .join(&o.target)
            .join(if o.release { "release" } else { "debug" });
    let out = dist_dir(&o.target);
    if out.exists() {
        fs::remove_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    fs::create_dir_all(out.join("include")).map_err(|e| e.to_string())?;

    let (server_exports, variant_exports) = exports()?;
    for ((artifact, shall), names) in LIBRARIES.iter().zip([&server_exports, &variant_exports]) {
        let name = format!("{shall}{bits}");
        if windows {
            copy(
                &artifacts.join(format!("{artifact}.dll")),
                &out.join(format!("{name}.dll")),
            )?;
            import_library(&o.target, &out, &name, names)?;
        } else {
            copy(
                &artifacts.join(format!("lib{artifact}.so")),
                &out.join(format!("lib{name}.so")),
            )?;
        }
    }
    // the status TUI, opened by the libraries from their own folder
    let tui = tui_name(windows);
    copy(&artifacts.join(tui), &out.join(tui))?;
    for header in headers()? {
        copy(
            &header,
            &out.join("include").join(header.file_name().expect("file")),
        )?;
    }

    println!("dist/{}:", o.target);
    let mut files: Vec<_> = walk(&out)?
        .into_iter()
        .map(|p| p.strip_prefix(&out).unwrap().display().to_string())
        .collect();
    files.sort();
    for f in files {
        println!("  {f}");
    }
    Ok(())
}

/// File name of the status TUI.
fn tui_name(windows: bool) -> &'static str {
    if windows { "must-tui.exe" } else { "must-tui" }
}

fn dist_dir(target: &str) -> PathBuf {
    root().join("dist").join(target)
}

/// Writes `name.def` and builds the import library `name.lib` with MSVC's `lib.exe`, so
/// that programs linking it load `name.dll` (cargo's own import library names the DLL
/// it built, before renaming).
fn import_library(target: &str, out: &Path, name: &str, exports: &BTreeSet<String>) -> Result<()> {
    let def = out.join(format!("{name}.def"));
    let mut text = format!("LIBRARY {name}\nEXPORTS\n");
    for e in exports {
        text.push_str("    ");
        text.push_str(e);
        text.push('\n');
    }
    fs::write(&def, text).map_err(|e| e.to_string())?;
    let machine = if bits(target)? == 32 {
        "/MACHINE:X86"
    } else {
        "/MACHINE:X64"
    };
    let mut lib = msvc_tool(target, "lib.exe")?;
    lib.arg("/NOLOGO")
        .arg(format!("/DEF:{}", def.display()))
        .arg(format!(
            "/OUT:{}",
            out.join(format!("{name}.lib")).display()
        ))
        .arg(machine);
    run(&mut lib)?;
    // lib.exe also writes an .exp file that nobody needs
    let _ = fs::remove_file(out.join(format!("{name}.exp")));
    Ok(())
}

fn msvc_tool(target: &str, tool: &str) -> Result<Command> {
    cc::windows_registry::find(target, tool)
        .ok_or_else(|| format!("{tool} not found (MSVC build tools needed)"))
}

// --- checks

fn check(o: &Options) -> Result<()> {
    let bits = bits(&o.target)?;
    let windows = o.target.contains("windows");
    let out = dist_dir(&o.target);
    let (server_exports, variant_exports) = exports()?;
    let mut problems = Vec::new();
    for ((_, shall), expected) in LIBRARIES.iter().zip([&server_exports, &variant_exports]) {
        let name = format!("{shall}{bits}");
        let file = out.join(if windows {
            format!("{name}.dll")
        } else {
            format!("lib{name}.so")
        });
        if !file.exists() {
            return Err(format!(
                "{} missing; run cargo xtask dist --target {} first",
                file.display(),
                o.target
            ));
        }
        let (exported, dependencies) = if windows {
            pe_info(&o.target, &file)?
        } else {
            elf_info(&file)?
        };

        let missing: Vec<_> = expected.iter().filter(|e| !exported.contains(*e)).collect();
        if missing.is_empty() {
            println!("{name}: all {} header functions exported", expected.len());
        } else {
            problems.push(format!(
                "{name} does not export: {}",
                missing
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let unexpected: Vec<_> = exported
            .iter()
            .filter(|e| e.starts_with("SECoP_") && !expected.contains(*e))
            .collect();
        if !unexpected.is_empty() {
            problems.push(format!(
                "{name} exports functions missing from the headers: {unexpected:?}"
            ));
        }

        let bad: Vec<_> = dependencies
            .iter()
            .filter(|d| !allowed_dependency(d, windows))
            .collect();
        println!(
            "{name}: depends on {}",
            dependencies.iter().cloned().collect::<Vec<_>>().join(", ")
        );
        if !bad.is_empty() {
            problems.push(format!("{name} depends on non-system libraries: {bad:?}"));
        }
    }
    if !out.join(tui_name(windows)).is_file() {
        problems.push(format!(
            "{} missing next to the libraries",
            tui_name(windows)
        ));
    }
    if problems.is_empty() {
        // handles cross between the two libraries as built, loaded from the dist folder
        let mut test = Command::new(cargo());
        test.args([
            "test",
            "-p",
            "must-ffi-variant",
            "--test",
            "binding",
            "--target",
            &o.target,
            "--",
            "--include-ignored",
        ])
        .env("MUST_DIST_DIR", &out)
        .current_dir(root());
        run(&mut test).map_err(|e| format!("cross-library test failed: {e}"))?;
        cross_process(o, &out)?;
        println!("check passed");
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

/// A host process for the target (`examples/c_host.rs`, e.g. 32-bit) using the dist
/// libraries is discovered and read by the monitoring client built for this machine
/// (e.g. 64-bit), and opens the status TUI found next to the libraries.
fn cross_process(o: &Options, out: &Path) -> Result<()> {
    run(Command::new(cargo())
        .args([
            "build",
            "-p",
            "must-ffi-variant",
            "--example",
            "c_host",
            "--example",
            "fake_tui",
            "--target",
            &o.target,
        ])
        .current_dir(root()))?;
    let example = |name: &str| {
        root()
            .join("target")
            .join(&o.target)
            .join("debug")
            .join("examples")
            .join(if o.target.contains("windows") {
                format!("{name}.exe")
            } else {
                name.to_owned()
            })
    };
    let exe = example("c_host");
    let mut test = Command::new(cargo());
    test.args([
        "test",
        "-p",
        "must-ffi-variant",
        "--test",
        "cross_process",
        "--",
        "--include-ignored",
        "--nocapture",
    ])
    .env("MUST_HOST_EXE", &exe)
    .env("MUST_FAKE_TUI_EXE", example("fake_tui"))
    .env("MUST_DIST_DIR", out)
    .current_dir(root());
    run(&mut test).map_err(|e| format!("cross-process monitoring test failed: {e}"))
}

fn allowed_dependency(name: &str, windows: bool) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.contains("qt") {
        return false;
    }
    if windows {
        // the C runtime is linked statically, so its DLLs must not appear
        const SYSTEM: &[&str] = &[
            "kernel32.dll",
            "ntdll.dll",
            "ws2_32.dll",
            "advapi32.dll",
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "userenv.dll",
            "secur32.dll",
            "shell32.dll",
            "ole32.dll",
            "oleaut32.dll",
            "user32.dll",
            "ntoskrnl.exe",
        ];
        SYSTEM.contains(&lower.as_str())
            || (lower.starts_with("api-ms-win-core-") && !lower.contains("crt"))
    } else {
        const SYSTEM: &[&str] = &[
            "libc.so.6",
            "libm.so.6",
            "libgcc_s.so.1",
            "libpthread.so.0",
            "libdl.so.2",
            "librt.so.1",
            "ld-linux-x86-64.so.2",
        ];
        SYSTEM.contains(&lower.as_str())
    }
}

/// Exported symbols and needed libraries of an ELF shared library.
fn elf_info(file: &Path) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
    let nm = run_output(Command::new("nm").args(["-D", "--defined-only"]).arg(file))?;
    let exported = nm
        .lines()
        .filter_map(|l| l.split_whitespace().nth(2))
        .map(str::to_owned)
        .collect();
    let readelf = run_output(Command::new("readelf").arg("-d").arg(file))?;
    let needed = readelf
        .lines()
        .filter(|l| l.contains("(NEEDED)"))
        .filter_map(|l| l.rsplit('[').next())
        .map(|s| s.trim_end_matches(']').to_owned())
        .collect();
    Ok((exported, needed))
}

/// Exported symbols and imported DLLs of a Windows DLL, via `dumpbin`.
fn pe_info(target: &str, file: &Path) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
    let exports = run_output(
        msvc_tool(target, "dumpbin.exe")?
            .arg("/NOLOGO")
            .arg("/EXPORTS")
            .arg(file),
    )?;
    // export lines look like "  ordinal hint RVA name"
    let exported = exports
        .lines()
        .filter_map(|l| {
            let parts: Vec<_> = l.split_whitespace().collect();
            (parts.len() == 4 && parts[0].parse::<u32>().is_ok()).then(|| parts[3].to_owned())
        })
        .collect();
    let deps = run_output(
        msvc_tool(target, "dumpbin.exe")?
            .arg("/NOLOGO")
            .arg("/DEPENDENTS")
            .arg(file),
    )?;
    let dependencies = deps
        .lines()
        .map(str::trim)
        .filter(|l| {
            let lower = l.to_ascii_lowercase();
            Path::new(&lower)
                .extension()
                .is_some_and(|x| x == "dll" || x == "exe")
                && !l.contains(' ')
                && !l.contains('\\')
                && !l.contains('/')
        })
        .map(str::to_owned)
        .collect();
    Ok((exported, dependencies))
}

// --- headers

fn headers() -> Result<Vec<PathBuf>> {
    let dir = root().join("include");
    let mut files: Vec<_> = fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "h"))
        .collect();
    files.sort();
    Ok(files)
}

/// The functions declared with `SHALL_EXPORT` in include/: those of SECoP-exports.h and
/// `SECoP-Variant.h` go into `SHALLserver`, those of `SECoP-Variant.h` into `SHALLvariant`.
fn exports() -> Result<(BTreeSet<String>, BTreeSet<String>)> {
    let read = |name: &str| {
        fs::read_to_string(root().join("include").join(name)).map_err(|e| format!("{name}: {e}"))
    };
    let variant = declared_functions(&read("SECoP-Variant.h")?);
    let mut server = declared_functions(&read("SECoP-exports.h")?);
    server.extend(variant.iter().cloned());
    Ok((server, variant))
}

fn declared_functions(header: &str) -> BTreeSet<String> {
    let code = strip_comments(header);
    let mut names = BTreeSet::new();
    let mut rest = code.as_str();
    while let Some(i) = rest.find("SHALL_EXPORT") {
        rest = &rest[i + "SHALL_EXPORT".len()..];
        let Some(paren) = rest.find('(') else { break };
        // the function name is the last identifier before the parenthesis
        let decl = &rest[..paren];
        if decl.contains(';') || decl.contains('#') {
            continue; // the macro definition itself, not a declaration
        }
        if let Some(name) = decl
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .rfind(|s| !s.is_empty())
            && name.starts_with("SECoP_")
        {
            names.insert(name.to_owned());
        }
    }
    names
}

fn strip_comments(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("/*") {
            rest = r.find("*/").map_or("", |i| &r[i + 2..]);
        } else if let Some(r) = rest.strip_prefix("//") {
            rest = r.find('\n').map_or("", |i| &r[i..]);
        } else {
            let c = rest.chars().next().expect("not empty");
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

// --- helpers

fn copy(from: &Path, to: &Path) -> Result<()> {
    fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| format!("copy {} -> {}: {e}", from.display(), to.display()))
}

fn walk(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            files.extend(walk(&path)?);
        } else {
            files.push(path);
        }
    }
    Ok(files)
}

fn run(cmd: &mut Command) -> Result<()> {
    let status = cmd.status().map_err(|e| format!("{cmd:?}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{cmd:?} failed: {status}"))
    }
}

fn run_output(cmd: &mut Command) -> Result<String> {
    let out = cmd.output().map_err(|e| format!("{cmd:?}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "{cmd:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_declared_functions() {
        let header = r"
// SHALL_EXPORT is used to mark exported functions
#  define SHALL_EXPORT __declspec(dllimport)
/** \brief SECoP_V_notThis(x) */
CSECoPbaseType SHALL_EXPORT * SECoP_V_create(const char* szDescription);
const char SHALL_EXPORT * SECoP_S_getErrorString(enum SECoP_S_error iError);
enum SECoP_S_error SHALL_EXPORT SECoP_S_initLibrary(QApplication *pApplication, int bGUI);
";
        let names: Vec<_> = declared_functions(header).into_iter().collect();
        assert_eq!(
            names,
            [
                "SECoP_S_getErrorString",
                "SECoP_S_initLibrary",
                "SECoP_V_create"
            ]
        );
    }

    #[test]
    fn header_function_counts() {
        let (server, variant) = exports().unwrap();
        // the exports of SHALL's own DLLs: 41 variant functions, minus the Qt-only
        // SECoP_V_printStream; 27 server functions
        assert_eq!(variant.len(), 40);
        assert_eq!(server.len(), 40 + 27);
    }
}
