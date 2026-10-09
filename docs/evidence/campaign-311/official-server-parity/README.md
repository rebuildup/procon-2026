# 公式簡易サーバー燃料可否実験 — P0-C 燃料の実行可否の確定

source: `bbb7c49d1afff9ea3adf38e7c81a803d4ad598fd`（branch `342-fuel`、Issue #342 P0-C）。
公式配布の簡易回答サーバー（`docs/pdf/回答システムに関する情報・簡易版の回答用サーバー_1.zip` 同梱
`procon-server-darwin-arm64`、sha256 `cd552d93980c6b2a503c859dedc0357666823451662381cfa886260f5a81e041`）を
ローカル起動し、燃料実行可否の解釈（出発セルのみ vs max(出発,到着)）を受理判定で切り分けた。

## 環境

- macOS (Darwin arm64)、公式簡易サーバー `procon-server-darwin-arm64`（配布 zip 同梱、改変なし）
- チーム token は実験用の `local-test`（本番 token ではない）
- 8x8 盤、平地すべて + pos 1 = 山地。巡回車 1 台（`POST /agent [0]`）。スポットは pos 10 に1個（本実験では未使用）
- `daySteps = [16, 16]`、`daySeconds = 6`、`fuelLimits` はケースごと。日付・盤面は最小構成
- 注意: 試合設定スキーマ上の `daySteps >= W+H` / `fuelLimits >= daySteps[0]` は簡易サーバーでは強制されない。
  これは境界値実験のため意図的に `fuelLimits = 0..2` を使っている

## 手順（再現）

```bash
python3 scripts/campaign/fuel_boundary_probe.py \
    --server-binary <公式簡易サーバーバイナリ> \
    --output-dir <空のディレクトリ>
```

各ケースでサーバーを1回起動し、`GET /setting` → `POST /agent [0]` → 試合開始待ち →
day 0 に計画を `POST /` → daySeconds 経過後に `GET /`（day 1 開始時点 = day 0 終了時点の燃料・位置）。
`cases/<name>/config.json` に盤面、`result.json` に全 HTTP 往復、`server.log` にサーバー出力、
`summary.json` にバイナリ digest と要約。

計画はいずれも重み付きステップ合計が `daySteps=16` に一致するよう待機ランでパディングする
（平地移動 2 步、山地移動 3 步、待ち `-N` で N 步）。

## 結果（verbatim）

| ケース | day 0 計画 | 意味 | day0 answer | day1-start (pos, fuel) |
|---|---|---|---|---|
| A_plain1_to_mountain | `[[2, -14]]` | fuel=1, 平地→山地 | `{"revision": 1}` **受理** | `pos=1, fuel=0` |
| B_mountain1_to_plain | `[[5, -13]]` | fuel=1, 山地→平地 | `{"revision": -1}` 拒否 | `pos=1, fuel=1` |
| C_plain2_to_mountain | `[[2, -14]]` | fuel=2, 平地→山地 | `{"revision": 1}` 受理 | `pos=1, fuel=1` |
| D_plain1_to_plain | `[[3, -14]]` | fuel=1, 平地→平地 | `{"revision": 1}` 受理 | `pos=9, fuel=0` |
| A2_plain0_to_mountain | `[[2, -14]]` | fuel=0, 平地→山地 | `{"revision": -1}` 拒否 | `pos=0, fuel=0` |
| E_step_mismatch | `[[2]]` | 步合計 2 ≠ 16 | `{"revision": -1}` 拒否 | `pos=0, fuel=10` |
| F_wait_only | `[[-16]]` | 待機のみ | `{"revision": 1}` 受理 | `pos=0, fuel=10` |

方向コード: `2` = 右、`3` = 右下、`5` = 左。`-N` は N 步の待機ラン。

## 解釈の確定

決定的ケース **A**（fuel=1, 平地→山地）が公式に **受理** された。
Q&A Q11（表1「燃量」= 各地形タイプ**から**移動するための燃料消費量）および
競技要項「移動命令を受けた時点での地形に応じた燃料を消費」どおり、
**実行可否・消費ともに出発セルの燃料コストのみ**で判定される。

| 解釈 | A (fuel=1, 平地→山地) | B (fuel=1, 山地→平地) | C の移動後燃料 | 判定 |
|---|---|---|---|---|
| 出発のみ（spec-text） | 受理 | 拒否 | 2-1=1 | **一致** |
| max(出発,到着) を要求・出発課金（旧 conservative） | 拒否 | 拒否 | 2-1=1 | A と不一致 |
| 到着のみ（pre-#245） | 拒否 | **受理** | 2-2=0 | A/B と不一致 |

補足:

- **B** は出発セル（山地, 2）を払えないため拒否。到着のみ解釈なら受理されるはずで、除外される。
- **C** の移動後 fuel=1 は課金が**出発**（平地 1）であることを示す（到着 2 なら 0 になる）。
- **A2** は出発コスト 1 も払えないため拒否。`>=` の閾値（厳密 `>` ではない）は従来どおり。
- **E** は步合計不一致で回答全体が拒否（`revision = -1`）。Q6 の「規定ステップ数と一致しない場合は回答全体をリジェクト」。
- day 1 開始時点の `fuel` が前日終了時点の残りであることは Q19 と一致する（日をまたいで自動補給はされない）。

## 未観測・未完

- **採取タイミング（Move+Wait でスポット玉が公式ステップで入るか）**: 簡易サーバーの `GET /`
  は `kind/pos/fuel` のみを返し、玉数・スコアを公開しないため wire 上で観測できない。
  本実験では未確認。Q6/Q7 の記載（反映フェーズの取得）を正本とする。
- 外部 arena 未接続。スコア・順位の検証ではない。
- 簡易サーバーは公式本番サーバーと同一 sha256 の配布バイナリだが、本選当日の運用差は保証されない。
