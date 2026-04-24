# Porting Map

## Python Sources To Port

Core render path:

- `ctb_live_editor.py`
  - `prepare_action_lines`
  - `IncrementalCTBRenderer`
  - checkpoint snapshot/restore
- `ctb_enemy_inserter.py`
  - virtual CTB turn insertion
  - scripted encounter behavior
  - damage/comment formatting
- `anypercent_seed_optimizer.py`
  - `ScriptRunner`
  - action-line editing
  - repeat expansion

Upstream tracker engine:

- `ffx_rng_tracker/tracker.py`
- `ffx_rng_tracker/gamestate.py`
- `ffx_rng_tracker/data/actor.py`
- `ffx_rng_tracker/data/actions.py`
- `ffx_rng_tracker/data/monsters.py`
- `ffx_rng_tracker/data/encounters.py`
- `ffx_rng_tracker/events/*.py`

Data files:

- `ffx_rng_tracker/data/data_files/*.csv`
- `ffx_rng_tracker/data/data_files/*.json`
- `search_outputs/3096296922/ctb_actions_input.txt`

## Recommended Validation Fixtures

Create fixtures from Python before replacing each layer:

- RNG initial values and first N rolls for several seeds
- Parser normalized line output for representative commands
- Encounter state after each encounter line
- Full CTB rendered output for the default seed
- Focused outputs for Tanker, Garuda, Chocobo Eater, Spherimorph, and multi-zone encounters

## Performance Target

The first benchmark target should be the Tanker custom action path:

- Python desktop/local target: about `0.9s`
- Pyodide baseline: about `2.0s`
- Rust/WASM target: below local Python once equivalent checkpointing is implemented
