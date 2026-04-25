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

## Rust Port Checkpoints

- RNG tracker parity is implemented and covered by known-output tests.
- Parser normalization covers action, monster action, equip, party, heal, status/stat, encounter, and friendly RNG roll aliases.
- The simulator now handles party changes, summons, manual RNG advances, `status atb`, heal, equipment/element no-ops, stat/status edits, monster spawns, early encounter monster ICVs for `sinscales`, `ammes`, and `tanker`, placeholder unknown encounters, and basic CTB advancement for character/monster action lines.
- Encounter formation, forced party/condition, random-zone formation selection, character defaults, monster HP/agility, and action ranks now load from vendored upstream data files in this repo instead of leaning on external repo paths or narrow hardcoded Rust tables.
- Action metadata now also comes from the upstream bins for target class, status application, buffs, healing/removal flags, delay flags, and monster delay immunity, so the simulator no longer has to special-case most common status actions by name.
- Monster-specific target overrides now load from `monster_actions.json`, giving monster actions a closer target shape for entries such as party-wide Blender and Spines-style random character attacks.
- Equipment commands now parse and store known autoabilities, with First Strike/Initiative feeding encounter ICV logic, auto-statuses applied at encounter start, and weapon status abilities applied shallowly on weapon-property actions.
- The repo now vendors the core upstream tracker datasets under `data/`, including `characters.json`, action bins, monster data, formations, monster action metadata, and equipment/item tables needed for future parity work.
- The default `search_outputs/3096296922/ctb_actions_input.txt` smoke test fixture now lives locally under `fixtures/` and currently renders with zero unsupported command gaps.
- `wasm-pack build --target web` has been verified and emits the browser package under `pkg/`.
- The JSON render response separates command coverage from parity: `implemented` means every parsed command was handled by the Rust layer, while `parity_complete` remains `false` until full Python-equivalent event logic is ported.
- Action effects are still intentionally shallow: common ranks plus upstream-driven status/buff/heal/delay and first-pass equipment effects are modeled, but damage formulas, full target-selection parity, full equipment mechanics, elemental affinity mechanics, and event-specific AI remain to be ported for parity.
