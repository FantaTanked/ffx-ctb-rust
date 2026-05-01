# FFX CTB Rust/WASM Port

This is a fresh Rust port target for the CTB live editor. The existing Python/GitHub Pages repo is intentionally left alone.

## Goal

Move the deterministic CTB simulation out of Pyodide and into Rust compiled to WebAssembly, so the static browser version can approach native performance without a Python backend.

## Current Status

Porting priority: focus first on behavior relevant to the bundled/default CTB action route and its browser helper flows. Broader parity for other action files and alternate routes comes after the default CTB route is clean, fast, and practically reliable.

Implemented:

- Exact Rust port of `FFXRNGTracker`
- Known-output parity tests copied from the Python implementation
- WASM-facing API shell
- Porting map for the remaining parser, game-state, event, and renderer layers
- Basic CTB simulation for party changes, RNG advances, early encounter monster ICVs, Python-style scripted starting monster-party trimming for `sinscales`/`sahagin_chiefs`, first-pass Geosgaeno Ambush scripted pre-actions, first-pass Ammes/Wakka tutorial scripted enemy action overrides, first-pass Sinscales/Machina 3 scripted respawns, character/monster turn advancement, haste/slow, and Python-style heal output
- Shallow route-continuity support for summon, spawn, stat/status edits including Python's explicit `unknown` actor, MP stat tracking, equipment, element edits, Python-style state-command usage/parser errors, unknown encounter errors, and the default `ctb_actions_input.txt`
- Static formation, character defaults, and action/monster data vendored from the upstream tracker into `data/`
- Action metadata loading for ranks, status application, buffs, healing, delay flags, monster-specific target overrides, all-target resolution, ranked HP/MP/stat target selection, and first-pass last-target resolution from the upstream bins
- First-pass HP/MP stat handling for max-stat edits, max-stat status flags, Python-style character caps and uncapped monster HP/MP stat edits, zero-HP status/buff cleanup without auto-removing Death on positive max-HP edits, healing restoration, MP action costs, Half/One MP Cost, Magic Booster, MP damage, and MP drains
- Shallow equipment ability handling for First Strike, Initiative, HP/MP bonus and Break HP/MP/Damage Limit abilities, auto-statuses, SOS auto-statuses, Ribbon/Aeon Ribbon immunities, weapon status abilities, armor status wards/proofs, and Python-style invalid ability errors
- First-pass automatic reaction handling for Counterattack, Evade & Counter, Magic Counter, and inventory-consuming Auto-Potion/Auto-Med/Auto-Phoenix after monster actions
- Verified `wasm-pack build --target web` output in `pkg/`
- Local smoke-test fixture for the default route script under `fixtures/`, with coverage for zero unsupported command gaps and no rendered `Error:` rows
- Line-for-line CTB output parity for the bundled/default `fixtures/ctb_actions_input.txt` route at seed `3096296922` against the Python renderer
- WASM web-editor helpers for loading the bundled sample script and reporting party/reserves at the editor cursor
- Web encounter scanning now mirrors the Python frontend's stripped `encounter ` line scan for dropdown/cursor metadata, with block-commented encounter rows ignored including closes on lines ending with `*/`, textarea-style trailing blank line counts preserved, stale rendered encounter metadata bypassed after live edits, and cursor helpers bounded to the active encounter block
- Browser startup now auto-loads the bundled CTB sample like the Python web editor
- First-pass Chocobo Eater action and party-swap insertion helpers in Rust/WASM, including Python-style active-encounter-state validation at the raw textarea cursor, active-line handling for commented haste markers, raw-line macro/repeat prefix replay, inferred encounter-party sync from character rows, and dead/ejected/petrified reserve filtering for auto-swaps
- First-pass Tros first-attack helper in Rust/WASM, matching the Python editor's replace-or-insert behavior for `m1 attack` and `m1 tentacles`
- First-pass Garuda 1 five-attack helper in Rust/WASM, matching the Python editor's matching-row replacement while preserving surrounding encounter notes
- First-pass Garuda 2 attack helper in Rust/WASM, replacing or removing the active Garuda action row and using the Python helper's active-seed opening-CTB insertion point when no row exists
- First-pass Lancet Tutorial helper in Rust/WASM, toggling the Ragora Seed Cannon timing before or after Lancet
- Default drops/encounters tracker payloads, including anypercent notes and encounter slider metadata
- First-pass encounters tracker rendering in Rust/WASM, with Python-style cleanup of CTB encounter output including tracker-specific usage/directive handling, inert and indentation-tolerant block-comment `/repeat`, random-row zone hiding, preserved encounter-row spacing, boss/simulation formation lists including empty formations, encounter-count rows, section separators, and padded columns unless active `/nopadding` is present
- First-pass drops tracker rendering for steal, Python-style steal success/rarity RNG lanes, kill item drops including bare monster kill lines, AP row text, manual `ap` commands, kill/bribe AP credits with canonical uppercase initials, item/gil inventory commands and parser errors including negative switch-slot range errors, first-pass equipment drops, Python-style equipment and auto-ability names/spellings including duplicate handling before the four-ability cap, manual equipment inventory commands and parser errors including internal `None` equipment slots and `None` item-switch labels, Python-style item-inventory table width for internal empty slots, Python-style `inventory show gil` spacing, bribe-drop notes, party/death/roll/drop-command/AP parser errors including negative RNG-index roll validation, indentation-tolerant block comments with inert `/nopadding` and `/repeat`, Python-style duplicate monster names, lowercased event-token dispatch, case-insensitive fourth-token overkill markers, column padding, column-sensitive `/nopadding` and `/repeat`, `/usage` with optional trailing text, `/macro` errors, `///`, and the default drops route without unsupported-command rows
- Browser tracker panes for drops and encounters, including encounters slider controls
- Virtual monster CTB catch-up now uses Python-style preview semantics, persisting RNG plus HP/MP damage while restoring target memory and automatic reaction side effects
- Scripted encounter setup includes Python's Geneaux starting party override, so its CTBs initialize for Tidus/Yuna/Lulu
- Initial counter target memory now defaults to Tidus like Python's `GameState`
- Action MP costs now subtract through Python-style MP clamping, so low-MP action spending bottoms out at zero
- CTB action macro expansion from the upstream default macros file, before Python-order `/repeat` expansion
- CTB input preparation now auto-prefixes bare monster-name action lines like the Python web editor and normalizes generated route-template encounter prefixes such as `71 - nameencounter ...` / `+-encounter ...`
- Generated CTB output pasted back as input can now use `# Encounter:`/`# Random Encounter:` metadata rows to restore explicit monster slot lists plus character/monster CTB values, so default-adjacent enemy-turn route files render without slot errors
- CTB rendering skips Tanker slot placeholder comments like Python while preserving normal comments
- The web CTB sidebar includes the Python-style Tanker pattern helper for overwriting `m5/m7/m6/m2/m3/m4/m8` action rows
- Sahagin Chief spawn comments now preserve the comment and apply Python-style hidden spawns, including the `4th appears` unlock
- Unavailable character/monster action lines now render Python-style `# skipped: ...` comments before validating the action
- Block-comment handling that preserves ignored CTB lines without executing their effects, including indented block-comment fences
- First-pass virtual CTB catch-up and scripted Geosgaeno Ambush pre-actions before character actions, including earlier monster turns rendered as Python-style inserted `m#` / `m# action` lines with `# enemy rolls:` damage/miss comments and enemy-to-party block spacing, using the vendored Python default enemy-action table plus `attack`, one-action monsters, and forced actions, while leaving Python manual-only Sinscale/Echuilles turns for explicit route input
- Duplicate same-name monster action rows now keep Python's first target definition, fixing automatic/default enemy attacks such as Dingo `Attack` selecting the correct random target instead of the later low-HP variant
- First-pass explicit character and monster action roll comments now append Python-style `# party rolls:` / `# enemy rolls:` damage/miss summaries, and rendered explicit action lines now echo Python-style command text instead of rich Rust action rows while lower-level helpers still expose detailed rows for tests/helpers
- Rich action-result rows now suppress populated damage payloads for `NoDamage` formulas like Python, preserving `(No damage)` plus status/buff text
- MP damage/drain rows now report the same target-current-MP cap that is applied to battle state, matching Python roll/result output
- Reflected elemental actions now check the original Reflect holder for Nul/miss/hit behavior before applying effects to the reflected target like Python
- Counter-revive Auto-Regen clamps revived actors at max HP like Python, and CTB inventory item errors now list Python parser tokens
- Petrify preserves existing buff stacks and applies Python's final same-action status cleanup, and duplicate monster actions expose target-suffixed command aliases
- Encounters tracker output cleanup preserves plain first encounter rows unless a `Command: ///` hide marker is present
- Chocobo Eater cursor replay skips blank, hash-comment, and indented block-comment lines before shadow CTB ticking, including Python-style block-comment closes on lines ending with `*/`, matching the Python web helper flow for commented route notes
- Chocobo Eater and party-at-cursor helpers now replay only the macro-expanded prefix before the raw textarea cursor line, so default-route helpers after `/macro` rows stay aligned with the Python editor cursor
- Chocobo Eater shadow CTBs now tick only after actual turn/action commands, avoiding stale double-ticks from helper lines like `status atb` or party swaps
- Encounter-start reserve CTBs are initialized, normalized, and ticked down for later party swaps, improving Chocobo Eater helper parity
- Python-style multiline action result formatting for character/monster hit, miss, damage, grouped status/removal tokens, duplicate status coalescing, buff, and reflect rows
- Monster multi-hit random target selection now uses Python's RNG5-per-hit lane while preserving RNG4 for single-hit monster random targets
- Reflected actions now retarget using Python's Death/Eject target filters while still allowing Petrified targets
- CTB parser aliases for `end encounter`, Python-style `m#` `monsteraction` slots, and monster-name `monsteraction` inputs, with named temporary monsters kept outside the encounter monster party, retired from CTB after rendering, and still available for target-memory counters; bare numeric monster-action actors are treated as names like Python, `forced_action` resolves through the monster's real forced action, plus Python-style handled errors for invalid `action` actors and bare `monsteraction`
- CTB roll parser errors now render without falling into the unsupported-command path
- CTB rendering treats generated Python result-pane headers such as `Action Results` as inert, so generated search/analyze action outputs can be re-rendered without spurious action-usage errors
- CTB slash directives now render as their raw input lines like the Python web editor
- Unknown CTB lines now render Python-style impossible-parse errors without being counted as Rust-port coverage gaps
- CTB and drops hash comments/directives now only trigger from column 1, matching Python's parser behavior for indented `#` and `/` lines, while block-comment fences tolerate surrounding whitespace
- CTB block comments now render with Python-style `# ` prefixes while still suppressing the commented commands, including indented fences
- CTB `element` commands now mirror Python's loose monster-slot parsing quirks for slot-like strings such as `m10`, `m0`, and `x1`, while `stat`/`status` keep Python's stricter exact monster-slot recognition
- CTB `compatibility` command parsing/rendering with Python-style clamping and parse errors
- CTB `encounters_count` command parsing/rendering for total, random, and zone counters
- First-pass CTB `inventory` gil/item/equipment commands for `show`, `show equipment`, `show/get/use gil`, `get/buy/use/sell` items, `get/buy/sell equipment`, `sell equipment [slot]`, `switch`, and `autosort`, including Python-style totals, prices, fixed item slots with `None` holes, equipment slot holes/trailing cleanup, inventory table, equipment inventory display with owner/sell-value suffixes, inventory errors, and parser errors
- CTB `death` command parsing/rendering with Python-style RNG10 advancement
- First-pass CTB `magusturn` parsing/rendering through existing action simulation, including Python-style command/motivation output even after virtual monster turns, command-menu RNG18 side effects and RNG-gated menu availability, Python-style Magus fallback HP/MP/combat stats, first-pass motivation persistence after `Taking a break...`, first-pass `Fight!` AI action selection, first-pass `Do as you will.` branches for Cindy, Sandy, and Mindy including Cindy's MP-gated spell RNG plus Mindy's reflected-Cindy spell chain with sequential two-spell output formatting and reflected spell-list replay for `One more time.`, broader `One more time.` repeat/fallback paths for Cindy, Sandy, and Mindy including live-target fallback break penalties plus repeat-after-`Do as you will.` cases for Sandy, reflected Mindy spells, and Mindy's no-target fallback break, first-pass Cindy `Go, go!`, `Help each other!` with Python-style hidden cure-tier RNG consumption, first-pass Sandy `Defense!` support-chain selection with filtered random support targets plus post-`Defense!` `One more time.` repeat/fallback paths, first-pass Mindy `Are you all right?` recovery-chain selection, `One more time.` command availability/repeat-state rules, Mindy low-resource command availability, Auto-Life counter dispatch, Delta Attack for Combine Powers, `Taking a break...` fallback rows for missing monster targets, and direct `Dismiss` handling with Python-style all-sister `on_dismiss` CTB/revive side effects
- CTB target validation now matches Python more closely for required empty monster slots, filtered/sorted explicit party/monster aliases with empty `Unknown` fallback, ranked character target `Unknown` fallback, optional party/random-character overrides, Python-index ordering for eligible character targets after swaps, whole-list multi-hit party target expansion, fixed data targets that retain existing non-party/dead actors, per-actor last-target clearing/fallback and last-attacker memory including Python's narrower Death-only last-target memory filter, no-memory counter `Unknown` fallback, plus retired temporary valid monster-name targets and invalid-name/empty-slot fallback to the action's default target shape
- CTB `element`, `spawn`, `equip`, `status`, `stat`, and `ap` parser/output paths now match Python enum/range validation more closely, including explicit `unknown` actor state commands, zero-clamping negative `stat <actor> ctb` edits, and AP totals clamping at zero while still reporting negative manual AP adjustments
- Python-style CTB blank-line collapsing in rendered output
- Status output now reports active status durations, manually supplied status stacks, upstream special statuses, and buff stacks
- Monster, boss, simulation, and random-zone data lookups now tolerate Python-style case/spacing differences
- CTB web render payloads now normalize output with Python-style trailing newlines and expose first-changed prepared-line metadata for browser incremental-render summaries
- Remove-status actions now keep action status flags separate from removable statuses, so flag-like effects are not cleared by item cures
- Character `escape` now uses Python-style output and success/failure side effects
- Delay actions now apply weak/strong delay after a hit even against delay-immune monsters, matching the Python action layer
- Drops tracker bare `roll`, `advance`, and `waste` commands now report Python-style RNG-index parse errors
- Tracker render JSON now matches the Python web payload shape without Rust-only status/message fields
- CTB web block comments render verbatim while encounters tracker cleanup keeps Python tracker-style commented block lines
- Status flags now skip 255-resisted targets silently like Python
- `inventory sell equipment weapon|armor ...` usage errors now distinguish incomplete equipment specs from slot sells
- Fixed-character monster action targets now fall back to `Unknown` when the character is not targetable in the active party
- Manual Chocobo Eater swap insertion now follows the Python frontend cursor/start-line rule
- Drain recovery clamps at max HP/MP like Python property setters
- Weapon status abilities merge with action statuses before rolling, matching Python's one-roll higher-chance status behavior
- Drops exposes the first No Encounters search WASM/API hook, including Python-style empty-input and missing-Ghost messages
- The No Encounters search hook now receives the active seed, runs a first-pass Rust drops route preview, aligns parsed future Encounters output/input to the Drops search section, ignores stale rendered Encounters output after live input edits, reports Python-style cursor/first-Ghost/tested-route, search-mode including first-pass `prefer-guaranteed` when exact output still needs a guaranteed route, and future-section context, constrains current-route preview and bounded synthesis to the first Ghost search window including commented optional Ghost rows, reports first-pass guaranteed-only vs random-row context for exact future rows through first Ghost, reports whether the current Ghost route already yields a No Encounters armor, recognizes commented Ghost rows as optional search rows, ignores indentation-tolerant block-commented repeat directives while preserving one-line block-comment notes, accepts Python-style one-line block comments in exact Encounters input, parses raw CTB-style `Random Encounter:`, `Multizone encounter:`, `Simulated Encounter:`, and `Encounter:` rows from output or pasted fallback input, aligns slash-separated raw multizone labels to the requested Drops section when possible, tries a bounded Ghost-repeat/pre-Ghost-death synthesis pass with candidate counts, applies normalized or synthesized drops input back into the browser, and includes parsed future-encounter row context from the Encounters pane output or by rendering valid Encounters input as a fallback
- Drops tracker rendering accepts generated optimizer outputs that start with `Drop Search Result`/`Drop Analysis Result` by skipping to the `Resolved drop route` section before parsing route commands
- Native examples include `render_ctb` and `render_tracker` probes for checking CTB and tracker output against local/generated fixtures without the browser
- Magus Sister action availability checks raw MP costs like Python before later action-spending discounts apply
- Core CTB normalization preserves reserve character CTB like Python, leaving Chocobo shadow CTB handling separate
- Total encounter-count changes recalculate first-pass Yuna-derived summon aeon stats for Valefor through Yojimbo
- Focused Rust verification currently covers the full native library suite, the default CTB transcript, encounters tracker, No Encounters route search, Chocobo cursor-state helpers, and `wasm-pack build --target web`

Not implemented yet:

- Full event parser
- FFX game-state model
- Full character/monster action parity beyond the current first-pass simulation
- Incremental CTB renderer
- Full Python parity for drops tracker output beyond the currently covered default route and first-pass No Encounters route-search surface
- Full Python parity for encounters tracker output beyond the currently covered focused tracker tests
- Full Python parity for Chocobo Eater party-swap behavior beyond the currently covered cursor-state, inferred-party, reserve-CTB, and shadow-CTB helper tests
- Broader parity fixtures for alternate CTB action files and uncommon route branches outside the bundled/default action route

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

Render a local CTB or tracker fixture from the command line:

```powershell
cargo run --example render_ctb -- 3096296922 fixtures\ctb_actions_input.txt
cargo run --example render_tracker -- drops 3096296922 ..\ctb-live-editor-pages\search_outputs\3096296922\seed_3096296922_search_drops.txt
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
