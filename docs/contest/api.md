# Procon 37 API 完全ドキュメント（公式 OpenAPI 仕様の和文詳細）

> Issue: #60 / 親 Issue: #14
> 公式ソース: `.reference/procon37/openapi.json` (OpenAPI 3.1.0)
> 写経先: `crates/protocol/tests/fixtures/procon37-openapi.json`
> 本ドキュメントは競技中にクライアントを実装する人間 / AI 実装者を対象とした **詳細な仕様書**。
> 簡易サマリは Issue #14 が `docs/contest/openapi.md` に作成（PR #66 でマージ予定）。
> 戦略・前提は [`docs/contest/strategy.md`](./strategy.md) を参照。

---

## 1. 概要

第 37 回高専プロコン競技部門「ヘキサうどん」の競技サーバは、参加者クライアントに対して
以下の性格を持つ HTTP API を提供する。

| 項目 | 内容 |
|---|---|
| トランスポート | HTTP/1.1 over TCP (LAN 上で動作) |
| データ形式 | `application/json; charset=utf-8` |
| 認証 | `Procon-Token` ヘッダ or `?token=` クエリパラメータ |
| タイムゾーン | UNIX 時刻 (UTC, 秒単位) |
| 数値エンコーディング | `int32` / `int64` (JSON 数値) |
| ベース URL | 大会運営から配布されるホスト名 (例: `http://192.168.x.y:PORT`) |

クライアント実装上の注意:

- 公式仕様 (募集要項 / `docs/pdf/通信仕様*.pdf`) は **JSON のみで運用** される想定。
  バイナリ protobuf は存在しない（仕様書には明示されない限り採用しない）。
- 全レスポンスは **公式の OpenAPI 仕様 1:1 の snake_case キーで** 返される。
  Rust 側で `#[serde(rename_all = "camelCase")]` 等を使って再命名する必要はない。
- 試合中 (`day` が 0 以上) は状態が頻繁に進む。GET の応答を poll する場合は
  `endsAt` を見てから 1 秒以上の間隔で再取得する。
- POST リクエストは **冪等ではない**（行動計画やエージェント種別は最後の提出で上書き）。
  リトライする場合は status code と応答内容を確認してから行うこと。

---

## 2. 認証

公式 OpenAPI では認証ヘッダ／クエリは `required: false` と記載されているが、
**実際には試合接続時に大会運営から配布される `Procon-Token` を必ず付けること**。
未指定の場合は 401 (`AuthError`) が返る。

| 渡し方 | 例 |
|---|---|
| HTTP ヘッダ | `Procon-Token: <token>` |
| クエリ文字列 | `?token=<token>` |

クライアント実装では両方サポートしておくと、接続形態（プロキシ・curl デバッグ等）に
左右されにくい。両方が付与された場合、サーバ側の実装依存だが一般に **ヘッダ優先**。

トークン流出時の影響:

- トークンを取得した第三者が正規クライアントになりすまして行動提出が可能。
- LAN 上で傍受される前提なので、競技本番では **HTTPS 終端プロキシを置く** か
  大会側の提供エンドポイントを使う。
- 開発・CI 用に読み取り専用トークンを分けてもらえると事故が減る（運営に要望中）。

---

## 3. エンドポイント一覧

公式 OpenAPI の tag は次の 4 フェーズに対応する。本ドキュメントではこの順で解説する。

> 注: OpenAPI 仕様上、試合状態取得と行動提出は同じパス `/` に **GET / POST で同居** している。
> タスク概要などで「GET /match」と呼ばれることがあっても、実装上は **`GET /`** である点に注意。

| # | Path | Method | Operation ID | 概要 | Tag |
|---|---|---|---|---|---|
| 1 | `/setting` | GET | `ProconApi_getMatchSetting` | 試合設定を取得する | 1_初期アクセス |
| 2 | `/agent` | POST | `ProconApi_submitAgentKinds` | エージェント種別を決定する | 2_初期設定 |
| 3 | `/` | GET | `ProconApi_getMatchState` | 試合状態を取得する | 3_状態取得 |
| 4 | `/` | POST | `ProconApi_submitActions` | 行動計画を提出する | 4_行動提出 |

呼び出し順序（試合 1 本ごと）:

```text
  GET /setting        ← 試合開始前: マップ・閾値・スポット等の静的設定を取得
        │
        ▼
 POST /agent          ← 試合開始直後: 自エージェントの種別（kind=0/1）を決定
        │
        ▼
  GET /setting        ← startsAt が確定したか再確認（必要なら数回 poll）
        │
        ▼  （startsAt が 0 以外の UNIX 時刻を返した瞬間 = 試合開始）
        │
  GET /        ← 当日の試合状態（agents / others / traffics / endsAt）を取得
        │
        ▼  （endsAt までに 1 日ぶんの行動を決定）
        │
 POST /        ← 全エージェント分の行動列を提出
        │
        ▼  （日が変わったら 3 に戻る）
```

---

## 4. リクエストスキーマ詳細

`POST /agent` と `POST /` の 2 つだけがリクエストボディを持つ。

### 4.1 `POST /agent` のリクエスト

提出するエージェント種別配列。要素数は **自プレイヤーのエージェント数と一致** させる。

```jsonc
// Content-Type: application/json
// 例: 4 体のうち 0 番目と 2 番目を kind=1、それ以外を kind=0
[0, 1, 0, 1]
```

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| (root) | `AgentKind[]` | ○ | エージェント種別一覧。長さ = エージェント数 |
| `AgentKind` | `0 \| 1` (number) | ○ | `0` または `1` のいずれか |

備考:

- 順序は GET `/` の `agents[].kind` 初期表示と一致する。
- 試合中に再提出した場合の挙動は公式 OpenAPI 仕様上未定義。**試合開始前に 1 回だけ** 提出する想定。

### 4.2 `POST /` のリクエスト（行動計画）

全エージェント分の行動列。外形は「**エージェントごとの配列の配列**」。

```jsonc
// Content-Type: application/json
// 例: 4 体ぶんの 1 日行動 (daySteps = 6 の場合)
[
  [ 0, 0, 1, -1, 2,  3 ],   // agent 0
  [ 1, 0, 0, 1, 1, -3 ],   // agent 1
  [ 2, 2, 2, 0, 4, -1 ],   // agent 2
  [-1, -1, -1, -1, 0, 5 ]   // agent 3 (途中待機 + 後半のみ移動)
]
```

各要素（int）の意味:

| 値 | 意味 |
|---|---|
| `-1` 以下 | 待機（絶対値 = 待機ステップ数） |
| `0` 〜 `5` | 移動方向（次節参照） |

#### 4.2.1 移動方向 (`direction`)

ヘキサ（六角）座標での隣接方向。OpenAPI では **int 0〜5** で符号化される。

| 値 | 方向 |
|---|---|
| 0 | 左上 |
| 1 | 右上 |
| 2 | 右 |
| 3 | 右下 |
| 4 | 左下 |
| 5 | 左 |

#### 4.2.2 妥当性ルール（公式仕様抜粋）

`POST /` の `description` フィールドに次のルールが明記されている。

> - 合計ステップ数がその日のステップ数と一致しない場合は不正
> - 移動できない方向への移動は不正
> - 必要燃料が不足する移動は不正
> - 1つでも不正な行動が含まれる場合、**全エージェントの行動計画を不正として扱う**

クライアント側でも **提出前にこれら 4 つのバリデーション** を通すこと。
不正な計画を提出すると `200` で受理されつつ後段で破棄される（仕様上 `revision < 0` が返る）。
これに気づかずに進むと `endsAt` を過ぎても無効な計画のままで 0 点になる。

#### 4.2.3 リクエストフィールド表

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| (root) | `array<array<int32>>` | ○ | `agents` と同数の外側配列。各内側配列の長さの合計 = `daySteps` |
| `[*]` | `int32[]` | ○ | 1 エージェント分の行動列。要素は「方向 (0..5) または負の待機値」 |
| `[*][*]` | `int32` | ○ | `-N` = N ステップ待機 / `0..5` = 移動方向 |

---

## 5. レスポンススキーマ詳細

各エンドポイントの 200 応答スキーマを列挙する。

### 5.1 `GET /setting` → `MatchSetting`

試合開始前に取得できる **静的設定**。`startsAt` だけ試合直前まで `0` で、
開始時刻確定後に非ゼロの UNIX 時刻を返す。

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `startsAt` | `int64` (UNIX 秒) | ○ | 試合開始時刻。確定前は `0` |
| `daySeconds` | `int32[]` | ○ | 各日の回答時間（秒） |
| `daySteps` | `int32[]` | ○ | 各日のステップ数。範囲 `W+H` 〜 `(W+H)×4` |
| `map` | `Map` | ○ | 盤面 |
| `spots` | `Spot[]` | ○ | スポット一覧 |
| `agents` | `int32[]` | ○ | 自プレイヤーのエージェント初期位置 |
| `fuelLimits` | `int32` | ○ | 巡回車の燃料上限（1 日目ステップ数の 1〜3 倍） |
| `players` | `int32` | ○ | 参加競技者数 |
| `busyThreshold` | `int32` | ○ | 混雑基準値（1〜5） |
| `jammedThreshold` | `int32` | ○ | 渋滞基準値（2〜10） |

例:

```json
{
  "startsAt": 0,
  "daySeconds": [60, 60, 60],
  "daySteps":   [64, 64, 64],
  "map":  { "height": 12, "width": 12, "cells": [[0,0,1,1,...], ...] },
  "spots": [
    { "brand": 0, "pos": 47, "stocks": 3 },
    { "brand": 1, "pos": 12, "stocks": 3 }
  ],
  "agents":       [42, 55, 68],
  "fuelLimits":   192,
  "players":      4,
  "busyThreshold":   3,
  "jammedThreshold": 6
}
```

### 5.2 `GET /` → `MatchState`

各日開始時点の試合状態。`endsAt` までの 1 分程度（または `daySeconds` 秒）に
`POST /` で行動計画を提出する。

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `endsAt` | `int64` (UNIX 秒) | ○ | 当日の回答締切 UNIX 時刻 |
| `day` | `int32` | ○ | 日番号（初日 = 0） |
| `agents` | `AgentState[]` | ○ | 自プレイヤーのエージェント一覧 |
| `others` | `OtherPlayer[]` | ○ | 他プレイヤーのエージェント一覧（id と agents） |
| `traffics` | `TrafficInfo[]` | ○ | 道路状態一覧 |

#### 5.2.1 `AgentState`

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `kind` | `AgentKind` | ○ | `0` か `1` |
| `pos` | `int32` | ○ | 現在のセル位置 |
| `fuel` | `int32` | ○ | 残り燃料（kind=1 / 巡回車のみ意味がある） |

#### 5.2.2 `OtherPlayer`

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `id` | `int32` | ○ | 他プレイヤーの識別子 |
| `agents` | `AgentState[]` | ○ | そのプレイヤーのエージェント一覧 |

#### 5.2.3 `TrafficInfo`

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `pos` | `int32` | ○ | 道路セルの位置 |
| `status` | `TrafficStatus` | ○ | 0=順調 / 1=混雑 / 2=渋滞 |

### 5.3 `POST /agent` → 200 (body なし)

受理されると HTTP 200 が返るのみでボディは存在しない（OpenAPI 上 `description` のみ）。
ステータスコード 200 なら受理、200/401/403 以外は未定義のため異常扱い。

### 5.4 `POST /` → `ActionAccepted`

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `revision` | `int32` | ○ | `0` 未満: 受理失敗 / `0` 以上: 受理。値が大きいほど新しい |

> 同じ日に複数回提出した場合（並列リクエストを含む）は、値が大きい提出がより後に
> 受理されたものです。（公式仕様 `description` より）

例:

```json
{ "revision": 1 }
```

---

## 6. エラーレスポンス

公式 OpenAPI で定義されるエラーは 2 種類。

### 6.1 `AuthError` (HTTP 401)

```json
{
  "code":    "AuthError",
  "message": "認証に失敗しました"
}
```

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `code` | `"AuthError"` (enum 文字列) | ○ | 固定値 |
| `message` | `string` | ○ | 認証失敗を示すメッセージ |

発生条件:

- `Procon-Token` ヘッダ / `token` クエリの **どちらも付与されていない**
- トークンが無効、期限切れ、別試合のトークン

クライアント側実装:

- 401 を受けたら **即座に再試行せず**、試合設定再取得 (`GET /setting`) からやり直す。
- 連続 401 は運用上の異常（プロキシ・NAT 設定ミス）なので **人間に通知**。

### 6.2 `AccessTimeError` (HTTP 403)

```json
{
  "code":    "AccessTimeError",
  "message": "試合が設定される前、または試合終了後です"
}
```

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `code` | `"AccessTimeError"` | ○ | 固定値 |
| `message` | `string` | ○ | 「試合前 / 試合後」を示すメッセージ |

発生条件:

- 試合がまだ開始時刻に達しておらず、サーバが試合情報を返せないタイミング。
- 試合が完全に終了し、応答不能になった後。
- 提出受付時間外。

クライアント側実装:

- 403 を受けたら `GET /setting` の `startsAt` / `day` を見て次の行動を判断。
- `startsAt > 0 && now < startsAt` → 試合前待機
- `now > endsAt` → 試合後

### 6.3 その他のステータスコード

| コード | 意味 | 想定対応 |
|---|---|---|
| `200` | 正常 | レスポンス本体を処理 |
| `204` | (仕様にない) | `POST /agent` 系の受理応答で来る可能性あり、body なしで正常扱い |
| `400` | (仕様にない) | OpenAPI 不整合。サーバ側で 200 を返すはずなのでほぼ起こらない |
| `401` | `AuthError` | トークン再取得 |
| `403` | `AccessTimeError` | 試合時刻を確認 |
| `429` / `5xx` | (仕様にない) | 想定外。指数バックオフで再試行、上限超えたら運用に連絡 |

---

## 7. curl 例

大会運営から配布されたベース URL を `$BASE`、トークンを `$TOKEN` とする
（環境変数で実値を注入する想定）。

### 7.1 試合設定を取得（試合前）

```bash
curl -sS "$BASE/setting" \
     -H "Procon-Token: $TOKEN" \
     -H "Accept: application/json"
```

### 7.2 エージェント種別を提出

```bash
curl -sS -X POST "$BASE/agent" \
     -H "Procon-Token: $TOKEN" \
     -H "Content-Type: application/json" \
     --data '[0, 1, 0, 1]'
```

### 7.3 試合状態を取得（試合中）

```bash
curl -sS "$BASE/" \
     -H "Procon-Token: $TOKEN" \
     -H "Accept: application/json"
```

### 7.4 行動計画を提出

```bash
curl -sS -X POST "$BASE/" \
     -H "Procon-Token: $TOKEN" \
     -H "Content-Type: application/json" \
     --data '[
       [ 0, 0, 1, -1, 2, 3 ],
       [ 1, 0, 0,  1, 1,-3 ],
       [ 2, 2, 2,  0, 4,-1 ],
       [-1,-1,-1, -1, 0, 5 ]
     ]'
```

### 7.5 クエリトークン版（curl デバッグ用）

```bash
curl -sS "$BASE/setting?token=$TOKEN"
```

### 7.6 401 / 403 の再現

```bash
# トークンを付けない → 401
curl -sS -o /dev/null -w "%{http_code}\n" "$BASE/setting"

# 試合前に GET / → 403 (タイミングによる)
curl -sS -o /dev/null -w "%{http_code}\n" \
     -H "Procon-Token: $TOKEN" "$BASE/"
```

---

## 8. フィールド辞書（camelCase / snake_case 対応）

公式 OpenAPI は **全フィールドが snake_case で固定** されている。
Rust 側 (`crates/protocol/src`) も `serde` 既定挙動で snake_case のまま扱うので、
クライアント⇄サーバ間で snake_case ⇔ camelCase 変換を行う **必要はない**。

下の表は **公式 OpenAPI に出現する全フィールド名** を列挙したもの。

| フィールド名 | 型 | 出現先 | 備考 |
|---|---|---|---|
| `openapi` | string | root | `"3.1.0"` |
| `info.title` | string | root | `"Procon 37 API"` |
| `info.version` | string | root | `"0.0.0"` |
| `tags` | array | root | 4 つのフェーズタグ |
| `paths` | object | root | `/`, `/agent`, `/setting` |
| `components.parameters` | object | schemas 上部 | `PlayerAuth.tokenHeader` / `PlayerAuth.tokenQuery` |
| `components.schemas` | object | schemas 全体 | 下のスキーマ群 |
| `Procon-Token` | header string | parameters | 認証ヘッダ（ハイフン込み） |
| `token` | query string | parameters | 認証クエリ |
| `code` | string enum | `AuthError`, `AccessTimeError` | 固定文字列 |
| `message` | string | `AuthError`, `AccessTimeError` | 人間可読 |
| `revision` | int32 | `ActionAccepted` | 提出受理結果 |
| `kind` | number enum | `AgentKind`, `AgentState` | `0` or `1` |
| `pos` | int32 | `AgentState`, `Spot`, `TrafficInfo` | セル位置 |
| `fuel` | int32 | `AgentState` | 残り燃料 |
| `id` | int32 | `OtherPlayer` | 他プレイヤー ID |
| `agents` | int32[] / AgentState[] | `MatchSetting`, `MatchState`, `OtherPlayer` | 用途ごとに型が異なる |
| `brand` | int32 | `Spot` | 系列 |
| `stocks` | int32 | `Spot` | 最大在庫量 |
| `status` | number enum | `TrafficInfo.status` (= `TrafficStatus`) | 0=順調 / 1=混雑 / 2=渋滞 |
| `height` | int32 | `Map` | 8 〜 32 |
| `width` | int32 | `Map` | 8 〜 32 |
| `cells` | int32[][] (= CellType) | `Map` | 2 次元配列 |
| `startsAt` | int64 | `MatchSetting` | UNIX 秒。確定前 `0` |
| `daySeconds` | int32[] | `MatchSetting` | 各日の回答時間 |
| `daySteps` | int32[] | `MatchSetting` | 各日のステップ数 |
| `map` | Map | `MatchSetting` | |
| `spots` | Spot[] | `MatchSetting` | |
| `fuelLimits` | int32 | `MatchSetting` | 燃料上限 |
| `players` | int32 | `MatchSetting` | 参加競技者数 |
| `busyThreshold` | int32 | `MatchSetting` | 混雑基準値 |
| `jammedThreshold` | int32 | `MatchSetting` | 渋滞基準値 |
| `endsAt` | int64 | `MatchState` | UNIX 秒 |
| `day` | int32 | `MatchState` | 日番号（初日 0） |
| `others` | OtherPlayer[] | `MatchState` | 他プレイヤー一覧 |
| `traffics` | TrafficInfo[] | `MatchState` | 道路状態一覧 |
| `CellType` | number enum | `Map.cells` | `0`, `1`, `2`, `3` |

> **Camel / snake 揺れに関する注記**:
> 公式 OpenAPI は一貫して snake_case（`busyThreshold` / `jammedThreshold` 等に
> 含まれる語の境界はキャメル要素そのもの）だが、**JSON キーはキャメルでもスネークでも
> どちらも受け付けられる** 実装が一部で見られる。
> ただし本大会公式の挙動は **snake_case 固定** と仕様書から読み取れる。
> 不整合に遭遇したらサーバ実装（環境変数 `PROCON_API_CASE` 等）の影響を疑うこと。

---

## 9. 公式 OpenAPI 抜粋

`/setting`, `/agent`, `/` すべてのエンドポイントとスキーマ定義を 1 ファイルに
集約したものが `crates/protocol/tests/fixtures/procon37-openapi.json` である
（Issue #14 で `.reference/procon37/openapi.json` を fixtures 化済）。
原本は `.reference/procon37/openapi.json`（公式配布）。
要約ではなく**完全版が欲しい場合はそちらを参照**。
本ドキュメントでは要点のみを §3 〜 §8 にまとめている。

---

## 10. 参考リンク

- 公式仕様 PDF: `docs/pdf/第37回大会競技部門問い合わせと回答*.pdf`
- 募集要項: `docs/pdf/募集要項*.pdf`
- 戦略ノート: [`docs/contest/strategy.md`](./strategy.md)
- チームコンテキスト: [`docs/contest/team-context.md`](./team-context.md)
- 簡易 OpenAPI サマリ: Issue #14 の PR #66 で `docs/contest/openapi.md` を追加予定
- 写経実装: `crates/protocol/src/match_config.rs` (Issue #14, PR #66)
- 公式原本: `.reference/procon37/openapi.json`
- Issue: #14 (OpenAPI → Rust 型, PR #66), #60 (本ドキュメント)
