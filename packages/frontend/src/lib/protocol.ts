/**
 * Protocol types — must mirror `crates/protocol/src/` in the Rust workspace.
 *
 * When the Rust side changes, regenerate this file or update it by hand.
 * The shape is intentionally narrow: every event carries a discriminant.
 */

export interface ProtocolVersion {
  readonly major: number;
  readonly minor: number;
}

export interface MatchId {
  readonly type: 'match';
  readonly value: string;
}

export interface PlayerId {
  readonly type: 'player';
  readonly value: string;
}

export interface TurnId {
  readonly type: 'turn';
  readonly value: string;
}

export type Score = number;

export type Outcome =
  | { readonly kind: 'pending' }
  | { readonly kind: 'draw' }
  | { readonly kind: 'winner'; readonly player: PlayerId };

export interface GameState {
  readonly matchId: MatchId;
  readonly turn: number;
  readonly scores: ReadonlyArray<readonly [PlayerId, Score]>;
}

export interface Turn {
  readonly id: TurnId;
  readonly author: PlayerId;
  readonly actionIndex: number;
  readonly payload: unknown;
}

export interface MatchSnapshot {
  readonly matchId: MatchId;
  readonly state: GameState;
  readonly outcome: Outcome;
  readonly serverTimeUnixMs: number;
}

export interface Hello {
  readonly protocol: ProtocolVersion;
  readonly serverId: string;
  readonly supportedFeatures: ReadonlyArray<string>;
}

export type ClientEvent =
  | { readonly type: 'hello'; readonly player: PlayerId }
  | {
      readonly type: 'submit_turn';
      readonly matchId: MatchId;
      readonly actionIndex: number;
      readonly payload: unknown;
    }
  | { readonly type: 'request_snapshot'; readonly matchId: MatchId }
  | { readonly type: 'ping'; readonly nonce: number };

export type ServerEvent =
  | {
      readonly type: 'welcome';
      readonly serverId: string;
      readonly features: ReadonlyArray<string>;
    }
  | { readonly type: 'snapshot'; readonly matchId: MatchId; readonly turn: number }
  | {
      readonly type: 'turn_accepted';
      readonly matchId: MatchId;
      readonly turnId: string;
    }
  | {
      readonly type: 'turn_rejected';
      readonly matchId: MatchId;
      readonly reason: string;
    }
  | { readonly type: 'pong'; readonly nonce: number }
  | { readonly type: 'error'; readonly message: string };

/** Current protocol version. Keep in sync with `crates/protocol/src/lib.rs`. */
export const PROTOCOL_VERSION: ProtocolVersion = { major: 1, minor: 0 };
