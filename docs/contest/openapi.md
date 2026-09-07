# Procon 37 OpenAPI — 和文要約

公式配布の **「回答システムに関する情報」 ZIP** に同梱されている OpenAPI 仕様
(`.reference/procon37/openapi.json`) を Rust に写経した
`crates/protocol/src/match_config.rs` の根拠資料です。

> Source of Truth: `.reference/procon37/openapi.json` (v0.0.0, OpenAPI 3.1.0)
> Rust 写経: `crates/protocol/src/match_config.rs`
> Fixture: `crates/protocol/tests/fixtures/procon37-openapi.json`

## 認証は全エンドポイント共通

| 場所 | 名前 | 必須 |
|------|------|------|
| HTTP ヘッダ | `Procon-Token` | ❌ (`required: false`) |
| クエリ文字列 | `token` | ❌ |

公式ドキュメントでは両方が提示されており、サーバ実装により許可される手段が
異なる可能性があります。実装では両方を同列に扱うこと (`PlayerAuth.tokenHeader` /
`PlayerAuth.tokenQuery` として参照)。

---

## エンドポイント一覧

公式タグは次の 4 区分です。

1. `1_初期アクセス` — 試合設定取得
2. `2_初期設定` — エージェント種別提出
3. `3_状態取得` — 試合状態取得
4. `4_行動提出` — 行動計画提出

### `GET /setting` — 試合設定取得 (`1_初期アクセス`)

`operationId`: `ProconApi_getMatchSetting`

- **用途**: 試合開始前の静的な設定情報 (盤面・スポット数・燃料上限・他プレイヤー数など)
  を取得する。
- **200**: [`MatchSetting`](#matchsetting)
- **401**: [`AuthError`](#autherror)
- **403**: [`AccessTimeError`](#accesstimeerror)

### `POST /agent` — エージェント種別決定 (`2_初期設定`)

`operationId`: `ProconApi_submitAgentKinds`

- **用途**: 各エージェントに割り当てる種類 (`0` or `1`) を提出する。
- **requestBody**: `AgentKind[]` (要素数 = エージェント数)
- **200**: 受理 (レスポンスボディなし)
- **401**: [`AuthError`](#autherror)
- **403**: [`AccessTimeError`](#accesstimeerror)

例: `[0, 1, 0, 1]`

### `GET /` — 試合状態取得 (`3_状態取得`)

`operationId`: `ProconApi_getMatchState`

- **用途**: 現在日 (または次の日) の試合状態スナップショットを取得する。
- **200**: [`MatchState`](#matchstate)
- **401**: [`AuthError`](#autherror)
- **403**: [`AccessTimeError`](#accesstimeerror)

### `POST /` — 行動計画提出 (`4_行動提出`)

`operationId`: `ProconApi_submitActions`

- **用途**: その日の全エージェント分の行動計画を提出する。
- **requestBody**: `int[][]`
  - 外側: エージェント数ぶんの配列
  - 内側: 1 エージェント分の行動列
  - 各要素の意味:
    - `-N` (N > 0): N ステップ待機
    - `0..=5`: 移動方向 (`0:左上, 1:右上, 2:右, 3:右下, 4:左下, 5:左`)
  - 例: `[-15]` = 15 ステップ待機、`[0, 1, -10]` = 移動 2 回 + 10 ステップ待機
- **200**: [`ActionAccepted`](#actionaccepted)
- **401**: [`AuthError`](#autherror)
- **403**: [`AccessTimeError`](#accesstimeerror)

受理ルール (公式仕様より):

- 合計ステップ数 ≠ 当日ステップ数 → 不正
- 移動できない方向への移動 → 不正
- 必要燃料不足の移動 → 不正
- 1 エージェントでも不正があれば全エージェント分の提出が破棄される

---

## スキーマ

### `AccessTimeError`

| フィールド | 型 | 必須 | 説明 |
|-----------|----|------|------|
| `code` | `"AccessTimeError"` (固定) | ✅ | 識別子 |
| `message` | string | ✅ | 「試合が設定される前、または試合終了後」を示す人間可読メッセージ |

HTTP 403 と組み合わせて返される。

### `ActionAccepted`

| フィールド | 型 | 必須 | 説明 |
|-----------|----|------|------|
| `revision` | int32 | ✅ | 受理リビジョン番号。0 未満 = 失敗、0 以上 = 受理。同日内で大きい値ほど後から受理されたもの。 |

### `AgentKind`

`number` (`enum: [0, 1]`)。エージェント種別。
`MatchSetting.agents` のインデックスに対応。

### `AgentState`

| フィールド | 型 | 必須 | 説明 |
|-----------|----|------|------|
| `kind` | [`AgentKind`](#agentkind) | ✅ | エージェント種別 |
| `pos` | int32 | ✅ | 盤面セル位置 (row-major 線形化インデックス) |
| `fuel` | int32 | ✅ | 残り燃料 |

### `AuthError`

| フィールド | 型 | 必須 | 説明 |
|-----------|----|------|------|
| `code` | `"AuthError"` (固定) | ✅ | 識別子 |
| `message` | string | ✅ | 認証失敗を示す人間可読メッセージ |

HTTP 401 と組み合わせて返される。

### `CellType`

`number` (`enum: [0, 1, 2, 3]`)。`Map.cells` の各要素。

### `Map`

| フィールド | 型 | 必須 | 説明 |
|-----------|----|------|------|
| `height` | int32 (`8..=32`) | ✅ | 盤面の高さ |
| `width` | int32 (`8..=32`) | ✅ | 盤面の幅 |
| `cells` | `CellType[][]` | ✅ | row-major 2 次元配列 (`cells[y][x]`) |

### `MatchSetting`

試合開始前に取得する静的な設定。

| フィールド | 型 | 必須 | 説明 |
|-----------|----|------|------|
| `startsAt` | int64 | ✅ | UNIX 時間の開始時刻。未確定の間は `0`。 |
| `daySeconds` | int32[] | ✅ | 各日の回答時間 (秒) |
| `daySteps` | int32[] | ✅ | 各日のステップ数。範囲 `W+H ..= (W+H)*4` |
| `map` | [`Map`](#map) | ✅ | 盤面 |
| `spots` | [`Spot`](#spot)[] | ✅ | うどん販売所のリスト |
| `agents` | int32[] | ✅ | 自プレイヤーのエージェント初期位置 (セル index) |
| `fuelLimits` | int32 | ✅ | 燃料積載量上限 (正整数、1 日目ステップ数の 1〜3 倍) |
| `players` | int32 | ✅ | 参加競技者数 |
| `busyThreshold` | int32 (`1..=5`) | ✅ | 混雑基準値 |
| `jammedThreshold` | int32 (`2..=10`) | ✅ | 渋滞基準値 |

### `MatchState`

各日開始時点の試合状態スナップSHOT。

| フィールド | 型 | 必須 | 説明 |
|-----------|----|------|------|
| `endsAt` | int64 | ✅ | UNIX 時間の終了時刻 |
| `day` | int32 | ✅ | 日 (初日は 0) |
| `agents` | [`AgentState`](#agentstate)[] | ✅ | 自プレイヤーのエージェント一覧 |
| `others` | [`OtherPlayer`](#otherplayer)[] | ✅ | 他プレイヤーのエージェント一覧 |
| `traffics` | [`TrafficInfo`](#trafficinfo)[] | ✅ | 道路状態一覧 |

### `OtherPlayer`

| フィールド | 型 | 必須 | 説明 |
|-----------|----|------|------|
| `id` | int32 | ✅ | プレイヤー ID |
| `agents` | [`AgentState`](#agentstate)[] | ✅ | そのプレイヤーのエージェント |

### `Spot`

| フィールド | 型 | 必須 | 説明 |
|-----------|----|------|------|
| `brand` | int32 | ✅ | 系列 ID |
| `pos` | int32 | ✅ | セル位置 |
| `stocks` | int32 (`>= 1`) | ✅ | 最大在庫量 |

### `TrafficInfo`

| フィールド | 型 | 必須 | 説明 |
|-----------|----|------|------|
| `pos` | int32 | ✅ | 道路 / セルの位置 |
| `status` | [`TrafficStatus`](#trafficstatus) | ✅ | 現在の道路状態 |

### `TrafficStatus`

`number` (`enum: [0, 1, 2]`)。道路状態。

---

## Rust 写経の方針

`crates/protocol/src/match_config.rs` で以下の方針を採用している。

1. **JSON フィールド名は camelCase そのまま**:
   全 struct / enum に `#[serde(rename_all = "camelCase")]` を付与し、
   `startsAt` / `daySeconds` 等の公式名を崩さない。
2. **整数 enum は値そのもの**:
   `AgentKind` / `CellType` / `TrafficStatus` は `#[serde(rename = "0")]` 等で
   数値リテラルとして直列化する。`AgentKind::value() -> i32` 等の補助 API も
   提供。
3. **`code` リテラル enum**:
   `AuthError.code` / `AccessTimeError.code` は spec 上のリテラル
   (`"AuthError"` / `"AccessTimeError"`) をそのまま採用するため、
   単一 variant enum として定義。
4. **必須 / 任意の区別**:
   spec で `required` 指定されたフィールドは `Option` で包まず、
   未指定フィールドは存在しないため `Option<T>` を使わない。
   欠落は serde のデコードエラーとして表現する。
5. **domain との衝突回避**:
   既存の `Hello` / `MatchSnapshot` / `ClientEvent` / `ServerEvent` と名前空間が
   重なる構造体型は導入せず、本モジュールは spec 由来の型のみを集約する。

## 関連リンク

- 元データ: `.reference/procon37/openapi.json`
- Fixture: `crates/protocol/tests/fixtures/procon37-openapi.json`
- Rust 写経: `crates/protocol/src/match_config.rs`
- ラウンドトリップテスト: `crates/protocol/tests/match_config_roundtrip.rs`
- Issue: #14