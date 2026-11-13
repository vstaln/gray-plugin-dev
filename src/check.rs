//! `plugin-dev check DIR` — the conformance probe, moved out of gray
//! core (`plugin_check.rs`). Speaks the sidecar NDJSON protocol directly:
//! argv-manifest → spawn handshake → tool/call ×2 concurrent →
//! tool/before → command/run per declared command → notify → shutdown.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

struct Report {
    name: &'static str,
    pass: bool,
    detail: String,
}

impl Report {
    fn show(&self) {
        let mark = if self.pass { "PASS" } else { "FAIL" };
        println!("{mark} {} — {}", self.name, self.detail);
    }
}

/// `gray-plugin` resolve_argv: dir → executable argv (executable file,
/// plugin.sh, target-triple leg, or the single exec in the dir).
fn resolve_argv(dir: &Path) -> Result<Vec<String>, String> {
    let is_exec = |p: &Path| {
        let Ok(m) = std::fs::metadata(p) else {
            return false;
        };
        if !m.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            m.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            true
        }
    };
    if is_exec(dir) {
        return Ok(vec![dir.to_string_lossy().into_owned()]);
    }
    if !dir.is_dir() {
        return Err(format!("{} is not a directory (or executable)", dir.display()));
    }
    let script = dir.join("plugin.sh");
    if is_exec(&script) {
        return Ok(vec![script.to_string_lossy().into_owned()]);
    }
    let triple = if cfg!(windows) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(target_os = "macos") {
        "x86_64-apple-darwin"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64-unknown-linux-musl"
    } else {
        "x86_64-unknown-linux-musl"
    };
    let leg_dir = dir.join(triple);
    if leg_dir.is_dir() {
        if let Ok(argv) = resolve_argv(&leg_dir) {
            return Ok(argv);
        }
    }
    let mut execs = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if is_exec(&path) {
            execs.push(path);
        }
    }
    match execs.len() {
        1 => Ok(vec![execs[0].to_string_lossy().into_owned()]),
        0 => Err(format!(
            "no executable in {} (expected plugin.sh or one executable)",
            dir.display()
        )),
        _ => Err(format!(
            "ambiguous plugin dir {}: several executables, add plugin.sh",
            dir.display()
        )),
    }
}

/// One sidecar process: a writer half and a reply channel fed by a
/// reader thread (requests time out on recv_timeout).
struct Sidecar {
    child: Child,
    stdin: ChildStdin,
    replies: mpsc::Receiver<Value>,
    next_id: u64,
}

impl Sidecar {
    fn spawn(argv: &[String]) -> Result<Self, String> {
        let mut child = Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn {}: {e}", argv[0]))?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let (tx, rx) = mpsc::channel::<Value>();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(v) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if tx.send(v).is_err() {
                    break;
                }
            }
        });
        Ok(Sidecar {
            child,
            stdin,
            replies: rx,
            next_id: 0,
        })
    }

    fn write(&mut self, v: &Value) -> Result<(), String> {
        writeln!(self.stdin, "{v}").map_err(|e| e.to_string())?;
        self.stdin.flush().map_err(|e| e.to_string())
    }

    /// Send a request and wait for its matching-id reply up to `ttl`.
    fn request(&mut self, method: &str, params: Value, ttl: Duration) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        self.write(&json!({"id": id, "method": method, "params": params}))?;
        let deadline = Instant::now() + ttl;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(format!("{method} timed out"));
            }
            match self.replies.recv_timeout(left) {
                Ok(v) if v.get("id").and_then(Value::as_u64) == Some(id) => {
                    return Ok(v["result"].clone())
                }
                Ok(_) => continue,
                Err(_) => return Err(format!("{method} timed out")),
            }
        }
    }

    /// Two requests in flight; both replies must land inside `ttl`.
    fn request_pair(
        &mut self,
        method: &str,
        params: Value,
        ttl: Duration,
    ) -> Result<(Value, Value), String> {
        let a = self.next_id + 1;
        let b = self.next_id + 2;
        self.next_id = b;
        self.write(&json!({"id": a, "method": method, "params": params}))?;
        self.write(&json!({"id": b, "method": method, "params": params}))?;
        let deadline = Instant::now() + ttl;
        let (mut ra, mut rb) = (None, None);
        while ra.is_none() || rb.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(format!("{method} pair timed out"));
            }
            match self.replies.recv_timeout(left) {
                Ok(v) => match v.get("id").and_then(Value::as_u64) {
                    Some(id) if id == a => ra = Some(v["result"].clone()),
                    Some(id) if id == b => rb = Some(v["result"].clone()),
                    _ => continue,
                },
                Err(_) => return Err(format!("{method} pair timed out")),
            }
        }
        Ok((ra.unwrap(), rb.unwrap()))
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.write(&json!({"method": method, "params": params}))
    }
}

fn session() -> Value {
    json!({"id": "plugin-check", "cwd": "."})
}

fn outcome_text(o: &Value) -> String {
    if let Some(s) = o["text"].as_str() {
        return format!("{} bytes", s.len());
    }
    if o.get("terminal").is_some() {
        return format!("terminal: {}", o["terminal"]);
    }
    for k in ["say", "prompt", "model_picker", "panel", "picker"] {
        if o.get(k).is_some() {
            return format!("{k}: {}", o[k].to_string().chars().take(60).collect::<String>());
        }
    }
    format!("{}", o)
}

pub fn run(dir: &str) -> i32 {
    let mut reports: Vec<Report> = Vec::new();
    let argv = match resolve_argv(Path::new(dir)) {
        Ok(a) => a,
        Err(e) => {
            println!("FAIL resolve — {e}");
            return 1;
        }
    };
    let argv0 = argv.first().cloned().unwrap_or_default();

    // argv manifest probe (what install does).
    let probe = Command::new(&argv0)
        .arg("manifest")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let mut argv = argv;
    if let Ok(o) = &probe {
        if o.status.success() {
            if let Ok(v) = serde_json::from_slice::<Value>(&o.stdout) {
                argv.extend(
                    v["sidecar_args"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.as_str().map(str::to_string)),
                );
            }
        }
    }

    let mut sc = match Sidecar::spawn(&argv) {
        Ok(s) => s,
        Err(e) => {
            println!("FAIL spawn — {e}");
            return 1;
        }
    };
    let m = match sc.request("plugin/manifest", json!({}), Duration::from_secs(30)) {
        Ok(v) => v,
        Err(e) => {
            println!("FAIL spawn — manifest handshake: {e}");
            let _ = sc.child.kill();
            return 1;
        }
    };
    reports.push(Report {
        name: "manifest",
        pass: m["name"].as_str().is_some_and(|n| !n.trim().is_empty())
            && m["version"].as_str().is_some_and(|v| !v.trim().is_empty()),
        detail: format!(
            "name={:?} version={:?} tools={} commands={:?} protocol={:?}",
            m["name"],
            m["version"],
            m["tools"].as_array().map(Vec::len).unwrap_or(0),
            m["commands"],
            m["protocol"]
        ),
    });

    reports.push(match &probe {
        Ok(o) if o.status.success() => {
            match serde_json::from_slice::<Value>(&o.stdout) {
                Ok(v) => {
                    let name_ok =
                        v.get("name").and_then(Value::as_str) == m["name"].as_str();
                    let proto_ok = v.get("protocol").is_some();
                    Report {
                        name: "argv-manifest",
                        pass: name_ok && proto_ok,
                        detail: format!(
                            "argv manifest {} wire (name={}, protocol present={})",
                            if name_ok && proto_ok {
                                "agrees with"
                            } else {
                                "disagrees with"
                            },
                            name_ok,
                            proto_ok
                        ),
                    }
                }
                Err(e) => Report {
                    name: "argv-manifest",
                    pass: false,
                    detail: format!("manifest arg printed non-JSON: {e}"),
                },
            }
        }
        Ok(o) => Report {
            name: "argv-manifest",
            pass: false,
            detail: format!("manifest arg exited {:?} — install probe would fail", o.status.code()),
        },
        Err(e) => Report {
            name: "argv-manifest",
            pass: false,
            detail: format!("manifest arg spawn failed: {e}"),
        },
    });

    if let Some(tool) = m["tools"].as_array().and_then(|t| t.first()) {
        let name = tool["name"].as_str().unwrap_or("tool").to_string();
        let params = json!({"name": name, "args": {}, "session": session()});
        let t = Instant::now();
        let one = sc.request("tool/call", params.clone(), Duration::from_secs(10));
        reports.push(match one {
            Ok(v) => Report {
                name: "tool/call",
                pass: true,
                detail: format!(
                    "{name} → {} in {:?}",
                    v["content"].as_str().map(|s| format!("{} bytes", s.len()))
                        .unwrap_or_else(|| v.to_string()),
                    t.elapsed()
                ),
            },
            Err(e) => Report {
                name: "tool/call",
                pass: false,
                detail: format!("{name}: {e} (hang-fixture behavior)"),
            },
        });
        let both = sc.request_pair("tool/call", params, Duration::from_secs(15));
        reports.push(Report {
            name: "concurrency",
            pass: both.is_ok(),
            detail: if both.is_ok() {
                "2 concurrent tool/calls both resolved".to_string()
            } else {
                format!("concurrent tool/calls failed: {}", both.err().unwrap_or_default())
            },
        });
    }

    if m["hooks"]
        .as_array()
        .is_some_and(|h| h.iter().any(|x| x == "tool/before"))
    {
        let t = Instant::now();
        let verdict = sc.request(
            "tool/before",
            json!({"name": "bash", "args": {"command": "check-probe"}, "session": session()}),
            Duration::from_secs(15),
        );
        reports.push(match verdict {
            Ok(v) => Report {
                name: "tool/before",
                pass: true,
                detail: format!(
                    "{} resolved in {:?}",
                    v["decision"].as_str().unwrap_or("?"),
                    t.elapsed()
                ),
            },
            Err(e) => Report {
                name: "tool/before",
                pass: false,
                detail: format!("tool/before: {e} (hang-fixture behavior)"),
            },
        });
    }

    if let Some(cmds) = m["commands"].as_array() {
        for cmd in cmds.iter().filter_map(Value::as_str) {
            let t = Instant::now();
            let outcome = sc.request(
                "command/run",
                json!({"name": cmd, "argv": [], "session": session()}),
                Duration::from_secs(15),
            );
            reports.push(match outcome {
                Ok(v) if !v.is_null() => Report {
                    name: "command/run",
                    pass: true,
                    detail: format!("{cmd} → {} in {:?}", outcome_text(&v), t.elapsed()),
                },
                Ok(_) => Report {
                    name: "command/run",
                    pass: true,
                    detail: format!("{cmd} declined (empty) in {:?}", t.elapsed()),
                },
                Err(e) => Report {
                    name: "command/run",
                    pass: false,
                    detail: format!("{cmd}: {e} (hang-fixture behavior)"),
                },
            });
        }
    }

    let t = Instant::now();
    let ok = sc.notify(
        "event/notify",
        json!({"type": "turn_end", "usage": {}, "session": session()}),
    );
    reports.push(Report {
        name: "notify",
        pass: ok.is_ok() && t.elapsed() < Duration::from_secs(5),
        detail: format!("turn_end notify returned in {:?}", t.elapsed()),
    });

    let t = Instant::now();
    let _ = sc.notify("plugin/shutdown", json!({"reason": "check done"}));
    let wait_deadline = Instant::now() + Duration::from_secs(10);
    let exited = loop {
        match sc.child.try_wait() {
            Ok(Some(_)) => break true,
            Ok(None) if Instant::now() < wait_deadline => {
                thread::sleep(Duration::from_millis(50))
            }
            _ => break false,
        };
    };
    if !exited {
        let _ = sc.child.kill();
    }
    reports.push(Report {
        name: "shutdown",
        pass: exited,
        detail: format!("teardown took {:?}", t.elapsed()),
    });

    let mut failed = 0;
    for r in &reports {
        r.show();
        if !r.pass {
            failed += 1;
        }
    }
    if failed > 0 {
        eprintln!("{failed} check(s) failed");
        return 1;
    }
    0
}

#[allow(dead_code)]
fn _unused(_p: PathBuf) {}
