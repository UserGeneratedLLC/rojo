# Atlas Development Guide

Atlas is a Rust CLI + Roblox Studio Lua plugin that bridges filesystem and Roblox Studio. See `README.md` for user-facing docs and `.cursor/rules/atlas.mdc` for architecture details.

## Cursor Cloud specific instructions

### Prerequisites

- **Rust >= 1.88** (MSRV). Install via `rustup install 1.88.0 && rustup default 1.88.0`.
- **Git submodules** must be initialized: `git submodule update --init --recursive`. Build will fail without this (rbx-dom crates are local path dependencies).
- **Rokit** for Lua tooling (selene, stylua). Install via `curl -sSf https://raw.githubusercontent.com/rojo-rbx/rokit/main/scripts/install.sh | bash`, then `rokit trust` all tools and `rokit install` in the repo root. Ensure `~/.rokit/bin` is on PATH.

### Build / Test / Lint

Standard commands from `Cargo.toml` and CI (`.github/workflows/ci.yml`):

| Task | Command |
|------|---------|
| Build | `cargo build --locked` |
| Test | `cargo test --locked` |
| Rust format check | `cargo fmt -- --check` |
| Rust lint | `cargo clippy` |
| Lua format check | `stylua --check plugin/src` |
| Lua lint | `selene plugin/src` |

### Gotchas

- **Clippy warnings**: The codebase has `uninlined_format_args` warnings throughout — these are non-blocking style warnings, not errors. Do not attempt to fix them unless explicitly asked.
- **No external services needed**: Atlas is fully self-contained. No Docker, databases, or network services are required for build/test.
- **Test fixtures** live in `rojo-test/` (build-tests, serve-tests, syncback-tests). Snapshot tests use the `insta` crate — review changes with `cargo insta review`.
- **Plugin tests** require Roblox Studio (`run-in-roblox`) which cannot run in headless/CI Linux environments.
- The atlas binary is at `target/debug/atlas` (dev) or `target/release/atlas` (release). Core hello-world: `atlas init --kind place` then `atlas build . -o out.rbxl`.
