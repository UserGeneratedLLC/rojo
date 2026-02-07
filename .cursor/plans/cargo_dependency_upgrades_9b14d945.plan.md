---
name: Cargo Dependency Upgrades
overview: Upgrade all 11 incompatible cargo dependencies across the rojo workspace and the rbx-dom submodule, organized into tiers by risk and complexity.
todos:
  - id: tier1-rand
    content: Upgrade rand 0.8 -> 0.9 in rbx_types (thread_rng -> rng rename)
    status: completed
  - id: tier1-bitflags
    content: Upgrade bitflags 1.x -> 2.x in rbx_types (faces.rs, axes.rs macro syntax)
    status: completed
  - id: tier2-winreg
    content: Upgrade winreg 0.10 -> 0.55 in rojo (auth_cookie.rs, winapi -> windows-sys)
    status: completed
  - id: tier2-serde-yaml
    content: Upgrade serde_yaml 0.8 -> 0.9 in rojo, rojo-insta-ext, rbx_util, rbx_reflector. Run cargo insta review for snapshot diffs.
    status: completed
  - id: tier3-criterion
    content: Upgrade criterion 0.3/0.5 -> 0.8 in rojo and rbx_binary benchmarks
    status: completed
  - id: tier3-xml-rs
    content: Upgrade xml-rs 0.8 -> 1.0 in rbx_xml (deserializer_core.rs, serializer_core.rs)
    status: completed
  - id: tier3-reqwest
    content: Upgrade reqwest 0.11 -> 0.13 in rojo and rbx_reflector (blocking feature still supported)
    status: completed
  - id: tier4-bincode
    content: Upgrade bincode 1.x -> 3.0 in rojo and rbx_types (build.rs embedding, runtime deserialize, config::legacy for compat)
    status: completed
  - id: tier4-hyper
    content: Upgrade hyper 0.14 -> 1.x + hyper-tungstenite 0.11 -> 0.19. Add hyper-util, http-body-util. Rewrite server setup and body handling.
    status: completed
  - id: final-verify
    content: Run cargo test, cargo clippy, cargo fmt --check across entire workspace to verify
    status: completed
isProject: false
---

# Cargo Incompatible Dependency Upgrade Plan

## Inventory

11 packages across 8 `Cargo.toml` files need incompatible (semver-breaking) upgrades:


| Package           | From    | To     | Location(s)                                   | Risk        |
| ----------------- | ------- | ------ | --------------------------------------------- | ----------- |
| rand              | 0.8.5   | 0.9.2  | rbx_types                                     | Low         |
| bitflags          | 1.3.2   | 2.10.0 | rbx_types                                     | Low         |
| winreg            | 0.10.1  | 0.55.0 | rojo (Windows)                                | Low         |
| serde_yaml        | 0.8.26  | 0.9.34 | rojo, rojo-insta-ext, rbx_util, rbx_reflector | Low         |
| criterion         | 0.3/0.5 | 0.8.2  | rojo, rbx_binary                              | Medium      |
| xml-rs            | 0.8.28  | 1.0.0  | rbx_xml                                       | Medium      |
| reqwest           | 0.11.27 | 0.13.2 | rojo, rbx_reflector                           | Medium      |
| bincode           | 1.3.3   | 3.0.0  | rojo, rbx_types                               | Medium-High |
| hyper             | 0.14.32 | 1.8.1  | rojo                                          | High        |
| hyper-tungstenite | 0.11.1  | 0.19.0 | rojo                                          | High        |


**Note:** 6 of these packages live in the `rbx-dom/` submodule (separate git repo). Those changes must be committed to the submodule first, then the submodule pointer updated in rojo.

---

## Tier 1 -- Low Risk (rbx-dom submodule)

### 1a. rand 0.8 to 0.9

- **File:** [rbx-dom/rbx_types/Cargo.toml](rbx-dom/rbx_types/Cargo.toml)
- **Code change:** [rbx-dom/rbx_types/src/unique_id.rs](rbx-dom/rbx_types/src/unique_id.rs) -- 1 file, ~2 lines
- `thread_rng()` renamed to `rng()` in rand 0.9. `gen_range(0..i64::MAX)` syntax remains valid.

### 1b. bitflags 1.x to 2.x

- **File:** [rbx-dom/rbx_types/Cargo.toml](rbx-dom/rbx_types/Cargo.toml)
- **Code changes:** [rbx-dom/rbx_types/src/faces.rs](rbx-dom/rbx_types/src/faces.rs) and [rbx-dom/rbx_types/src/axes.rs](rbx-dom/rbx_types/src/axes.rs) -- 2 files
- bitflags 2.x keeps the `bitflags!` macro but struct visibility syntax changes slightly. Manual serde impls and `.bits()` / `.from_bits()` remain but some trait impls (like `Hash`) are now auto-derived.

---

## Tier 2 -- Low Risk (rojo main + crates)

### 2a. winreg 0.10 to 0.55

- **File:** [Cargo.toml](Cargo.toml) (Windows target dep)
- **Code change:** [src/auth_cookie.rs](src/auth_cookie.rs) -- 1 file, ~6 lines
- Core API (`RegKey::predef`, `open_subkey`, `get_value`) is preserved. Main change is the backend switched from `winapi` to `windows-sys`.

### 2b. serde_yaml 0.8 to 0.9

- **Files:**
  - [Cargo.toml](Cargo.toml) (dev-deps)
  - [crates/rojo-insta-ext/Cargo.toml](crates/rojo-insta-ext/Cargo.toml)
  - [rbx-dom/rbx_util/Cargo.toml](rbx-dom/rbx_util/Cargo.toml)
  - [rbx-dom/rbx_reflector/Cargo.toml](rbx-dom/rbx_reflector/Cargo.toml)
- **Code changes:** [crates/rojo-insta-ext/src/redaction_map.rs](crates/rojo-insta-ext/src/redaction_map.rs), [rbx-dom/rbx_util/src/view_binary.rs](rbx-dom/rbx_util/src/view_binary.rs), [rbx-dom/rbx_reflector/src/patches.rs](rbx-dom/rbx_reflector/src/patches.rs) -- 3 files
- serde_yaml 0.9 renames `Value::Mapping` internals and changes error types. The basic `from_str`, `to_writer`, `to_value` APIs are largely the same. May also affect insta snapshot test output formatting.

---

## Tier 3 -- Medium Risk

### 3a. criterion 0.3/0.5 to 0.8

- **Files:**
  - [Cargo.toml](Cargo.toml) (dev-deps: `0.3.6`)
  - [rbx-dom/rbx_binary/Cargo.toml](rbx-dom/rbx_binary/Cargo.toml) (dev-deps: `0.5.1`)
- **Code changes:** [benches/build.rs](benches/build.rs), [rbx-dom/rbx_binary/benches/suite/main.rs](rbx-dom/rbx_binary/benches/suite/main.rs), [rbx-dom/rbx_binary/benches/suite/util.rs](rbx-dom/rbx_binary/benches/suite/util.rs) -- 3 files
- criterion 0.8 removed `criterion_group!` / `criterion_main!` macros in favor of a new runner. Benchmark functions remain similar but setup code will change.

### 3b. xml-rs 0.8 to 1.0

- **File:** [rbx-dom/rbx_xml/Cargo.toml](rbx-dom/rbx_xml/Cargo.toml)
- **Code changes:** [rbx-dom/rbx_xml/src/deserializer_core.rs](rbx-dom/rbx_xml/src/deserializer_core.rs), [rbx-dom/rbx_xml/src/serializer_core.rs](rbx-dom/rbx_xml/src/serializer_core.rs), and [rbx-dom/rbx_xml/src/types/physical_properties.rs](rbx-dom/rbx_xml/src/types/physical_properties.rs) -- 3 files, ~300 lines
- xml-rs 1.0 rebrands to the `xml` crate. The event-based reader/writer API has the same structure but types/namespaces changed. `ParserConfig`, `EventReader`, `EmitterConfig`, `EventWriter` all have equivalents but may be renamed or restructured.

### 3c. reqwest 0.11 to 0.13

- **Files:** [Cargo.toml](Cargo.toml), [rbx-dom/rbx_reflector/Cargo.toml](rbx-dom/rbx_reflector/Cargo.toml)
- **Code changes:** [src/cli/studio.rs](src/cli/studio.rs), [src/cli/syncback.rs](src/cli/syncback.rs), [src/cli/upload.rs](src/cli/upload.rs) -- 3 files
- reqwest 0.13 **still supports `blocking**`. Main breaking changes: internal HTTP client changes, some header/feature reorganization. The `reqwest::blocking::Client` API is preserved.

---

## Tier 4 -- High Risk

### 4a. bincode 1.x to 3.0

- **Files:** [Cargo.toml](Cargo.toml) (deps + build-deps), [rbx-dom/rbx_types/Cargo.toml](rbx-dom/rbx_types/Cargo.toml) (dev-deps)
- **Code changes:**
  - [build.rs](build.rs) -- `bincode::serialize_into` calls
  - [src/cli/plugin.rs](src/cli/plugin.rs) -- `bincode::deserialize` call
  - [src/cli/init.rs](src/cli/init.rs) -- `bincode::deserialize` call
  - ~16 test usages across `rbx-dom/rbx_types/src/` (serialization roundtrips)
- bincode 3.0 completely rewrites the API. Serde support is opt-in via the `serde` feature. `serialize`/`deserialize` become `bincode::serde::encode_to_vec` / `bincode::serde::decode_from_slice` with an explicit `Configuration`. For backward compatibility with existing bincode 1.x-encoded data (embedded plugin/template snapshots), use `config::legacy()`.
- **Critical concern:** The build.rs embeds snapshots at compile time that are deserialized at runtime. The format must remain compatible across the upgrade, or the embedded blobs must be regenerated.

### 4b. hyper 0.14 to 1.x + hyper-tungstenite 0.11 to 0.19

- **Files:** [Cargo.toml](Cargo.toml)
- **New dependency needed:** `hyper-util` (provides the server and service utilities removed from hyper core)
- **Code changes:**
  - [src/web/mod.rs](src/web/mod.rs) -- Server setup (~25 lines). `hyper::Server::bind` replaced by `hyper_util::server::conn::auto::Builder` + a TCP accept loop.
  - [src/web/api.rs](src/web/api.rs) -- Request/response handling (~50 lines). `hyper::Body` becomes a trait; use `http_body_util::Full<Bytes>` or `BoxBody`.
  - [src/web/ui.rs](src/web/ui.rs) -- Same Body/Response changes (~20 lines).
  - [src/web/util.rs](src/web/util.rs) -- Response builder helpers (~30 lines). `Body::from()` becomes `Full::new(Bytes::from(...))`.
  - [tests/rojo_test/serve_util.rs](tests/rojo_test/serve_util.rs) -- Test WebSocket client (~15 lines).
- **New dependencies:** `hyper-util`, `http-body-util`, `bytes` (if not already present)
- This is the most architecturally impactful change. The server accept loop, body type system, and service pattern all change fundamentally.

---

## Execution Strategy

Work in this order so each tier builds on stable ground:

1. **Tier 1** (rbx-dom: rand, bitflags) -- Isolated to rbx_types, no downstream API changes
2. **Tier 2** (winreg, serde_yaml) -- Simple API surface, low blast radius
3. **Tier 3** (criterion, xml-rs, reqwest) -- More code changes but well-scoped
4. **Tier 4** (bincode, hyper stack) -- Architectural changes, do last

After each tier: run `cargo test` and `cargo clippy` to verify. For rbx-dom changes, commit to the submodule first, then update the submodule pointer in the main repo.

**serde_yaml snapshot tests:** The 0.8 to 0.9 upgrade may change YAML serialization output slightly (e.g., tag formatting). Run `cargo insta review` after upgrading to accept snapshot diffs.