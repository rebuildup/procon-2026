# 一発起動・切り替え runbook — #317 選抜候補（PR #324）

Issue #317 の「本番前チェックリスト / 起動コマンド / 失敗時の切り替え / 既知リスク」。**現段階の検証状態は bridge parity とローカル公式バイナリ受理まで**で、holdout・release 計測・最終 winner 認定は未実施。ここに書く起動・切り替えコマンドはすべて実行済み経路の再実行用であり、最終推奨の確定ではない。

## 検証状態マトリクス（この branch）

| candidate | offline 13盤順位 | offline/live 同一計画 | 公式バイナリ live CLI 受理 | 実 arena |
|---|---|---|---|---|
| `search:trials=16:budget=official` | 1位（平均順位 1.69） | 未検証（bridge parity テスト対象外） | 済（7台・低燃料、`../official-cli-317-search-seven/`） | 未 |
| `search:trials=64:budget=official` | 2位（2.15） | 未検証 | 未 | 未 |
| `coverage:budget=official` | 3位（2.31） | **済**（`cargo test -p cli --lib campaign_candidate`） | 済（5台・P,S、`../official-cli-317-coverage/`） | 未（park 変形の arena 実績は #331 側） |
| `multiloop:kmeans=5:seed=42:walk=astar:assign=all:budget=official` | 4位（3.77、baseline） | — | 未 | 未 |

順位表の根拠と制約は `README.md` を参照。**holdout 未実施のため winner 認定はしない。**

## 1. ビルド（一発）

```bash
cargo build --release --workspace --locked
```

source SHA を埋め込む起動は以降のコマンドで `PROCON_SOURCE_COMMIT_SHA=$(git rev-parse HEAD)` を付ける。

## 2. オフライン順位表の再現（一発）

```bash
PROCON_SOURCE_COMMIT_SHA=$(git rev-parse HEAD) ./target/release/procon-eval \
  --cases all \
  --case-files crates/bench/tests/fixtures/arena-701.json \
  --generated 'open:seed=17:days=3,road-heavy:seed=17:fuel=tight,many-series:seed=29' \
  --candidates 'multiloop:kmeans=5:seed=42:walk=astar:assign=all:budget=official,coverage:budget=official,search:trials=16:budget=official,search:trials=64:budget=official' \
  --baseline 'multiloop:kmeans=5:seed=42:walk=astar:assign=all:budget=official' \
  --output-dir target/eval-317-tick-fixed
```

既存 output を上書きしない。時間計測は release profile で行う。

## 3. 公式バイナリでの live CLI 受理確認（一発）

```bash
# normalizer tick 修正の公式受理（最小ケース）
python3 scripts/campaign/official_tick_probe.py \
  --server-binary /path/to/procon-server --output-dir target/official-tick

# coverage（5台 P,S、701、10日）
python3 scripts/campaign/official_cli_smoke.py \
  --server-binary /path/to/procon-server --cli target/release/procon \
  --candidate 'coverage:budget=official' --kinds 'P,S' \
  --output-dir target/official-cli-coverage

# search:trials=16（7台 P,P,P,S,P,P,P、燃料12、10日）
python3 scripts/campaign/official_cli_smoke.py \
  --server-binary /path/to/procon-server --cli target/release/procon \
  --candidate 'search:budget=official:trials=16' --kinds 'P,P,P,S,P,P,P' --seven-agents \
  --output-dir target/official-cli-search-seven
```

`--output-dir` は必ず新規ディレクトリ（作成に失敗したら名前を変える）。`/path/to/procon-server` は公式配布の `procon-server-linux-amd64`。全 run exit 0・全 revision 受理が合格条件。

## 4. live 投入（一発）

```bash
# 第1: offline 1位。ただし bridge parity 未検証 — 報告時にその旨を添える
infisical run --env=dev -- env PROCON_SOURCE_COMMIT_SHA=$(git rev-parse HEAD) \
  target/release/procon coordinator --arena --next --exit-after-match \
  --candidate 'search:trials=16:budget=official' \
  --arena-evidence target/arena/runs.jsonl

# 切り替え: bridge parity 検証済みの coverage
infisical run --env=dev -- env PROCON_SOURCE_COMMIT_SHA=$(git rev-parse HEAD) \
  target/release/procon coordinator --arena --next --exit-after-match \
  --candidate 'coverage:budget=official' \
  --arena-evidence target/arena/runs.jsonl
```

等価: `infisical run --env=dev -- just arena-next --candidate '<id>'`（`justfile` の `arena-next`）。token は CLI 引数にしない（Infisical が `PROCON_ARENA_TOKEN` を注入）。`--next` が `:00/:15/:30/:45` の window まで待って登録し、1 match で終了する。連続投入は window ごとに再起動。

kinds は `--kinds` で明示するか既定（非 wait ソルバは P,S 交互）。公式 kinds 判断は別途。

## 5. 失敗時の切り替え

1. **search が window に間に合わない / live で plan が受理されない** → `coverage:budget=official` に切り替えて再起動（offline/live 同一計画は検証済み、response は 1 桁速い）。
2. **coverage も不可** → `multiloop:kmeans=5:seed=42:walk=astar:assign=all:budget=official`（この13盤の baseline。offline 全 run success）。もしくは stack 親 #313 / PR #322 の運用構成。
3. **パニック / 無効行動を観測** → 該当 match の `target/arena/runs.jsonl` を保存し、candidate を fallback に下げて次 window。手記録を Issue #317 に残す。

## 6. ロールバック

- 即時: candidate を fallback 表記に変えればコード変更不要。id は `bench::CandidateRegistry` 経由で解決され、未知 id は fail-closed。
- コード巻き戻し: stack 親は branch `313`（PR #322、head `badc5df089d08948e83bf8c3f695a724a8595711`）。`git revert` は PR #324 のマージコミットで行う。tick normalizer のみ戻す場合は `8728ed5`（親 `c188211`）を revert。
- 本 branch の release 統合は Supervisor のみ行う。main / `release-0-3-0` への直接 push はしない。

## 7. 既知リスク

- **holdout 未実施** — 同一 corpus は campaign 中に触った盤を含む。1位の search を「実戦で最強」と報告しない。
- **search の応答時間** — 701 で trials=16 合計 1.7 s、trials=64 は 6.9 s（計測 profile 未記録）。live 回答窓は wall-clock で切られるため、trial 数を上げても玉数が伸びるとは限らない。
- **bridge parity は coverage のみ** — search の offline/live 同一計画は未検証。live 投入時は上記の旨を報告に残す。
- **score の比較限界** — canonical MatchSim は要素時計。公式 tick 受理の確認は `../official-tick-317/` と CLI smoke 側の evidence に分離済み。実外部 arena での順位確認は未。
- **計測 profile** — `tick-fixed-comparison-317` の response time に profile 記録が無い。release 計測は後続。

## 関連

- 順位表: [`README.md`](README.md)
- 公式受理 evidence: [`../official-tick-317/`](../official-tick-317/) [`../official-cli-317-coverage/`](../official-cli-317-coverage/) [`../official-cli-317-search-seven/`](../official-cli-317-search-seven/)
- park 変形（#331）の runbook: `docs/evidence/campaign-311/coverage-park/LIVE-RUNBOOK.md`（別 track。本 runbook の winner 認定はしない）
