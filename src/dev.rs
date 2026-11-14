use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{json, Value};

use crate::check::{resolve_argv, Sidecar};

const PY_TMPL: &str = r#"#!/usr/bin/env python3
import json
import sys

NAME = "{{NAME}}"
DISPLAY = "{{DISPLAY}}"
VERSION = "0.1.0"
DESCRIPTION = "{{DESCRIPTION}}"


def manifest():
    return {
        "name": NAME,
        "display_name": DISPLAY,
        "version": VERSION,
        "protocol": "1.1",
        "tools": [
            {
                "name": NAME + "_ping",
                "description": DISPLAY + " health probe - returns pong.",
                "parameters": {"type": "object", "properties": {}},
            }
        ],
        "commands": ["/" + NAME],
        "command_descriptions": {"/" + NAME: DESCRIPTION},
    }


def handle(method, params):
    if method == "tool/call":
        tool = params.get("name") or ""
        if tool == NAME + "_ping":
            return {"result": {"content": "pong"}}
        return {"result": {"content": "unknown tool " + tool, "is_error": True}}
    if method == "command/run":
        argv = params.get("argv") or []
        verb = argv[0] if argv else "status"
        if verb == "status":
            return {"result": {"text": DISPLAY + ": alive"}}
        return {"result": {"text": "unknown verb " + str(verb)}}
    return {"error": {"code": -32601, "message": "method not found: " + method}}


def log(message):
    sys.stderr.write(NAME + ": " + message + "\n")
    sys.stderr.flush()


def sidecar():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except ValueError:
            log("dropped malformed line")
            continue
        method = message.get("method") or ""
        if "id" not in message:
            if method == "plugin/shutdown":
                return
            continue
        if method == "plugin/manifest":
            reply = {"id": message["id"], "result": manifest()}
        else:
            reply = handle(method, message.get("params") or {})
            reply["id"] = message["id"]
        sys.stdout.write(json.dumps(reply) + "\n")
        sys.stdout.flush()


def main():
    args = sys.argv[1:]
    if not args or args[0] == "sidecar":
        return sidecar()
    if args[0] == "manifest":
        print(json.dumps(manifest()))
        return
    if args[0] in ("-h", "--help", "help"):
        print(DISPLAY + " - gray plugin. NDJSON sidecar; `manifest` prints the manifest.")
        return
    log("unknown verb " + args[0])
    sys.exit(1)


if __name__ == "__main__":
    main()
"#;

const SH_TMPL: &str = r#"#!/bin/sh
name="{{NAME}}"
manifest='{{MANIFEST}}'

case "$1" in
manifest)
    printf '%s\n' "$manifest"
    exit 0
    ;;
"" | sidecar) ;;
-h | --help | help)
    printf '%s\n' "{{DISPLAY}} - gray plugin. NDJSON sidecar; \`manifest\` prints the manifest."
    exit 0
    ;;
*)
    printf '%s: unknown verb %s\n' "{{NAME}}" "$1" >&2
    exit 1
    ;;
esac

while IFS= read -r line || [ -n "$line" ]; do
    method=$(printf '%s' "$line" | sed -n 's/.*"method": *"\([^"]*\)".*/\1/p')
    id=$(printf '%s' "$line" | sed -n 's/^{"id": *\([0-9][0-9]*\),.*/\1/p')
    if [ -z "$id" ]; then
        if [ "$method" = "plugin/shutdown" ]; then
            exit 0
        fi
        continue
    fi
    case "$method" in
    plugin/manifest) reply='{"result":'"$manifest"'}' ;;
    tool/call) reply='{"result":{"content":"pong"}}' ;;
    command/run) reply='{"result":{"text":"'"$name"': alive"}}' ;;
    *) reply='{"error":{"code":-32601,"message":"method not found: '"$method"'"}}' ;;
    esac
    printf '{"id":%s,%s\n' "$id" "${reply#\{}"
done
exit 0
"#;

const README_TMPL: &str = r#"# gray-{{NAME}}

{{DISPLAY}} - Gray plugin, a single-file {{LANG}} sidecar with one sample tool.

## Protocol

NDJSON over stdio. One JSON object per line; exactly one reply per request
`id`, none for notifications, `-32601` for unknown methods, exit on
`plugin/shutdown` or stdin EOF. stdout is the wire: log to stderr only.

Implemented out of the box:

- argv `manifest` and the `plugin/manifest` wire method
- `tool/call` for `{{NAME}}_ping` (returns `pong`)
- `command/run` for `/{{NAME}}`
- `-32601` on anything else, silent notifications, EOF exit

## Dev loop

```sh
gray plugin check ~/.gray/plugins-dev/gray-{{NAME}}
```

Tool calls arrive as `{"name": ..., "args": {...}, "session": ...}` - read the
arguments from `params["args"]`. From a session, exercise a tool before
installing with `plugin_dev_call`.

Install it live with `gray plugin install ~/.gray/plugins-dev/gray-{{NAME}}`
(or `plugin_dev_install` from a session). Replace the sample tool in
`{{ENTRY}}` with what the plugin really does, and keep stdout wire-only.
"#;

fn gray_home() -> PathBuf {
    std::env::var_os("GRAY_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .or_else(|| std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()))
                .map(|h| PathBuf::from(h).join(".gray"))
        })
        .unwrap_or_else(|| PathBuf::from(".gray"))
}

pub fn plugins_dir() -> PathBuf {
    gray_home().join("plugins")
}

pub fn dev_root() -> PathBuf {
    gray_home().join("plugins-dev")
}

fn bare(name: &str) -> String {
    let n = name.trim();
    n.strip_prefix("gray-").unwrap_or(n).to_string()
}

pub fn dev_dir(name: &str) -> PathBuf {
    dev_root().join(format!("gray-{}", bare(name)))
}

fn grayplugins_root() -> PathBuf {
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()))
        .map(|h| PathBuf::from(h).join("grayplugins"))
        .unwrap_or_else(|| PathBuf::from("grayplugins"))
}

fn validate_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "invalid plugin name {name:?} - lowercase letters, digits and '-' only"
        ))
    }
}

fn display_name(name: &str) -> String {
    name.split('-')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>())
                .unwrap_or_default()
                + c.as_str()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn py_str(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ")
}

fn sh_quote(s: &str) -> String {
    s.replace('\'', "'\\''")
}

fn write_file(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(unix)]
fn make_exec(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(not(unix))]
fn make_exec(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn plugin_sh(runner: &str, entry: &str) -> String {
    format!(
        "#!/bin/sh\nhere=\"$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\"\nexec {runner} \"$here/{entry}\" \"$@\"\n"
    )
}

pub fn new(name: &str, lang: &str, description: &str) -> Result<String, String> {
    let normalized = bare(name);
    let name = normalized.as_str();
    validate_name(name)?;
    let root = dev_dir(name);
    if root.exists() {
        return Err(format!(
            "{} already exists - pick another name or delete it",
            root.display()
        ));
    }
    let display = display_name(name);
    let desc = match description.trim() {
        "" => format!("{display} sample plugin"),
        d => d.to_string(),
    };
    let lang = if lang.trim().is_empty() {
        "python"
    } else {
        lang.trim()
    };
    let (entry, runner, label) = match lang {
        "python" | "python3" | "py" => ("main.py", "python3", "Python 3 (stdlib only)"),
        "shell" | "sh" => ("main.sh", "sh", "POSIX shell"),
        other => {
            return Err(format!(
                "unknown lang {other:?} - python or shell (Rust plugins: /plugin-dev new <name>)"
            ))
        }
    };
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let text = match lang {
        "python" | "python3" | "py" => PY_TMPL
            .replace("{{NAME}}", name)
            .replace("{{DISPLAY}}", &display)
            .replace("{{DESCRIPTION}}", &py_str(&desc)),
        _ => {
            let manifest = json!({
                "name": name,
                "display_name": display,
                "version": "0.1.0",
                "protocol": "1.1",
                "tools": [{
                    "name": format!("{name}_ping"),
                    "description": format!("{display} health probe - returns pong."),
                    "parameters": {"type": "object", "properties": {}}
                }],
                "commands": [format!("/{name}")],
                "command_descriptions": {format!("/{name}"): desc},
            })
            .to_string();
            SH_TMPL
                .replace("{{NAME}}", name)
                .replace("{{DISPLAY}}", &display)
                .replace("{{MANIFEST}}", &sh_quote(&manifest))
        }
    };
    write_file(&root.join(entry), &text)?;
    let sh = root.join("plugin.sh");
    write_file(&sh, &plugin_sh(runner, entry))?;
    make_exec(&sh)?;
    write_file(
        &root.join("README.md"),
        &README_TMPL
            .replace("{{NAME}}", name)
            .replace("{{DISPLAY}}", &display)
            .replace("{{LANG}}", label)
            .replace("{{ENTRY}}", entry),
    )?;
    Ok(format!(
        "{}\n  {entry}\n  plugin.sh\n  README.md",
        root.display()
    ))
}

fn sidecar_args(manifest: &Value) -> Vec<String> {
    manifest["sidecar_args"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

fn probe_manifest(dir: &Path) -> Result<Value, String> {
    let argv = resolve_argv(dir)?;
    let out = Command::new(&argv[0])
        .arg("manifest")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("manifest probe spawn: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "`manifest` exited {:?}: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("`manifest` printed non-JSON: {e}"))
}

fn argv_for(dir: &Path) -> Result<Vec<String>, String> {
    let argv = resolve_argv(dir)?;
    let mut full = argv;
    if let Ok(manifest) = probe_manifest(dir) {
        full.extend(sidecar_args(&manifest));
    }
    Ok(full)
}

fn resolve_target(target: &str) -> PathBuf {
    if target.contains('/') || target.contains('\\') || Path::new(target).is_dir() {
        PathBuf::from(target)
    } else {
        dev_dir(target)
    }
}

pub fn check(target: &str) -> Result<String, String> {
    let target = bare(target);
    let dir = resolve_target(&target);
    if !dir.is_dir() {
        return Err(format!(
            "no plugin directory at {} - plugin_dev_new first",
            dir.display()
        ));
    }
    let dir = dir.display().to_string();
    let run = |argv: &[String]| -> Result<String, String> {
        let out = Command::new(&argv[0])
            .args(&argv[1..])
            .output()
            .map_err(|e| format!("spawn {}: {e}", argv[0]))?;
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        Ok(format!(
            "exit {}{}\n{}",
            out.status.code().unwrap_or(-1),
            if out.status.success() {
                ""
            } else {
                " (failed)"
            },
            text.trim_end()
        ))
    };
    let installed_same = std::fs::read_to_string(plugins_dir().join("plugin-dev-manifest.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["version"].as_str().map(str::to_string))
        .is_some_and(|v| v == env!("CARGO_PKG_VERSION"));
    let self_argv = || -> Result<Vec<String>, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        Ok(vec![
            exe.display().to_string(),
            "check".to_string(),
            dir.clone(),
        ])
    };
    let gray_argv = vec![
        "gray".to_string(),
        "plugin".to_string(),
        "check".to_string(),
        dir.clone(),
    ];
    if installed_same {
        if let Ok(text) = run(&gray_argv) {
            return Ok(format!("gray plugin check {dir}\n{text}"));
        }
        let text = run(&self_argv()?)?;
        return Ok(format!("plugin-dev check {dir}\n{text}"));
    }
    let text = run(&self_argv()?)?;
    Ok(format!(
        "plugin-dev check {dir} (installed plugin-dev is not this build, so the check ran in place)\n{text}"
    ))
}

fn load_lock(path: &Path) -> Result<Value, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let mut v: Value =
                serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            if !v.is_object() {
                return Err(format!("{} is not a JSON object", path.display()));
            }
            if !v.get("plugins").is_some_and(Value::is_object) {
                v["plugins"] = json!({});
            }
            if v.get("schema").is_none() {
                v["schema"] = json!(1);
            }
            Ok(v)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(json!({"schema": 1, "plugins": {}}))
        }
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn save_lock(path: &Path, lock: &Value) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(lock).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, format!("{text}\n")).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(unix)]
fn link_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}

#[cfg(windows)]
fn link_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(src, dst)
}

#[cfg(not(any(unix, windows)))]
fn link_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    copy_tree(src, dst, &[])
}

#[cfg(unix)]
fn remove_link(path: &Path) -> Result<(), String> {
    let is_link = path
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if is_link {
        std::fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))
    } else {
        std::fs::remove_dir_all(path).map_err(|e| format!("{}: {e}", path.display()))
    }
}

#[cfg(not(unix))]
fn remove_link(path: &Path) -> Result<(), String> {
    match std::fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(_) => std::fs::remove_dir_all(path).map_err(|e| format!("{}: {e}", path.display())),
    }
}

fn copy_tree(src: &Path, dst: &Path, skip: &[&str]) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if skip.iter().any(|s| name.to_string_lossy() == *s) {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        let meta = std::fs::symlink_metadata(&from)?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            copy_tree(&from, &to, skip)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

pub fn install(name: &str) -> Result<String, String> {
    let normalized = bare(name);
    let name = normalized.as_str();
    validate_name(name)?;
    let devdir = dev_dir(name);
    if !devdir.is_dir() {
        return Err(format!(
            "no dev plugin at {} - plugin_dev_new first",
            devdir.display()
        ));
    }
    let plugins = plugins_dir();
    std::fs::create_dir_all(&plugins).map_err(|e| e.to_string())?;
    let lock_path = plugins.join("lock.json");
    let mut lock = load_lock(&lock_path)?;
    let existing = lock["plugins"].get(name).cloned();
    match &existing {
        Some(e) if e.get("dev").is_none() => {
            return Err(format!(
                "refusing to overwrite plugin {name:?}: installed from {} (not a dev install) - pick another name",
                e["source"].as_str().unwrap_or("the registry")
            ))
        }
        None => {
            let link = plugins.join(name);
            if link.symlink_metadata().is_ok() {
                return Err(format!(
                    "refusing to overwrite {}: it exists and no dev lock entry owns it",
                    link.display()
                ));
            }
        }
        _ => {}
    }
    let manifest = probe_manifest(&devdir)?;
    if manifest["name"].as_str() != Some(name) {
        return Err(format!(
            "plugin manifest name {:?} does not match {name:?}",
            manifest["name"]
        ));
    }
    let link = plugins.join(name);
    if link.symlink_metadata().is_ok() {
        remove_link(&link)?;
    }
    if let Err(e) = link_dir(&devdir, &link) {
        return Err(format!(
            "cannot link {} -> {}: {e}",
            link.display(),
            devdir.display()
        ));
    }
    let mut argv = vec![link.join("plugin.sh").display().to_string()];
    argv.extend(sidecar_args(&manifest));
    let installed_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string();
    let tools: Vec<String> = manifest["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect();
    let commands: Vec<String> = manifest["commands"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c.as_str().map(str::to_string))
        .collect();
    lock["plugins"][name] = json!({
        "ecosystem": "gray-native",
        "version": manifest["version"].as_str().unwrap_or("0.0.0"),
        "hash": "",
        "source": devdir.display().to_string(),
        "argv": argv,
        "adapter_version": manifest["protocol"].as_str().unwrap_or("1"),
        "installed_at": installed_at,
        "scope": "user",
        "enabled": true,
        "granted_capabilities": [],
        "capabilities_hash": Value::Null,
        "runtime_role": Value::Null,
        "dev": devdir.display().to_string(),
    });
    let manifest_path = plugins.join(format!("{name}-manifest.json"));
    let pretty = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    write_file(&manifest_path, &format!("{pretty}\n"))?;
    save_lock(&lock_path, &lock)?;
    Ok(format!(
        "installed {name} (dev) at {}\n  manifest: {}\n  tools: {}\n  commands: {}\nLive in a new session (after /restart); testable right now with plugin_dev_call.",
        link.display(),
        manifest_path.display(),
        if tools.is_empty() { "-".into() } else { tools.join(", ") },
        if commands.is_empty() { "-".into() } else { commands.join(", ") }
    ))
}

pub fn uninstall(name: &str) -> Result<String, String> {
    let normalized = bare(name);
    let name = normalized.as_str();
    validate_name(name)?;
    let plugins = plugins_dir();
    let lock_path = plugins.join("lock.json");
    let mut lock = load_lock(&lock_path)?;
    let Some(entry) = lock["plugins"].get(name).cloned() else {
        return Err(format!("{name} is not installed"));
    };
    if entry.get("dev").is_none() {
        return Err(format!(
            "refusing: {name} is not a dev install - use `gray plugin remove {name}`"
        ));
    }
    let link = plugins.join(name);
    if link.symlink_metadata().is_ok() {
        remove_link(&link)?;
    }
    let manifest_path = plugins.join(format!("{name}-manifest.json"));
    if manifest_path.exists() {
        std::fs::remove_file(&manifest_path)
            .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    }
    if let Some(obj) = lock["plugins"].as_object_mut() {
        obj.remove(name);
    }
    save_lock(&lock_path, &lock)?;
    Ok(format!(
        "uninstalled {name} (dev) - source kept at {}",
        dev_dir(name).display()
    ))
}

pub fn call(name: &str, tool: &str, args: Value) -> Result<(String, bool), String> {
    let normalized = bare(name);
    let name = normalized.as_str();
    validate_name(name)?;
    let dir = match (dev_dir(name).is_dir(), plugins_dir().join(name).is_dir()) {
        (true, _) => dev_dir(name),
        (false, true) => plugins_dir().join(name),
        _ => {
            return Err(format!(
                "no plugin {name:?} at {} or {}",
                dev_dir(name).display(),
                plugins_dir().join(name).display()
            ))
        }
    };
    let argv = argv_for(&dir)?;
    let mut sc = Sidecar::spawn(&argv)?;
    let reply = sc.request(
        "tool/call",
        json!({"name": tool, "args": args, "session": {"id": "plugin-dev", "cwd": "."}}),
        Duration::from_secs(120),
    );
    let _ = sc.notify("plugin/shutdown", json!({"reason": "plugin_dev_call done"}));
    let reply = reply?;
    let content = match reply["content"].as_str() {
        Some(c) => c.to_string(),
        None => reply.to_string(),
    };
    let is_error = reply["is_error"].as_bool().unwrap_or(false);
    Ok((content, is_error))
}

fn repo_from_entry(entry: &Value, root: &Path) -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Some(s) = entry["source"]
        .as_str()
        .filter(|s| Path::new(s).is_absolute())
    {
        cands.push(PathBuf::from(s));
    }
    for key in ["argv", "cli_argv"] {
        if let Some(p) = entry[key]
            .as_array()
            .and_then(|a| a.first())
            .and_then(Value::as_str)
        {
            cands.push(PathBuf::from(p));
        }
    }
    for cand in cands {
        let mut cur = Some(cand.as_path());
        while let Some(c) = cur {
            if c.join("Cargo.toml").is_file() || c.join(".git").exists() {
                return c.starts_with(root).then(|| c.to_path_buf());
            }
            cur = c.parent();
        }
    }
    None
}

pub fn fork(name: &str) -> Result<String, String> {
    let normalized = bare(name);
    let name = normalized.as_str();
    validate_name(name)?;
    let plugins = plugins_dir();
    let lock = load_lock(&plugins.join("lock.json"))?;
    let entry = lock["plugins"].get(name).cloned();
    let source = entry
        .as_ref()
        .and_then(|e| e["source"].as_str())
        .unwrap_or("(not installed)")
        .to_string();
    let root = grayplugins_root();
    let direct = root.join(format!("gray-{name}"));
    let repo = entry
        .as_ref()
        .and_then(|e| repo_from_entry(e, &root))
        .or_else(|| direct.is_dir().then_some(direct));
    let Some(repo) = repo else {
        return Err(format!(
            "refusing to fork {name}: source {source:?} is not a source repo under {}",
            root.display()
        ));
    };
    let dest = dev_root().join(
        repo.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("plugin"),
    );
    if dest.exists() {
        return Err(format!(
            "{} already exists - remove it before forking {name} again",
            dest.display()
        ));
    }
    copy_tree(&repo, &dest, &["target", ".git"])
        .map_err(|e| format!("copy {} -> {}: {e}", repo.display(), dest.display()))?;
    Ok(format!(
        "forked {} -> {} (target/ and .git/ skipped)\nEdit it there; a Rust plugin needs a build before plugin_dev_check/plugin_dev_call can run it.",
        repo.display(),
        dest.display()
    ))
}
