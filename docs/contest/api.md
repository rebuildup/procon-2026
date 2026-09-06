# API リファレンス

`procon` コア ↔ フロントエンド間の通信 API を、提出審査に必要な範囲で要約する。
実装本体は `crates/protocol/src/` 配下。

## 1. プロトコル概要

- 通信形態: JSON over WebSocket (本番戦) / JSON over stdin (replay)。
- 全メッセージは `Envelope<T>` でラップされる（`envelope.rs`）。
- `Envelope` は `version: ProtocolVersion` を含む。major バージョンが一致しない
  場合は即座に切断する。
- 現行バージョン: `1.0` (`crates/protocol/src/lib.rs` の `PROTOCOL_VERSION`)。

## 2. トップレベル型

| 型 | 役割 | 出典 |
| --- | --- | --- |
| `Envelope<T>` | 全送受信メッセージの共通ヘッダ | `envelope.rs` |
| `ProtocolVersion { major, minor }` | プロトコルバージョン | `envelope.rs` |
| `ClientFrame` / `ServerFrame` | フレーム集約 | `frame.rs` |
| `ClientEvent` / `ServerEvent` | クライアント/サーバ発行イベント | `event.rs` |
| `MatchSnapshot` | サーバ → フロントの現在状態スナップショット | `lib.rs` |
| `MatchSummary` | 試合終了時の集計 | `frame.rs` |

## 3. Envelope

```rust
struct Envelope<T> {
    version: ProtocolVersion,
    kind: EnvelopeKind,
    payload: T,
}
```

`EnvelopeKind` は少なくとも:

- `Hello` — ハンドシェイク開始
- `Event` — 試合進行イベント
- `Replay` — リプレイ転送
- `Error { message }` — サーバ側エラー通知

## 4. イベント種別 (Server → Client)

- `MatchStart { match_id, players }`
- `Turn { turn, actions, state }`
- `ScoreUpdate { player_id, score }`
- `MatchEnd { outcome, summary }`

## 5. イベント種別 (Client → Server)

- `Join { player_id, display_name }`
- `Action { turn, action }`
- `Surrender { player_id }`

## 6. ハンドシェイク

1. クライアント → サーバ: `Hello { client_version }`
3. サーバ → クライアント: `MatchSnapshot` で初期状態
4. 双方向 `Event` ループ
5. サーバ → クライアント: `MatchEnd` で終了

## 7. エラー処理

- プロトコル不一致は `ProtocolError::UnsupportedVersion`
- JSON パース失敗は `ProtocolError::Decode`
- 不正な状態遷移は `ProtocolError::Invalid(message)`

## 8. CLI / replay からの利用

- `procon hello` — サーバの `Hello` ペイロードを表示
- `procon replay <file>` — `Replay` JSON ファイルをエンドツーエンドで再生
- `procon version` — `PROTOCOL_VERSION` を含むバージョン情報

詳細は `crates/protocol/src/{envelope,event,frame}.rs` を参照。