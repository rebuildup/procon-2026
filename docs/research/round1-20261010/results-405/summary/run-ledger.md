# Run ledger（orchestrator 管理・逐次記録）

## P1 embed 予算ラダー（s9012 単盤面・同一実行内・jobs 1・nice 15）
- 開始 20:58 / 終了 21:01、2 run、ウォール 2m15s、load 約 0.1（低負荷）
- `fleet:budget=official:budget_ms=2500:adapt=off:seed=42` → V/D/B = 27/189/**416**、maxday 2.50s、success
- `fleet:budget=official:budget_ms=5000:adapt=off:seed=42` → V/D/B = 27/189/**408**、maxday 5.00s、success
- 証跡: `wsl-10s/p1-embed-rung/runs.jsonl`（binary sha256 a6737883…、source merge 4c721871）

### 判定（分析担当の基準に照合）
- 予測（P1 的中）: @2500 ≈ 369±15、@5000 ≈ 388±10 → **両方とも外れ**（実測 416 / 408）
- convoy@10s 369 < fleet@2500 416（差 47）、convoy@20s 388 < fleet@5000 408（差 20）→ **embedded 予算不足だけでは劣後を説明できない。beam/pool 側の寄与（(b) 系）が実在**
- 注意: fleet@2500 416 > fleet@10s 396-406（b10s/b2 実測）— 低負荷実行の単サンプルで B 変動 ±20 以上あり。絶対値比較は同一実行内で行うこと
