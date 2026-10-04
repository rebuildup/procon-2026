/**
 * Frontend view models for the hex map viewer.
 *
 * These types are intentionally decoupled from the Rust domain types in
 * `crates/domain/src/` so the viewer can be built and tested independently
 * of the protocol bridge. Once the bridge lands, the JSON wire format can
 * be either reused as-is, or these types can be re-exported through a
 * thin adapter that flattens the Rust-side enums into the strings this
 * module expects.
 *
 * - Coordinate system matches the contest map: rectangular `(col, row)`
 *   with `even-r` layout (Q&A Q1: even rows are not shifted, odd rows are
 *   offset right). See `coordinate.rs` for the canonical definition.
 * - `cell.terrain` mirrors the four-variant terrain enum (`Plain`,
 *   `Mountain`, `Pond` → `water`, `Road`).
 */

export interface HexCoord {
  readonly col: number;
  readonly row: number;
}

export type TerrainKind = 'plain' | 'mountain' | 'water' | 'road';

export interface HexCell {
  readonly coord: HexCoord;
  readonly terrain: TerrainKind;
}

export interface Spot {
  /** Stable per-spot identifier (e.g. `"s0"`). */
  readonly id: string;
  readonly coord: HexCoord;
  /** Series index (0-based). The viewer renders each series in a brand color. */
  readonly series: number;
}

export type AgentKind = 'patrol' | 'supply';

export interface Agent {
  readonly id: string;
  readonly kind: AgentKind;
  readonly coord: HexCoord;
}

export interface HexMap {
  readonly width: number;
  readonly height: number;
  readonly cells: ReadonlyArray<HexCell>;
}

/** Convenience: the terrain colors used by the viewer. Centralized for theming. */
export const TERRAIN_COLORS: Readonly<Record<TerrainKind, string>> = {
  plain: '#b6c79a',
  mountain: '#6b6b76',
  water: '#6fb4d6',
  road: '#d9cba6',
} as const;

/** Distinct hues for spot series; cycled by `series % SPOT_SERIES_COLORS.length`. */
export const SPOT_SERIES_COLORS: readonly string[] = [
  '#d97706', // amber
  '#9333ea', // violet
  '#dc2626', // red
  '#0891b2', // cyan
  '#16a34a', // green
  '#db2777', // pink
] as const;

/** Marker fills for agent kinds (used as the triangle stroke; see MapViewer). */
export const AGENT_COLORS: Readonly<Record<AgentKind, string>> = {
  patrol: '#1f2328',
  supply: '#0969da',
} as const;
