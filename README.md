# FFX CTB Rust/WASM Port

This is a fresh Rust port target for the CTB live editor. The existing Python/GitHub Pages repo is intentionally left alone.

## Goal

Move the deterministic CTB simulation out of Pyodide and into Rust compiled to WebAssembly, so the static browser version can approach native performance without a Python backend.

## Current Status

Implemented:

- Exact Rust port of `FFXRNGTracker`
- Known-output parity tests copied from the Python implementation
- WASM-facing API shell
- Porting map for the remaining parser, game-state, event, and renderer layers

Not implemented yet:

- Full event parser
- FFX game-state model
- Character/monster actions
- Incremental CTB renderer
- Drops/encounters tracker output

## Setup

Install Rust:

```powershell
winget install Rustlang.Rustup
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
```

Run native tests:

```powershell
cargo test
```

If a newly opened PowerShell still cannot find `cargo`, use the direct rustup path:

```powershell
& "$env:USERPROFILE\.cargo\bin\cargo.exe" test
```

Build the WASM package:

```powershell
wasm-pack build --target web
```

If `wasm-pack` cannot find the Rust binaries from PowerShell, temporarily add Cargo to `PATH`:

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
wasm-pack build --target web
```

## Porting Order

1. RNG tracker parity
2. Static data loading for characters, monsters, actions, formations, items
3. Parser commands used by `ctb_actions_input.txt`
4. `GameState`, actors, status/equipment/action effects
5. Encounter creation and CTB ordering
6. `IncrementalCTBRenderer` checkpoints and output formatting
7. Browser UI integration

The guiding rule is parity first, speed second: every layer should get fixture tests against Python outputs before replacing the browser renderer.
