/* tslint:disable */
/* eslint-disable */

export function chocobo_action_json(seed: number, input: string, cursor_line: number, action_kind: string, slot_index?: number | null): string;

export function chocobo_swap_json(seed: number, input: string, cursor_line: number, slot_index: number, replacement: string): string;

export function garuda1_attacks_json(input: string, cursor_line: number, attacks: string): string;

export function garuda2_attack_json(seed: number, input: string, cursor_line: number, attack: string): string;

export function init_wasm(): void;

export function lancet_tutorial_timing_json(input: string, cursor_line: number, timing: string): string;

export function no_encounters_routes_json(seed: number, input: string, start_line: number, encounters_input?: string | null, encounters_output?: string | null): string;

export function party_json(seed: number, input: string, cursor_line: number): string;

export function render_ctb_diff_json(seed: number, input: string, previous_input: string): string;

export function render_ctb_json(seed: number, input: string): string;

export function rng_preview_json(seed: number, index: number, count: number): string;

export function sample_json(): string;

export function tanker_pattern_json(input: string, cursor_line: number, pattern: string): string;

export function tracker_default_json(tracker: string, seed: number): string;

export function tracker_render_json(tracker: string, seed: number, input: string): string;

export function tros_attack_json(input: string, cursor_line: number, attack: string): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly chocobo_action_json: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => [number, number, number, number];
    readonly chocobo_swap_json: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => [number, number, number, number];
    readonly garuda1_attacks_json: (a: number, b: number, c: number, d: number, e: number) => [number, number, number, number];
    readonly garuda2_attack_json: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number, number];
    readonly lancet_tutorial_timing_json: (a: number, b: number, c: number, d: number, e: number) => [number, number, number, number];
    readonly no_encounters_routes_json: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => [number, number, number, number];
    readonly party_json: (a: number, b: number, c: number, d: number) => [number, number, number, number];
    readonly render_ctb_diff_json: (a: number, b: number, c: number, d: number, e: number) => [number, number, number, number];
    readonly render_ctb_json: (a: number, b: number, c: number) => [number, number, number, number];
    readonly rng_preview_json: (a: number, b: number, c: number) => [number, number, number, number];
    readonly sample_json: () => [number, number, number, number];
    readonly tanker_pattern_json: (a: number, b: number, c: number, d: number, e: number) => [number, number, number, number];
    readonly tracker_default_json: (a: number, b: number, c: number) => [number, number, number, number];
    readonly tracker_render_json: (a: number, b: number, c: number, d: number, e: number) => [number, number, number, number];
    readonly tros_attack_json: (a: number, b: number, c: number, d: number, e: number) => [number, number, number, number];
    readonly init_wasm: () => void;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
