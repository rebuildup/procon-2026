# live 投入手順 — fleet（Issue #371 / PR #372）

対象 SHA: ブランチ `371` の head（`37a9af1` 以降）。計測の根拠は同じディレクトリの `README.md`。

## 起動

```bash
git fetch origin 371 && git checkout 371
cargo build --release --workspace --locked

# 練習 Arena（:00/:15/:30/:45 の window に登録して 1 試合）
infisical run --env=dev -- env PROCON_SOURCE_COMMIT_SHA=$(git rev-parse HEAD) \
  target/release/procon coordinator --arena --next --exit-after-match \
  --candidate 'fleet:budget=official:budget_ms=10000' --kinds-auto

# 本選サーバーなど、URL とトークンを直接渡す場合
target/release/procon coordinator --server http://<host>:<port> --token <token> \
  --exit-after-match \
  --candidate 'fleet:budget=official:budget_ms=10000' --kinds-auto
```

## 車種構成

`fleet` は補給車 1 台で巡回車全員の燃料を賄う。構成は次のどちらかで指定する。

| 指定 | 動作 | 使いどころ |
|---|---|---|
| `--kinds-auto` | 開始前に「補給 1 台（先頭）」と「全車巡回」で試合全体をシミュレーションし、全車巡回が V・D を落とさず B +5% 以上のときだけ全車巡回にする。1.5 秒で打ち切り、間に合わなければ補給 1 台（種別提出の締切を逃すと試合に参加できないため短くしてある） | 既定。短期戦・燃料に余裕のある盤面で巡回車が 1 台増える |
| `--kinds 'S,P,P,P,P,P,P,P'` | 常に補給 1 台 | 自動選択を切りたいとき。`S` は必ず先頭に置く（`P,P,P,S` は 3 台チームで補給 0 台になる） |

`fleet` 以外の候補にこの構成を使わない。`coverage` / `anytime` は補給車が組の巡回車 1 台しか給油しないので、巡回車を増やすと燃料切れで D が落ちる。

## 探索予算

- `budget_ms` は 1 日あたりの探索時間の**上限**。実際の探索時間は盤面の規模（巡回車数×スポット数）で決まり、16×16 は 1 秒前後、24×24 は 3 秒前後、32×32 は上限まで使う。応答時間は順位の第 4 軸で、練習 Arena では B 同点・回答時間決着が実際に起きた（match 1925）。
- 上限は `budget_ms=10000` を勧める（32×32 は 4 秒でもまだ玉数が伸びる）。常に上限まで探索させたいときは `:adapt=off` を付ける。
- coordinator は締切の 5 秒前までしか時間を渡さない。回答時間が短い日は、渡された時間から 0.3 秒引いた範囲で探索し、時間がゼロでも 0.25 秒は探索する。
- 空きコア（最大 4）で並列に探索する。

## ログの見方

| ログ | 意味 |
|---|---|
| `agent kinds trial kinds=… variety=… daily_variety=… balls=…` | `--kinds-auto` の各試行（offline シミュレーションの結果） |
| `agent kinds selected for POST /agent … kinds=…` | 実際に提出した構成 |
| `fleet day plan day=… clean=true chains=… model_series=… replay_series=… replay_balls=… predicted_ends=[(セル, 燃料), …]` | その日の計画。`clean=true` は公式ルールのリプレイで不正なし。`predicted_ends` は翌日サーバーが返すはずの位置・燃料 |

`clean=false` が出た日は `coverage` の計画に差し替えて提出している（offline 61 盤面と fuzz 240 盤面では 0 回）。翌日の `GET /` が `predicted_ends` と食い違う場合は、エンジンと公式サーバーの差なので記録して開発へ。

## fallback

1. build できない／起動直後に落ちる: `anytime:budget_ms=5000:seed=42:budget=official` を **`--kinds` なし**で起動する（kinds を付けたままにしない）。
2. 試合途中でプロセスが落ちた場合の再起動は未検証。coordinator は起動時に `POST /agent` を送るが、開始後は 403 になるため、そのままでは続行できない可能性がある（`fleet` 固有ではなく既存の制約）。

## 既知の限界

- offline 評価は合成盤面で、他チームの交通量を含まない。live では毎日その日の道路状態で計画する。
- 全車巡回は途中で給油できない。燃料は残り日数で配分するが、渋滞で遠回りが増えると終盤の玉数が落ちることがある（公式ローカルサーバーの 4 日戦では最終日 18 玉、合計は試行の予測 113 に対し 112）。
