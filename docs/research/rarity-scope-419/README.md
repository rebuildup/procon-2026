# 希少系列優先の作用範囲を分離する（#419）

#417 / PR #418では、`413@d87c66d` の `rarity=2` が45盤面でV差0 / D−4 / B−4だった。しかし希少系列優先が初期経路とSAの翌日推定に共有されていたため、初期経路だけの効果は分からない。

このcandidateは `rarity_scope=initial|both` により作用範囲を分離する。省略時の `both` は親candidateの動作を維持する。`initial` は当日と翌日探索の初期経路にだけ適用し、固定経路の評価・翌日種類数推定には適用しない。既定の `rarity=0` は維持する。

比較する3候補は `fleet:budget=official:iters=20000:chains=4:seed=42` に、それぞれ `rarity=0`、`rarity=2:rarity_scope=both`、`rarity=2:rarity_scope=initial` を指定する。車種は `S,P,P,P,P,P,P,P`、canonical evaluatorは `--jobs 1`。hard12と既存holdout24を同一sourceで評価する。V/D/B同点と回答時間込みの勝敗を分け、共有Macの時間差を実機での優劣と扱わない。

採用は公式V→D→B→−Tによる測定後に判断する。固定経路の評価不変・既定モードの再現性・合法性は性能改善とは別に確認する。親の否定結果から子の優劣を推測しない。別seed、未使用holdout、観測交通、Windows実機の検証前には本番採用を主張しない。

親Issue / PRは #413 / #414、baseは `413@d87c66d169087e917881fb4eb676a3b2cac8b06c`、target releaseは `release-0-5-1`。旧結果のsource SHAを再解釈せず、新しいsource SHAを評価対象にする。
