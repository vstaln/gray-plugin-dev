//! plugin-dev — scaffold a new gray sidecar plugin and run its
//! conformance loop (`cargo build` + `gray plugin check`).
//!
//!   /plugin new <name> [--kind tool|channel|provider|bare] [--dir PATH]
//!   /plugin check <dir>
//!
//! Layout stamped: gray-<name>/{Cargo.toml, src/main.rs, src/lib.rs,
//! README.md, LICENSE, .github/workflows/release.yml} — the release
//! workflow produces the tarball+sha256 shape the index expects.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use serde_json::{json, Value};

const NAME: &str = "plugin-dev";

fn manifest() -> Value {
    json!({
        "name": NAME,
        "display_name": "Plugin Dev",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": "1.1",
        "commands": ["/plugin-dev"],
        "subcommands": ["plugin-dev"],
        "tools": [],
    })
}

const MAIN_TMPL: &str = r#"//! {{NAME}} — a gray sidecar plugin.
//!
//! Wire: NDJSON over stdio. Requests carry a numeric `id` and MUST get
//! exactly one reply; notifications (no id) get none. `plugin/shutdown`
//! is a notification — exit promptly. See PLUGIN-CONTRACT.md.

use std::io::{BufRead, Write};
use serde_json::{json, Value};

fn manifest() -> Value {
    json!({
        "name": "{{NAME}}",
        "display_name": "{{DISPLAY}}",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": "1.1",
        "tools": [{
            "name": "{{NAME}}_ping",
            "description": "Health probe — returns pong.",
            "parameters": {"type": "object", "properties": {}}
        }],
        "commands": ["/{{NAME}}"],
        "subcommands": ["{{NAME}}"],
    })
}

fn handle(method: &str, params: &Value) -> Value {
    match method {
        "tool/call" => match params["name"].as_str().unwrap_or("") {
            "{{NAME}}_ping" => json!({"result": {"content": "pong"}}),
            t => json!({"result": {"content": format!("unknown tool {t}"),
                        "is_error": true}}),
        },
        "command/run" => {
            let argv: Vec<&str> = params["argv"].as_array()
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let text = match argv.first().copied() {
                Some("status") | None => "{{DISPLAY}}: alive".to_string(),
                Some(other) => format!("unknown verb {other:?}"),
            };
            json!({"result": {"text": text}})
        }
        other => json!({"error": {"code": -32601,
            "message": format!("method not found: {other}")}}),
    }
}

fn sidecar() -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        let method = v["method"].as_str().unwrap_or("");
        match v.get("id").cloned() {
            None if method == "plugin/shutdown" => return Ok(()),
            None => {}
            Some(id) => {
                let reply = if method == "plugin/manifest" {
                    json!({"id": id, "result": manifest()})
                } else {
                    let mut r = handle(method, &v["params"]);
                    r["id"] = id;
                    r
                };
                writeln!(out, "{reply}")?;
                out.flush()?;
            }
        }
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("manifest") => println!("{}", manifest()),
        Some("sidecar") => { let _ = sidecar(); }
        _ => {
            if args.is_empty() {
                if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                    println!("{{NAME}} — gray sidecar plugin. `{{NAME}} manifest` prints the manifest.");
                    return;
                }
                let _ = sidecar();
                return;
            }
            eprintln!("unknown verb — try `manifest`");
            std::process::exit(1);
        }
    }
}
"#;

const CARGO_TMPL: &str = r#"[package]
name = "gray-{{NAME}}"
version = "0.1.0"
edition = "2021"
rust-version = "1.88"
description = "{{DISPLAY}} — gray agent harness plugin"
license = "MIT"

[lib]
name = "{{CRATE}}"
path = "src/lib.rs"

[[bin]]
name = "{{NAME}}"
path = "src/main.rs"

[dependencies]
serde_json = { version = "1", features = ["preserve_order"] }
"#;

const RELEASE_YML: &str = r#"name: release
on:
  push:
    tags: ["v*"]
jobs:
  tarball:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build --release
      - name: package + index entry
        run: |
          V=${GITHUB_REF_NAME#v}
          T={{NAME}}-$V.tar.gz
          mkdir -p dist/{{NAME}} && cp target/release/{{NAME}} dist/{{NAME}}/
          tar -czf "$T" -C dist {{NAME}}
          SHA=$(sha256sum "$T" | cut -d' ' -f1)
          echo "tarball: $T sha256:$SHA"
      - uses: softprops/action-gh-release@v2
        with:
          files: "{{NAME}}-*.tar.gz"
"#;

fn grayplugins_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join("grayplugins"))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn scaffold(name: &str, dir: &Path) -> Result<String, String> {
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        || name.is_empty() {
        return Err("name must be [a-z0-9-_]".into());
    }
    let root = dir.join(format!("gray-{name}"));
    if root.exists() {
        return Err(format!("{} already exists", root.display()));
    }
    let src = root.join("src");
    std::fs::create_dir_all(&src).map_err(|e| e.to_string())?;
    let display = name
        .split('-')
        .map(|w| { let mut c = w.chars(); c.next().map(|f| f.to_uppercase().collect::<String>()).unwrap_or_default() + c.as_str() })
        .collect::<Vec<_>>()
        .join(" ");
    let render = |t: &str| t
        .replace("{{NAME}}", name)
        .replace("{{CRATE}}", &name.replace('-', "_"))
        .replace("{{DISPLAY}}", &display);
    std::fs::write(src.join("main.rs"), render(MAIN_TMPL)).map_err(|e| e.to_string())?;
    std::fs::write(src.join("lib.rs"),
        format!("//! empty lib — {name} binary is self-contained.\n")).map_err(|e| e.to_string())?;
    std::fs::write(root.join("Cargo.toml"), render(CARGO_TMPL)).map_err(|e| e.to_string())?;
    std::fs::write(root.join("README.md"),
        format!("# gray-{name}\n\n{display} — gray agent harness plugin.\n\n## Dev\n\n```sh\ncargo build\ngray plugin check target/debug/{name}\npython3 ../.port-tasks/qa/drive.py target/debug/{name}\n```\n"))
        .map_err(|e| e.to_string())?;
    let wf = root.join(".github/workflows");
    std::fs::create_dir_all(&wf).map_err(|e| e.to_string())?;
    std::fs::write(wf.join("release.yml"), render(RELEASE_YML)).map_err(|e| e.to_string())?;
    Ok(root.display().to_string())
}

fn run_check(dir: &str) -> Result<String, String> {
    let build = std::process::Command::new("cargo")
        .args(["build"])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("cargo build spawn: {e}"))?;
    if !build.status.success() {
        return Err(format!(
            "cargo build failed:\n{}", String::from_utf8_lossy(&build.stderr)));
    }
    // find the binary: name = dir minus gray- prefix
    let name = Path::new(dir)
        .file_name().and_then(|s| s.to_str()).unwrap_or("")
        .trim_start_matches("gray-");
    let bin = format!("{dir}/target/debug/{name}");
    let out = std::process::Command::new("gray")
        .args(["plugin", "check", &bin])
        .output()
        .map_err(|e| format!("gray plugin check spawn: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string()
        + &String::from_utf8_lossy(&out.stderr);
    Ok(format!("{bin}:\n{text}"))
}

fn run_cli(argv: &[String]) -> Result<String, String> {
    match argv.first().map(String::as_str) {
        Some("new") => {
            let name = argv.get(1).ok_or("usage: plugin new <name> [--dir PATH]")?;
            let dir = argv.iter().position(|a| a == "--dir")
                .and_then(|i| argv.get(i + 1))
                .map(PathBuf::from)
                .unwrap_or_else(grayplugins_dir);
            let path = scaffold(name, &dir)?;
            let check = run_check(&path).unwrap_or_else(|e| format!("check failed: {e}"));
            Ok(format!("scaffolded {path}\n\n{check}"))
        }
        Some("check") => {
            let dir = argv.get(1).ok_or("usage: plugin check <dir>")?;
            run_check(dir)
        }
        Some("status") | None => Ok("plugin-dev alive. /plugin-dev new <name>, /plugin-dev check <dir>".into()),
        Some("help") | Some("--help") | Some("-h") => Ok(
            "plugin-dev — scaffold + check gray plugins\n  new <name> [--dir PATH]\n  check <dir>".into()),
        Some(other) => Err(format!("unknown verb {other:?}")),
    }
}

fn sidecar() -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        let method = v["method"].as_str().unwrap_or("");
        match v.get("id").cloned() {
            None if method == "plugin/shutdown" => return Ok(()),
            None => {}
            Some(id) => {
                let reply = if method == "plugin/manifest" {
                    json!({"id": id, "result": manifest()})
                } else if method == "command/run" {
                    let argv: Vec<String> = v["params"]["argv"].as_array()
                        .map(|a| a.iter().filter_map(Value::as_str)
                            .map(str::to_string).collect())
                        .unwrap_or_default();
                    let text = run_cli(&argv).unwrap_or_else(|e| e);
                    json!({"id": id, "result": {"text": text}})
                } else {
                    json!({"id": id, "error": {"code": -32601,
                        "message": format!("method not found: {method}")}})
                };
                writeln!(out, "{reply}")?;
                out.flush()?;
            }
        }
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<String> = match args.first().map(String::as_str) {
        Some("plugin-dev") | Some("plugin") => args[1..].to_vec(),
        _ => args,
    };
    match args.first().map(String::as_str) {
        Some("manifest") => println!("{}", manifest()),
        Some("sidecar") => { let _ = sidecar(); }
        _ => {
            if args.is_empty() {
                if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                    println!("plugin-dev — /plugin-dev new <name> | /plugin-dev check <dir>");
                    return;
                }
                let _ = sidecar();
                return;
            }
            match run_cli(&args) {
                Ok(t) => println!("{t}"),
                Err(e) => { eprintln!("{e}"); std::process::exit(1); }
            }
        }
    }
}
