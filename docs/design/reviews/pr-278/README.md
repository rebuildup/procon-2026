# PR #278 — 盤面での視認性

実装・録画元 `211c4f0`。比較基準は直前のレビュー対象 `bc8ae3b`。Chromium / SwiftShader / Noto Sans CJK JP。検証サーバはRECORD_PORT=5277で分離する。

- [8記号×4地形の変更前後・通常表示](board-none.png)
- [グレースケール](board-achromatopsia.png)
- [Protanopia](board-protanopia.png) / [Deuteranopia](board-deuteranopia.png) / [Tritanopia](board-tritanopia.png)
- [全36記号・実寸15/24/48px](icons-before-after.png)
- [マップ上の3family](families-15px.png) / [公式のスポット配置例と空枠](sample-figure2-padding.png)
- [マップ](home.png) / [録画](home.webm)
- [Replays](replays.png) / [録画](replays.webm)
- [Solvers](solvers.png) / [録画](solvers.webm)
- [Live](live.png) / [録画](live.webm)
- [設定色のコントラスト](board-contrast.json) / [64px共通atlasの重心・bbox・面積](icon-geometry.json)
- [GPU・Canvas・SVG比較](shared-renderer-comparison.png) / [描画計測](renderer-performance.json) / [撮影manifest](manifest.json)

記号は32→40unit（25%拡大）。内部線を減らし、黒い輪郭を1.1→0.9unitに軽くした。全familyとagentに白casingを付ける。記号の背後は模様を省き、周縁に疎な地形模様を残す。系列色は導入済みPrimerの鮮明なdata-vis/display色を用い、地形の役割・公式例の位置と系列を維持する。元のbitmap色との一致は主張しない。

設定色のコントラスト: 黒輪郭/白casing約15.58:1、6系列色/黒輪郭の最小約3.13:1、地形模様/地形色の最小約3.03:1。色の組合せの計測であり、全アクセシビリティ要件への適合保証ではない。色覚シミュレーションはChromiumのEmulationであり、各利用者の視覚を保証するものではない。記号の形と地形模様も併用し、画像を目視確認する。

マップは公式の地形・スポット例で、絵柄はviewerのdesign。Live入力はfixture。実試合の録画ではない。CPU submissionは実機FPS保証と区別する。詳細: [UI-277](../../validation/UI-277.md)。

最終検証元 `211c4f0`: frontend check / lint / format / unit10件 / build成功、ブラウザ21件成功。canonical抽出一致も成功。4録画と比較画像を更新し、PR本文にはGitHub添付として掲載する。
