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

## Commands

| Command | What it does |
|---|---|
| `/plugin new <name> [--kind tool\|channel\|provider\|bare] [--dir PATH]` | Scaffold `gray-<name>/`: `Cargo.toml`, `src/main.rs`, `src/lib.rs`, `README.md`, `LICENSE`, and a release workflow that produces the tarball+sha256 shape the index expects. |
| `/plugin check <dir>` | `cargo build` + `gray plugin check` in one shot. |

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
