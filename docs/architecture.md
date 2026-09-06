# Architecture

procon-2026 のシステム全体像と依存方向、trust boundary を記述する。実装詳細よりも「なぜこの分離か」を優先する。

## 全体構成

```
┌──────────────────────────────────────────────────────────┐
│  Browser                                                 │
│  ┌──────────────────────────────────────────────────┐    │
│  │  SolidJS + TypeScript (packages/frontend)        │    │
│  │  ├─ UI components                                  │    │
│  │  ├─ WebGPU renderer                                │    │
│  │  └─ Protocol codec (mirrors crates/protocol)       │    │
│  └──────────────────────────────────────────────────┘    │
│                       ▲ JSON over WS / IPC                │
└───────────────────────┼──────────────────────────────────┘
                        │
┌───────────────────────┼──────────────────────────────────┐
│  Host / CI                                             │
│                       ▼                                  │
│  ┌──────────────────────────────────────────────────┐    │
│  │  Rust workspace                                   │    │
│  │  ┌─────────┐  ┌──────────┐  ┌─────────┐ ┌─────┐  │    │
│  │  │ domain  │←─│ protocol │←─│ engine  │─│ cli │  │    │
│  │  │(pure)   │  │(serde)   │  │(tokio)  │ │     │  │    │
│  │  └─────────┘  └──────────┘  └─────────┘ └─────┘  │    │
│  └──────────────────────────────────────────────────┘    │
└──────────────────────────────────────────────────────────┘
```

## Crate 責務

| Crate | 責務 | 依存方向 | I/O 方針 |
|---|---|---|---|
| `domain` | ドメイン型・不変条件・純粋ゲームロジック | 外部 crate に依存しない | 完全 no I/O |
| `protocol` | Rust ↔ Frontend 間の JSON v1 メッセージ | `domain` のみ | I/O なし、serde のみ |
| `engine` | 試合状態機械・match table・replay | `domain` + `protocol` + `tokio` + `tracing` | async I/O 枠を持つが transport 非依存 |
| `cli` | operator バイナリ | 全 crate | std I/O のみ、network は将来追加 |

**重要**: `domain` は他の crate に依存しない。これにより、コアロジックを独立して unit test できる。

## Trust boundary

- **Frontend ↔ Engine**: JSON envelope。`crates/protocol` でスキーマを一元管理し、frontend は同じ v1 を `packages/frontend/src/lib/protocol.ts` でミラーする。
- **Engine ↔ OS**: 将来 WebSocket / Unix socket を立てるが、当面は CLI と in-process tokio のみ。
- **CI ↔ GitHub**: workflow は `permissions: read-only` をデフォルトとし、必要な job だけ `write` を opt-in する。

## State model

- `MatchTable` (`crates/engine/src/match_table.rs`) が試合の唯一の状態保持。
- 各 match は独立した state を持ち、`Arc<RwLock<HashMap<MatchId, MatchEntry>>>` で multiplex。
- domain 側 `GameState` は immutable value object。state 変更は engine 側でのみ発生。

## Rendering

- SolidJS の reactive 層で UI state を扱う。
- 重い描画は WebGPU compute / render pipeline に逃し、CPU 側の Solid render は frame 数の HUD など軽量情報に限定する。
- WebGPU 未対応端末では fallback を出す (`packages/frontend/src/lib/webgpu/support.ts`)。

## 依存方向の禁止ルール

- `domain` → 他 crate への依存は禁止。
- `protocol` → `engine` への依存は禁止。
- `engine` → `cli` への依存は禁止。
- 上記を破る変更は ADR 必須。