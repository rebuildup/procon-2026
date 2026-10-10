# 探索テレメトリとモニタ（`--telemetry`、Issue #429）

`procon coordinator --telemetry <PORT>`（または環境変数 `PROCON_TELEMETRY`）は、探索中の候補ソルバ（`fleet` / `convoy`）の進捗を `127.0.0.1:<PORT>` のローカルダッシュボードに出します。**既定では無効**です。フラグを渡したときだけ、探索ホットループは 256 反復ごとに共有アトミックへ数値を書きます（ロック・確保・整形なし）。計測して遅くなることはありません（検証手順は下記）。

- 実装: [`crates/bench/src/telemetry.rs`](../../crates/bench/src/telemetry.rs)（カウンタ本体）と [`crates/cli/src/monitor.rs`](../../crates/cli/src/monitor.rs)（HTTP ダッシュボード）
- モニタは**読み取り専用**です。coordinator が書いたファイル（`records/<run-id>/`）と探索が更新したアトミックを読むだけで、`records/` へは何も書きません。モニタがクラッシュしても試合には影響しません（ポート確保に失敗した場合も警告を出して試合は続行します）。

## 2 ウィンドウ起動（本選マシン / Windows PowerShell）

> **STATUS: disabled-until-verified。** 以下の手順は macOS でのオフライン検証（`procon-eval --telemetry`、[`検証記録`](#検証記録)）までしか通っていません。本選で使う前に、**練習サーバーでの 1 試合分**を必ず通してください。とくに 2 の `--allow-search-cutoff` は、練習で 1 回も打ち切りの確認が取れるまで本選では渡さないこと。

ウィンドウ 1 — 通常どおり起動（`honsen.ps1` にフラグを 1 つ足すか、手動で起動）:

```powershell
# honsen.ps1 start をそのまま使い、起動後に手動で付け足す方法（レジストリ不要）:
#   .env と PROCON_TOKEN を読ませたうえで、下のコマンドを直接実行する。
.\target\release\procon.exe coordinator --server $env:PROCON_SERVER `
    --exit-after-match --candidate convoy:budget=official:budget_ms=20000:seed=42 `
    --kinds-auto --telemetry 8437
```

- `--telemetry 8437` を足すだけ。`PROCON_TELEMETRY=1` 環境変数でも同じ（`1` はポート 8437 の意味）。
- `--candidate` の値は普段のものをそのまま使う（テレメトリは candidate id に依存しない）。

ウィンドウ 2 — ブラウザ:

```text
http://127.0.0.1:8437/
```

1 ページに 3 つの要素があり、1.5 秒ごとに fetch ポーリングで更新します（フレームワークなし、単一 HTML）。

1. **日別スコア表** — `records/<run-id>/events.jsonl` を読み、日ごとに 公式観測値（日開始時の balls、判定、revision、POST 試行数、solve ms、budget ms）を出す。**公式観測値とモデル推定は列で区別され、混ぜません**。相手チームのスコアは公式 API が公開しないため、**一切推定・生成しません**（列自体が無い）。
2. **探索の実況** — day / phase / iters / target / best（モデル値）/ improvements / last improve / elapsed / budget / plans。
3. **改善曲線** — best モデルスコアの SVG スパークライン（直近 256 点のリングバッファ）。

### JSON API

| エンドポイント | 内容 |
|---|---|
| `GET /` | 上記の 1 ページ |
| `GET /telemetry.json` | カウンタ一式（`bench::telemetry::TelemetrySnapshot`） |
| `GET /table.json` | 日別表（run ディレクトリから読み直し） |
| `POST /cutoff` | 手動打ち切り（`--allow-search-cutoff` があるときだけ。次節） |

## 手動打ち切り（`--allow-search-cutoff`）— 既定は無効

```powershell
.\target\release\procon.exe coordinator --server $env:PROCON_SERVER `
    --exit-after-match --candidate convoy:budget=official:budget_ms=20000:seed=42 `
    --kinds-auto --telemetry 8437 --allow-search-cutoff
```

付ける（＝`--telemetry` も付けている）と、`POST /cutoff` が有効になります。

- **書き込み経路はこれ 1 本。** ボタン（ページ上部「今日の探索を打ち切って提出」）か `POST /cutoff` は、**現在 plan 中の日**の探索フラグを立てるだけです。プロセスを殺しません。プランを偽造しません。探索は壁時計のデッドラインと同じチェックでフラグを見て抜け、その後は普段どおりの完了経路（replay → 信頼ゲート → fallback → `POST /`）がそのまま走るので、提出されるのは「候補自身が提出したはずのプラン」です。
- **日の範囲に限定されます。** リクエストは plan 中の日と一致しないと拒否（`409`）。新しい日の開始時にフラグは必ずクリアされるので、前の日の誤って残ったリクエストが翌日を切ることはありません。
- **無効なとき**は `403` と `cutoff endpoint is disabled` が返ります。ダッシュボードのボタンを押しても何も起きません。
- 応答: `200 {"ok":true,"day":N}`、または `403/409 {"ok":false,"error":…,"detail":…}`。

### 打ち切りの安全性が担保される仕組み

1. フラグは `bench::telemetry::SEARCH` の 1 個の `AtomicBool`。`Telemetry::day_begin`（日の開始、coordinator 側の `decide` が呼ぶ）が毎回クリアします。
2. `fleet` の SA ループは 256 反復ごとの壁時計チェックと**同一箇所**で `Telemetry::tick` を呼び、真なら `break` します。`iters` モード・壁時計モードの両方に効きます。`convoy` は組込み fleet と日プールの replay で同じフラグを共有します（ネストした `decide` は深さカウンタで外側の日の境界を守ります）。
3. `break` 後の経路は変更していません。既存の `finish()` / `replay()` / `Candidate::key` / fallback がそのまま走ります。
4. テスト: `crates/bench/tests/telemetry_fleet.rs`（実探索に対して、打ち切りが 20 秒の窓を 5 秒未満で終わらせること、プランが still well-shaped であること、翌日に漏れないこと）。

## オフラインでの検証（macOS / 開発機）

```bash
# 701 ボードで fleet を 10 日分評価しつつ、カウンタとダッシュボードを実況する
cargo build -p bench --bin procon-eval
CASE=$(cat crates/bench/tests/fixtures/arena-701.json)
./target/debug/procon-eval --worker \
    --candidate-id "fleet:budget=official:iters=200000:seed=42:chains=1" \
    --case-json "$CASE" --telemetry 18444 &
# → http://127.0.0.1:18444/ を開く。iters が動き、best が上がり、
#   曲線が伸びる。POST /cutoff はその日の探索を次のチェックで止める。
```

オフライン用の最小サーバ（`bench::telemetry::spawn_demo_server`）は `/telemetry.json`、1 ページ、`POST /cutoff` だけを出します。日別表（記録の読み取り）は CLI 側のフルモニタにしかありません — eval プロセスは run ディレクトリを持たないためです。

## 検証記録（2026-10-10、macOS）

- `procon-eval --worker --telemetry 18441`（fleet iters=200000 chains=1）: iters 49.4M→108.7M、best 20365798→20365800、improvements 34→36、curve 256 点 — カウンタは動き、evaluation 結果は従来どおり（variety 20 / daily 200 / balls 484 / success）。テレメトリを付けてもスコアは変わらない。
- `POST /cutoff` → `{"ok":true,"day":0}` → 探索は次のチェックで終了し、フラグはクリア（cutoff:false）。
- `cargo test -p bench -p cli`: 603 passed / 0 failed。`cargo clippy --all-targets`: 0 警告。

## 本選での検証チェックリスト（Windows、実施するまで本選で有効化しない）

1. `--telemetry 8437` 付きで練習マッチ 1 試合。ブラウザで日別表・カウンタ・曲線が出ること。
2. 試合スコアがテレメトリ無しの同 candidate と同水準であること（response time 軸が有意に悪化していないこと。探索は 256 反復ごとのアトミック書き込みしかしない）。
3. `--allow-search-cutoff` を付けたもう 1 試合で、ボタン → `200 ok:true` → その日の提出が普段どおり行われること（拒否されない、 unanswered にならない）。
4. ボタン連打・他の日の指定（`409`）・テレメトリ無効時の `403` を確認。
5. モニタのタブを閉じても、ブラウザを強制終了しても試合が続行すること（モニタは読み取り専用）。
6. 1〜5 が通ったら、`docs/ops/honsen-runbook.md` に本番のフラグを記載し、この文書の STATUS 行を更新する。

## 非ゴール

- 相手チームのスコア・組成の推定（公式 API が公開しないため出せない。出しません）。
- モニタからの書き込み（cutoff フラグ以外。`records/` には何も書かない）。
- 複数ブラウザ・複数クライアント同時接続の想定（1 オペレータ用。1 リクエスト 1 スレッド）。
