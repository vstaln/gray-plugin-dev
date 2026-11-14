<div align="center">
  <img alt="gray-plugin-dev" src="assets/icon.svg" width="120" height="120" />
  <h1>gray-plugin-dev</h1>
  <p><strong>Scaffold a new gray plugin and run its conformance loop — the kit this ecosystem was built with.</strong></p>
  <p>
    <a href="https://gray.alignment.id">Website</a> ·
    <a href="https://github.com/vstaln/gray-plugin-dev">Source</a> ·
    <a href="https://github.com/vstaln/gray">gray</a>
  </p>
  <p>
    <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-1c1c20?style=flat-square&labelColor=0a0a0b" /></a>
    <a href="https://www.rust-lang.org"><img alt="Built with Rust" src="https://img.shields.io/badge/built%20with-rust-1c1c20?style=flat-square&labelColor=0a0a0b&logo=rust&logoColor=d4a373" /></a>
    <img alt="gray plugin" src="https://img.shields.io/badge/gray-plugin-1c1c20?style=flat-square&labelColor=0a0a0b&color=7aa2f7" />
  </p>
</div>

<br/>
```bash
gray plugin install plugin-dev
```

`gray-plugin-dev` is the developer's entry point to the gray plugin
ecosystem: stamp a working sidecar skeleton, then drive the
build-and-check loop until it passes the wire contract.

## Creator mode

```
/create a tool that returns the current moon phase for a date
```

`/create` opens a creator turn with a fixed recipe: name the plugin, scaffold
it, write the tool, run the conformance check, call every tool with real
arguments, install it, then report back. The whole loop needs neither a Rust
build nor a restart.

| Tool | What it does |
|---|---|
| `plugin_dev_new {name, lang, description}` | Stamps `~/.gray/plugins-dev/gray-<name>/` with a single-file `main.py` (stdlib only, `--lang shell` for POSIX sh), `plugin.sh` (`exec python3 "$here/main.py" "$@"`) and `README.md` — full protocol already implemented, one sample tool. |
| `plugin_dev_check {name}` | `gray plugin check` on the dev dir, PASS/FAIL lines verbatim. |
| `plugin_dev_call {name, tool, args}` | Spawns the dev sidecar and invokes one tool over the wire — this is how a brand-new tool is tested inside the running session. |
| `plugin_dev_install {name}` | Symlinks the dev dir to `~/.gray/plugins/<name>`, writes `<name>-manifest.json` from the plugin's own manifest, and adds a `dev`-marked `lock.json` entry. Refuses to overwrite a plugin it did not install. |
| `plugin_dev_fork {name}` | Copies an installed plugin's source repo under `~/grayplugins` into `~/.gray/plugins-dev`; refuses with the reason when the source is a release tarball. |
| `plugin_dev_uninstall {name}` | Drops the symlink, manifest json and lock entry of a dev install. |

An installed plugin is only visible to a *new* Gray process: the session reads
`plugins/<name>-manifest.json` (and `lock.json`) once at boot, so a dev plugin
becomes callable as tools and slash commands after `/restart` — until then
`plugin_dev_call` is the way to exercise it.

## Rust scaffolding

| Command | What it does |
|---|---|
| `/plugin-dev new <name> [--lang python\|shell] [--dir PATH] [--description TEXT]` | Stamps a script sidecar (default `--lang rust`: `Cargo.toml`, `src/main.rs`, `src/lib.rs`, `README.md`, `LICENSE`, release workflow producing the tarball+sha256 shape the index expects). |
| `/plugin-dev check <dir>` | `cargo build` when the dir has a `Cargo.toml`, then `gray plugin check`. |

## The stamped contract

Generated plugins implement the full sidecar protocol out of the box:

- `manifest` argv + `plugin/manifest` wire method
- NDJSON-over-stdio with stdout kept wire-only (logs go to stderr)
- exactly one reply per request `id`, silence on notifications,
  prompt EOF/shutdown exit, `-32601` on unknown methods
- a sample tool + command wired end to end so `check` passes immediately

## Build from source

```sh
cargo build --release
gray plugin check ./target/release/plugin-dev
```

## License

MIT
