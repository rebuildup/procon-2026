# PR #278 アイコン確認資料

録画・標本元: `cbd0a2b`。Chromium / SwiftShader / Noto Sans CJK JP。

- [全36記号の変更前後・実寸15/24/48px](icons-before-after.png)
- [マップ上の3family](families-15px.png) / [スポット配置例と空枠](sample-figure2-padding.png)
- [マップ](home.png) / [録画](home.webm)
- [Replays](replays.png) / [録画](replays.webm)
- [Solvers](solvers.png) / [録画](solvers.webm)
- [Live](live.png) / [録画](live.webm)
- [64px共有atlasの重心・bbox・面積](icon-geometry.json) / [GPU・Canvas・SVG比較](shared-renderer-comparison.png)
- [描画計測](renderer-performance.json) / [撮影manifest](manifest.json)

変更前はマージ済みPR #276の素材に固定。変更後は主要familyの12記号を描き直し、全36記号の重心と余白を調整した。共通atlas画素で各軸の重心誤差0.2unit未満、四周の余白1.25unit以上を確認する。見た目と識別性は数値だけでは決めず、実寸標本とマップを目視で確認した。GPU/Canvas共通素材の一致と六辺の黒線・白い隙間を維持する。

マップは公式資料の地形・スポット例で、絵柄は合流済みdesign。Live入力はfixture。実試合を表示する録画ではない。CPU submission計測は実機FPS保証とは区別する。詳細は[UI-277](../../validation/UI-277.md)。

最終検証: `cbd0a2b` でfrontend型チェック・lint・format・単体10件・build成功。ブラウザ20件成功（4録画、全36記号の実atlas重心/余白、15pxの系列識別、GPU/Canvas比較、六辺の黒線と白い余白を含む）。canonical抽出一致も成功。
