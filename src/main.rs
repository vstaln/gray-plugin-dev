//! plugin-dev — scaffold a new gray sidecar plugin and run its
//! conformance loop (`cargo build` + `plugin-dev check`).
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

mod check;
mod dev;

const NAME: &str = "plugin-dev";

fn manifest() -> Value {
    json!({
        "name": NAME,
        "display_name": "Plugin Dev",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": "1.1",
        "commands": ["/create", "/plugin-dev"],
        "command_descriptions": {
            "/create": "build a working plugin from a plain-English description",
            "/plugin-dev": "build and check a plugin"
        },
        "subcommands": ["plugin-dev"],
        "tools": [
            {
                "name": "plugin_dev_new",
                "description": "Stamp a single-file script plugin at ~/.gray/plugins-dev/gray-<name>/ (main.py or main.sh, plugin.sh, README.md) that already speaks the full Gray sidecar protocol and exposes one sample tool.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Short lowercase name: letters, digits, hyphen. The directory is gray-<name> and tools are prefixed <name>_."},
                        "lang": {"type": "string", "enum": ["python", "shell"], "description": "python (default) for a stdlib-only Python 3 sidecar, shell for POSIX sh."},
                        "description": {"type": "string", "description": "One line on what the plugin does; becomes its command description and README."}
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "plugin_dev_check",
                "description": "Run the Gray conformance check (gray plugin check) on a dev plugin by name or path and return its PASS/FAIL lines verbatim.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Plugin name (dev dir gray-<name>) or an explicit directory path."}
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "plugin_dev_install",
                "description": "Install a dev plugin live: symlink ~/.gray/plugins-dev/gray-<name> to ~/.gray/plugins/<name>, write <name>-manifest.json from the plugin's own manifest and add a lock.json entry marked dev. Refuses to overwrite a plugin that was not installed this way.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Plugin name; the dev directory must already exist."}
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "plugin_dev_call",
                "description": "Spawn a dev plugin right now and invoke one of its tools over the sidecar wire, returning that tool's output. This is how a freshly written tool gets tested inside the current session, with no restart and no hot reload.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Plugin name (dev dir gray-<name>)."},
                        "tool": {"type": "string", "description": "Tool name exactly as declared in the plugin manifest."},
                        "args": {"type": "object", "description": "Arguments object for the tool."}
                    },
                    "required": ["name", "tool"]
                }
            },
            {
                "name": "plugin_dev_fork",
                "description": "Copy an installed plugin's source repo into ~/.gray/plugins-dev so it can be modified, but only when that repo lives under ~/grayplugins; otherwise refuse and say why.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Installed plugin name."}
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "plugin_dev_uninstall",
                "description": "Remove a dev-installed plugin: drop the symlink, its manifest json and its lock entry. Refuses plugins that were not installed by plugin_dev_install.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Plugin name."}
                    },
                    "required": ["name"]
                }
            }
        ],
    })
}

const CREATE_PROMPT: &str = "Creator mode. Build a Gray plugin that does: {description}. Steps: 1) restate in one line what the plugin's tools/commands will be, pick a short name. 2) plugin_dev_new with lang python unless the task needs something Python stdlib can't do. 3) write the code in ~/.gray/plugins-dev/gray-<name>/main.py; stdlib only; keep stdout wire-only. 4) plugin_dev_check until it passes. 5) plugin_dev_call each tool with realistic arguments and confirm the output is right; fix and repeat until it is. 6) plugin_dev_install. 7) tell the user in two or three sentences what it does, how to call it, and that it is fully live after /restart (callable now through plugin_dev_call). Never touch plugins outside ~/.gray/plugins-dev except via plugin_dev_install.";

fn create_prompt(description: &str) -> String {
    CREATE_PROMPT.replace("{description}", description.trim())
}

fn run_tool(name: &str, args: &Value) -> (String, bool) {
    let text = |k: &str| args[k].as_str().unwrap_or("").trim().to_string();
    let outcome: Result<String, String> = match name {
        "plugin_dev_new" => {
            let lang = text("lang");
            dev::new(&text("name"), &lang, &text("description"))
                .map(|path| format!("scaffolded {path}"))
        }
        "plugin_dev_check" => dev::check(&text("name")),
        "plugin_dev_install" => dev::install(&text("name")),
        "plugin_dev_uninstall" => dev::uninstall(&text("name")),
        "plugin_dev_fork" => dev::fork(&text("name")),
        "plugin_dev_call" => {
            let call_args = if args["args"].is_object() { args["args"].clone() } else { json!({}) };
            return match dev::call(&text("name"), &text("tool"), call_args) {
                Ok((content, is_error)) => (content, is_error),
                Err(e) => (e, true),
            };
        }
        other => Err(format!("unknown tool {other:?}")),
    };
    match outcome {
        Ok(text) => (text, false),
        Err(e) => (e, true),
    }
}

fn run_command(name: &str, params_argv: &Value, raw: &str) -> Value {
    if name == "/create" {
        let from_argv = params_argv
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "))
            .unwrap_or_default();
        let description = match raw.trim() {
            "" => from_argv,
            r => r.to_string(),
        };
        if description.trim().is_empty() {
            return json!({"text": "usage: /create <what the plugin should do>"});
        }
        return json!({"prompt": create_prompt(&description)});
    }
    let argv: Vec<String> = params_argv
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    json!({"text": run_cli(&argv).unwrap_or_else(|e| e)})
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
        Some("-h" | "--help" | "help") => println!(
            "{{NAME}} — gray sidecar plugin. Commands: /{{NAME}} [status]. \
             `{{NAME}} manifest` prints the manifest."),
        _ => {
            if args.is_empty() {
                if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                    println!("{{NAME}} — gray sidecar plugin. `{{NAME}} --help` for usage.");
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

const LICENSE_TXT: &str = r#"MIT License

Copyright (c) gray plugin authors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
"#;

fn grayplugins_dir() -> PathBuf {
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()))
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
    std::fs::write(root.join("LICENSE"), LICENSE_TXT).map_err(|e| e.to_string())?;
    Ok(root.display().to_string())
}

fn run_check(dir: &str) -> Result<String, String> {
    let rust = Path::new(dir).join("Cargo.toml").is_file();
    let mut target = dir.to_string();
    if rust {
        let build = std::process::Command::new("cargo")
            .args(["build"])
            .current_dir(dir)
            .output()
            .map_err(|e| format!("cargo build spawn: {e}"))?;
        if !build.status.success() {
            return Err(format!(
                "cargo build failed:\n{}", String::from_utf8_lossy(&build.stderr)));
        }
        let name = Path::new(dir)
            .file_name().and_then(|s| s.to_str()).unwrap_or("")
            .trim_start_matches("gray-");
        target = format!("{dir}/target/debug/{name}");
    }
    // Conformance probe lives here now — self-spawn so the sidecar path
    // captures the PASS/FAIL report instead of sharing stdout.
    let exe = std::env::current_exe()
        .map_err(|e| format!("cannot self-spawn: {e}"))?;
    let out = std::process::Command::new(exe)
        .args(["check", &target])
        .output()
        .map_err(|e| format!("plugin-dev check spawn: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string()
        + &String::from_utf8_lossy(&out.stderr);
    Ok(format!("{target}:\n{text}"))
}

fn run_cli(argv: &[String]) -> Result<String, String> {
    match argv.first().map(String::as_str) {
        Some("new") => {
            let name = argv.get(1)
                .ok_or("usage: plugin new <name> [--lang python|shell] [--dir PATH]")?;
            let lang = argv.iter().position(|a| a == "--lang")
                .and_then(|i| argv.get(i + 1))
                .map(String::as_str)
                .unwrap_or("");
            if matches!(lang, "python" | "python3" | "py" | "shell" | "sh") {
                let description = argv.iter().position(|a| a == "--description")
                    .and_then(|i| argv.get(i + 1))
                    .map(String::as_str)
                    .unwrap_or("");
                let path = dev::new(name, lang, description)?;
                let dir = dev::dev_dir(name.trim_start_matches("gray-"));
                let check = dev::check(&dir.display().to_string())
                    .unwrap_or_else(|e| format!("check failed: {e}"));
                return Ok(format!("scaffolded {path}\n\n{check}"));
            }
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
        Some("status") | None => Ok("plugin-dev alive. /create <what it does>, /plugin-dev new <name> [--lang python|shell], /plugin-dev check <dir>".into()),
        Some("help") | Some("--help") | Some("-h") => Ok(
            "plugin-dev — build plugins from a description or scaffold+check them\n  /create <description>\n  new <name> [--lang python|shell] [--dir PATH]\n  check <dir>".into()),
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
                } else if method == "tool/call" {
                    let (content, is_error) = run_tool(
                        v["params"]["name"].as_str().unwrap_or(""),
                        &v["params"]["args"],
                    );
                    json!({"id": id, "result": {"content": content, "is_error": is_error}})
                } else if method == "command/run" {
                    json!({"id": id, "result": run_command(
                        v["params"]["name"].as_str().unwrap_or(""),
                        &v["params"]["argv"],
                        v["params"]["raw"].as_str().unwrap_or(""),
                    )})
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
            if let Some("check") = args.first().map(String::as_str) {
                let dir = args.get(1).expect("usage: plugin-dev check <dir>");
                std::process::exit(check::run(dir));
            }
            match run_cli(&args) {
                Ok(t) => println!("{t}"),
                Err(e) => { eprintln!("{e}"); std::process::exit(1); }
            }
        }
    }
}
