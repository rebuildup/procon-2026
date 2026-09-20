# 試合設定 JSON 構造説明

`procon-server` に渡す試合設定ファイルの構造です。

## トップレベル構造

```jsonc
{
  "problem": { ... },   // 問題（マップ・スポット・日程などの設定）
  "teams":   [ ... ]    // 参加チーム
}
```

| キー | 型 | 説明 |
|------|----|------|
| `problem` | object | 問題設定本体 |
| `teams` | array | 参加チーム。要素数が参加チーム数になる |

### teams 要素

```jsonc
{ "name": "任意の表示名", "token": "token-p0" }
```

| キー | 型 | 必須 | 説明 |
|------|----|------|------|
| `token` | string | 必須 | プレイヤー認証トークン |
| `name` | string | 任意 | 表示名 |

## problem フィールド

| キー | 型 | 説明 |
|------|----|------|
| `width` | int | マップの幅 W（列数） |
| `height` | int | マップの高さ H（行数） |
| `cells` | int[H][W] | 各セルの地形タイプ。行数=`height`、各行の要素数=`width` |
| `spots` | object[] | スポット（補給地点）一覧 |
| `agentStarts` | int[] | エージェント初期位置 |
| `fuelLimits` | int | 燃料積載量上限 |
| `daySteps` | int[] | 各日のステップ数。要素数=日数 |
| `daySeconds` | int[] | 各日の回答時間（秒）。要素数=日数（`daySteps`と同じ長さ） |
| `busyThreshold` | int | 道路「混雑」判定の基準値 |
| `jammedThreshold` | int | 道路「渋滞」判定の基準値 |

### cells の地形タイプ

`cells[y][x]` が座標 (x, y) の地形を表します。

| 値 | 地形 |
|----|------|
| `0` | 平地 |
| `1` | 道路 |
| `2` | 山地 |
| `3` | 池 |

### 位置インデックス（pos）の規則

`spots[].pos` と `agentStarts[]` はセル番号（1次元インデックス）で表します。
起点はマップ左上 `0`、右方向・下方向に増加する行優先です。

```
pos = y * width + x
x   = pos % width
y   = pos / width
```

例（width=4）: `pos=5` → (x=1, y=1) / `pos=8` → (x=0, y=2)

### spots 要素

```jsonc
{ "brand": 1, "pos": 8, "stocks": 5 }
```

| キー | 型 | 説明 |
|------|----|------|
| `brand` | int | 系列（整数） |
| `pos` | int | 設置セル番号（上記 pos 規則） |
| `stocks` | int | 最大在庫量 |
