# live 本番投入手順 — coverage park=far

Issue #317 の本番投入手順。offline 選抜結果は `docs/evidence/campaign-311/coverage-park/`。

## 選抜結果

| 優先 | candidate id | 状態 |
|---|---|---|
| **第1** | `coverage:budget=official:park=far` | D=198, B=358（第2軸最優先） |
| 第2 | `coverage:budget=official:park=none` | D=195, B=379（玉数優先時） |
| fallback | `multiloop:grid=2x2:walk=astar:assign=escort:budget=official:balance=on:surplus=split:fuelpair=need` | D=119（既存、互換性重視） |

## 起動コマンド

```bash
# ビルド（release profile、source SHA を埋め込む）
cargo build --release --workspace --locked

# arena window 起動（:00/:15/:30/:45 の window に自動登録）
infisical run --env=dev -- env \
  PROCON_SOURCE_COMMIT_SHA=$(git rev-parse HEAD) \
  target/release/procon coordinator \
  --arena --next --exit-after-match \
  --candidate 'coverage:budget=official:park=far'
```

- `--next` が次の `:00/:15/:30/:45` window まで待ってから `GET /setting` で登録する。
- `--exit-after-match` は 1 match で終了する（連続投入は supervisor が window ごとに起動する）。
- `--arena-evidence target/arena/runs.jsonl` で 1 match ごとの evidence を追記する。

## fallback 手順

1. **candidate が build に失敗した場合**: `coverage:budget=official:park=none` で再起動する。
2. **live で panic / 無効行動が観測された場合**: `multiloop:grid=2x2:walk=astar:assign=escort:budget=official:balance=on:surplus=split:fuelpair=need` に切り替える。
3. **window に登録できなかった場合**: 次の window を待って同一コマンドで再試行する（手動登録は `--next` を外す）。

## offline / live 同一性

PR #324 が `coverage:budget=official` の bridge parity を検証済み。
`park=far` は同一の `CoverageCandidate` に `park_risk` フラグを足した変形であり、
`CandidateBridge::from_registry_id` 経由で同じ `Solver` が使われる。
`park=none` はバイト単位で元 ID と同一。

## 注意

- debug profile の response time は競技計測ではない。本番では release profile を使う。
- 701 の D=198 は offline 計測。live では traffic / 競合により変わる。
- `park=far` は玉数を犠牲に日別種類を優先する。玉数で勝負する盤では `park=none` を選ぶ。
