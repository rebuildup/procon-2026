# Viewer の性能・互換性検証

camera 入力は最新状態を保ったまま animation frame ごとにまとめる。停止中の描画ループは回さず、route を離れたら予約を取り消す。WebGPU の初回 frame 検証は描画集約と別に実行し、初期化失敗・device loss 時は Canvas に切り替える。

## 再現可能な回帰検証

```bash
cd packages/frontend
bun install --frozen-lockfile
bunx playwright install chromium firefox
cd ../..
just test-viewer-frames
```

opt-in のローカル検証で、通常の `just ci` とは別に実行する。専用 port 5399 を使い、既存 server を再利用しない。出力は `target/frame-tests/results.json` と各 test の `frame-burst.json` attachment。

| Browser / renderer | DPR |
| --- | --- |
| Chromium / Canvas | 1、2 |
| Chromium / WebGPU（SwiftShader） | 1、2 |
| Firefox / Canvas | 1、2 |

計 6 ケースで次を確認する。

- 同一 task の 200 keydown が即時描画 0、次 frame の描画 1 に集約される。
- 最終 camera が正しく、描画した画像が camera 更新・reset・replay・theme・sample・padding に追従する。
- viewport の resize で canvas の backing size が変わる。
- route 変更後に破棄した Canvas を描画せず、browser の pageerror がない。
- WebGPU device を破棄して device loss を発生させても、Canvas が非白紙で表示され、操作を継続できる。
- 待機 frame に追加描画を行わない。

時間指標は attachment に保存するが、環境依存のため ms の固定閾値で gate しない。入力処理時間と、draw 後の次 rAF までの時間を区別する。どちらも GPU timestamp、presentation latency、CPU 使用率の代わりにはならない。

## 改善の調査根拠

Issue [#279](https://github.com/rebuildup/procon-2026/issues/279) の調査は source `afda45cb2d68e4f044ad7a315ebb25c08613bd67`、Ubuntu 26.04 headless / Playwright Firefox 155 / 1280×720 / DPR 2 / デモ 48 セル、3 試行で実施した。

基準実装は 200 keydown を一つの task から送ると 200 回描画し、1,333–1,720 ms を占有した。rAF 集約の試作は 1 回描画となり、draw 後の次 rAF まで 25–26 ms。合成ストレス試験であり、通常のタイピングの負荷ではない。また、この数値は後続の素材変更を含む現在の source の測定値ではない。現在の回帰検証は実際の描画回数と最終表示を確認する。

CSS 容量は [#280](https://github.com/rebuildup/procon-2026/issues/280)、実 GPU / NixOS の比較は [#281](https://github.com/rebuildup/procon-2026/issues/281) で追跡する。

## NixOS / Firefox の実端末検証

headless SwiftShader の成功は GPU API / pipeline を通った証拠であり、実 GPU の性能や NixOS 互換性の証拠ではない。Playwright Firefox も対象端末の NixOS Firefox と同じ build ではない。

対象端末で以下を記録する。

1. NixOS / Firefox の version と通常版・ESR の区別、Wayland / X11、CPU / GPU / driver、DPR、解像度、refresh rate、電源状態。
2. `about:support` の Graphics 情報。共有時は個人情報を含む全体 dump を避ける。
3. `nix shell nixpkgs#vulkan-tools -c vulkaninfo --summary` と `/run/opengl-driver/share/vulkan/icd.d/`。設定の根拠は [NixOS Graphics](https://wiki.nixos.org/wiki/Graphics) を参照し、software rendering と実 GPU を区別する。
4. HTTPS / localhost で API → adapter → device → pipeline → 非白紙の初回 frame を段階別に確認。
5. 同じ production build・盤面・viewport で Canvas / WebGPU を各 3 回以上比較。pan・zoom・replay・theme・resize・長時間 live を確認し、Firefox Profiler と OS 側の CPU / GPU / memory を記録する。

Mozilla は [Linux の Firefox ESR 153 で WebGPU を正式サポートしていない](https://bugzilla.mozilla.org/show_bug.cgi?id=2063383)と明記し、Linux 固有の WebGPU security fix を backport しないとしている。ESR 153 に `dom.webgpu.enabled` の強制変更を求めず、Canvas で操作可能にする。これは通常版 Linux Firefox 全体が未対応という意味ではない。

実競技盤面・実端末が未検証ならその範囲を明示する。デモの計測だけから本番規模や実 GPU の優位を保証しない。
