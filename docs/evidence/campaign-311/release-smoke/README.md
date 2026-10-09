# release smoke — candidate registry + 公式回答受理

Issue #342。release SHA `bbb7c49` 上で、対戦用候補の registry 解決と公式簡易回答サーバーへの受理を一括で確認する **smoke**（計測ではない）。実行は `scripts/campaign/release_smoke.py`。

- source: `bbb7c49d1afff9ea3adf38e7c81a803d4ad598fd`
- platform: Darwin arm64 (macOS 27.0.1)
- loadavg at run: 3.15 / 4.12 / 4.03
- date: 2026-10-09

## 使い方

```bash
# ビルド済み release バイナリを検出し、無ければ cargo build --release
python3 scripts/campaign/release_smoke.py

# step 1 のみ（公式サーバー不要）
python3 scripts/campaign/release_smoke.py --skip-cli

# サマリ JSON を任意パスにも書き出す
python3 scripts/campaign/release_smoke.py --json-out target/release-smoke/summary.json
```

- 一時成果物は `target/release-smoke/`（git-ignored）。サーバーバイナリは vendored zip から `target/official-server/` に展開され **commit されない**。
- 終了コード: 必須 step の失敗で非ゼロ。probe-only 事実（下記）は成否を変えない。

## Step 1 — candidate resolution

`procon-eval --worker --candidate-id <id> --case-json <tiny>` を最小 1 日 8x8 盤（3 agents / 3 spots / daySteps=[16]）で実行。`bench::CandidateRegistry::builtin()` による parse → schema 検証 → `build_solver` → `MatchSim` 1 日完走までを通す。

| candidate id | 区分 | 結果 |
|---|---|---|
| `coverage:budget=official:park=far` | 必須 | **PASS** — status=success, scored=true |
| `coverage:budget=official:park=none` | 必須 | **PASS** — status=success, scored=true |
| `anytime` | probe（事実記録のみ） | **NOT constructible** — `unknown algorithm "anytime"` |
| `coverage:budget=official:park=far:fuelaware=true` | probe（事実記録のみ） | **NOT constructible** — `algorithm "coverage" does not take parameter "fuelaware"` |

事実記録（修正しない）:

- `anytime:*` は builtin registry に登録が無く、`CandidateError::UnknownAlgorithm` で落ちる。
- `coverage` のパラメータは `budget`（必須, `official`）と `park`（`none`|`far`|`risk`）のみ。`fuelaware` は未知パラメータ。`nearest` 側の `fuel_aware` ですら schema 非公開（Issue #189 で child 候補へ先送り）。

## Step 2 — 公式 CLI 回答受理

`scripts/campaign/official_cli_smoke.py` を vendored 公式簡易サーバーに対して実行。

- サーバー: `docs/pdf/回答システムに関する情報・簡易版の回答用サーバー_1.zip` 内 `簡易サーバー.zip` の `procon-server-darwin-arm64`
- sha256: `cd552d93980c6b2a503c859dedc0357666823451662381cfa886260f5a81e041`（展開後に検証）
- CLI: `target/release/procon`（本 worktree の release build）
- 候補: `coverage:budget=official:park=far` / `--kinds P,S` / 701 盤 10 日 / daySeconds=3s に短縮

結果: **PASS**

| 日 | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| revision | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 |

- 10 日すべて `revision >= 1` で受理、`--exit-after-match` で exit 0。
- `POST /agent` で kinds 受理（`P,S,P,S,P`）。
- daySteps は 701 の公式値、daySeconds は 3 秒に短縮。1 チームのみ。公式形式の提出が最後まで通ることの確認であり、**arena スコア一致ではない**。

## サマリ（この run）

```
candidate build  : PASS
  - coverage:budget=official:park=far: PASS
  - coverage:budget=official:park=none: PASS
probe facts (informational):
  - anytime: NOT constructible — unknown algorithm "anytime"
  - coverage:budget=official:park=far:fuelaware=true: NOT constructible
    — algorithm "coverage" does not take parameter "fuelaware"
official CLI     : PASS  revisions=[1, 2, 3, 4, 5, 6, 7, 8, 9, 10] (min=1, floor=1)
overall          : PASS
```

機械可読サマリは同梱 `summary.json`（パスは repo 相対、トークン・ユーザー名を含まない）。raw log（`cli.log` / `server.log` / `requests.json` / `config.json`）は `target/release-smoke/official-cli/` に残るが commit しない。

## 制約・既知の非対称

- ローカル公式バイナリ・1 チーム・短縮ウィンドウ。arena ランク／スコア一致の検証ではない。
- `anytime:*` / `coverage:…:fuelaware=true` は現状 construct 不可。registry に将来登録されたらこの smoke の probe 行が事実を書き換える。
- 外部 arena には接続しない（spawn contract 上禁止）。

## 再現

```bash
python3 scripts/campaign/release_smoke.py
# または個別に:
cargo build --release --locked --bin procon --bin procon-eval
python3 scripts/campaign/official_cli_smoke.py \
  --server-binary target/official-server/simple-server/procon-server-darwin-arm64 \
  --cli target/release/procon \
  --case-file crates/bench/tests/fixtures/arena-701.json \
  --candidate 'coverage:budget=official:park=far' \
  --seconds 3 \
  --output-dir target/release-smoke/official-cli
```
