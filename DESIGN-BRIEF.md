## 2026-10-03 最新レビュー: 盤面での視認性

問題由来の役割・配置・色相の意味を維持し、viewerの系列色は彩度と明度を再調整する。過去の「色変更禁止」「公式図の実測値をそのまま使う」は、今回のユーザー指定により固定条件ではなくなった。操作UIのPrimer色は変更しない。系列色は導入済みPrimer Primitivesのlight data-vis/display色から選び、map内ではlight/dark共通の一定色で表示する。原資料のbitmap色との一致は主張しない。

識別の主役は大きい輪郭と色面とする。小さい内部線を減らし、map上の記号を32から40unitへ拡大する。全familyとagentにも同じ白いcasingを付け、色だけで系列を判定しない。地形模様は密度とコントラストを下げ、記号の背後では省き、周縁には地形判別の手がかりを残す。模様の線は装飾的な手がかりであり、非text境界の3:1判定対象と混同しない。hex枠・記号の黒輪郭・白いcasingの必要な境界を計測する。

> 最新レビュー反映: visualizer は Primer utility の枠でセクション化する。地形・境界・地図記号・agent は同じ WebGPU シーンで描画し、GPU 不可時は同じ記号アトラスの Canvas 描画へ退避する。地図内に SVG DOM を重ねない。Reset view は右側 View パネルのラベル付き Primer Button に置く。Replays の操作パネルは維持する。

# Design Brief — procon-2026 viewer

## 2026-10-03 レビューによる配置規則の更新

ユーザーの明示指示により、以下を過去の12列指定・sidebar指定・PR #272 の Overview 一覧化より優先する。

- 主領域は全ルートでビジュアライザ。状態・対象・操作・スコア・接続情報は適切な情報パネルへまとめる。
- グリッドは導入済み Primer CSS の PageLayout（content / pane、標準幅・ガター・stackRegions）を使う。独自の列比率・breakpoint を追加しない。
- ヘッダーはアプリ名、必要なモード切替、theme 操作のみ。意味のない組織名・repository path・重複する route header を置かない。
- パネルは情報単位の題名と内容を持つ。装飾的な背景帯・独立した詳細ヘッダーを足さない。
- ステータスは情報パネルのラベル付き行に置き、Octicons と Primer semantic color を使う。独自 dot / hollow marker / badge を設計しない。Issue / PR 専用 StateLabel の文脈を試合状態へ転用しない。
- UI 文字サイズと選択状態は Primer の既定に戻す。HUD 11/15px の独自適用は行わない。map の符号・geometry は別責務。今回の厳密な Primer 使用指示により quiet focus の独自上書きも撤去する。


> 生成: design-intent skill。実装・proposal 作成前に visual direction を固定するための共有 input。
>
> ## ★ v5 — **デザインシステムは撤廃**（最重要）
>
> **project 独自のデザインシステムは作らない。デザインシステムは Primer Primitives + Primer Octicons そのものとする。**
> トークン・色・radius・spacing・border・icon は**すべて Primer / Octicons から取る**。project 独自値の新設を禁止する（例外は「何が合わないか」を記録した場合のみ）。
>
> 自前で設計するのは **Primer が持っていないものだけ**:
>
> 1. **ビジュアライザ** — hex map の符号（モチーフ起点の記号）と地形・系列・agent の色割当
> 2. **UX** — interaction の状態モデル・キーボード・motion
> 3. **大まかなレイアウト** — shell と route の骨格（詳細 spec ではなく骨組み）
>
> **`design-system.html` は廃止する。** そこに含まれていたトークン表・component gallery・typography scale は Primer に存在するので不要。**符号と map の色だけ**を `visualizer.html` へ移設する。
>
> これは「自前の DS を育てるコストとドリフト」をやめる判断。過去の改訂でトークンの食い違い・グリッドの崩れ・色のドリフトが繰り返し発生したため、**根を絶つ**。

### 操作とコンポーネント使用の追記

- 再生操作は Octicon と表示ラベルのある Primer Button を使う。再生・一段移動・先頭末尾・速度・位置を一つの wrap 列へ混在させない。関連する移動操作は BtnGroup、速度はラベル付き radio group として揃える。theme 操作も表示ラベルを持つ。
- Primer component の padding / font / border / radius / shadow / focus / animation を project CSS で上書きしない。CSS は viewport の scroll ownership と map geometry のみに限定する。
- GitHub.com 全体との pixel 完全一致を宣言しない。導入版 Primer の component CSS / token 一致、用途・グループ構成、実画面と操作を別々に検証する。native range は Primer component ではなく既存 scrub の native control として扱い、同一とは報告しない。

## 実装層 — SolidJS に載せる形（v5 追記）

Primer は **SolidJS ポートを持たない**（org に `react` / `view_components` / `rails_forms` のみ。`@primer/octicons-solid` / `solid-primer` は npm に無い）。よって **Primer React の component は使わない**。使うのは**フレームワークに依らない層だけ**で、これは SolidJS の JSX から普通に扱える。

### 採用する 3 パッケージ（すべて MIT・実在確認済み）

| 用途 | パッケージ | SolidJS での扱い |
|---|---|---|
| **見た目** | `@primer/css`（22.x — "The CSS implementation of GitHub's Primer Design System"） | class をそのまま載せる |
| **トークン** | `@primer/primitives` | CSS カスタムプロパティとして読む。値を自前で写さない |
| **icon** | `@primer/octicons` | **inline SVG として JSX に書く**。`currentColor` 継承 |

### 行動（フォーカス・キーボード・ARIA）は `@kobalte/core`

`@kobalte/core` は **SolidJS 専用・unstyled by design**（MIT、実在確認済み）。無装飾が設計思想なので、**Primer CSS の class を乗せるのが本来の使い方**。これで「SolidJS に載った GitHub デザインシステム」が成立する。

補助として `@corvu/utils` / `@solid-primitives/*`（いずれも実在）を必要に応じて使う。

### 分担の規則

- **静的な部品は自前 component を作らない** — button / Box / Label / 表 / nav などは `@primer/css` の class で足りる。Solid component で包むのは **props と状態を渡すために必要な最小限**に留める
- **React 専用の行動 component だけ実装対象**になる — Overlay / ActionMenu / Dialog のような「React での行動込み」は CSS では再現できない。ここは `@kobalte/core` + Primer class で作る
- **トークン値を自前の CSS に書き写さない** — `@primer/primitives` の変数を参照する。写した瞬間にドリフトする
- icon は Octicons の path を **最小改変**で inline にする。`play` に相当する glyph は Octicons に無いので `triangle-fill` を使う（確定済み）

### なぜこれが v5 と整合するか

v5 は「自前のデザインシステムをやめる」判断。実装層でも同じで、**自前 component / 自前 CSS / 自前 token を作らず、Primer の資産に乗る**。作るのは Primer が持っていないビジュアライザの記号・interaction モデル・骨格レイアウトだけ。この対応関係を崩さないこと。

## Product context

- **Category**: competitive programming contest 用の match viewer / analysis tool（SaaS 的 marketing surface ではなく application）
- **Primary user**: コンテスト参加チーム（自分の solver と対戦相手の挙動を読み解く）、および会場で match を見ている観戦者。**desktop 前提**、venue の laptop / 外付けディスプレイ
- **Usage context**: 試合中は `/live` を常時睨めばり、休憩中に `/replays` で巻き戻して確認。**確認作業が主で、操作作業は従**。意思決定時間が短い
- **Defining screen / interaction**: `/live` の match-state（DAY / REVISIONS / SOLVER / SERVER / LAST UPDATE）と hex map、および `/replays` の再生コントロール
- **Stack**: SolidJS + SVG/WebGPU canvas。**UI 部分は DOM/CSS、地図だけ canvas/SVG** — この 2 層の境界を視覚的に成立させることが設計課題
- **性能制約**: 試合中の機器で即応する。icon は**ネットワーク不要・RAM 軽量の最適サイズ SVG のみ**。画像ファイル・icon font・外部配信は使わない

## Design intent

- **Emotional register**: 精密・静か・時間圧力下で迷わない。計器盤・タイミングボード寄りで、gamey にも marketing 的にもしない
- **Density**: 高。数値（日数・スコア・改訂番号・時刻・solver 名）が主要 reading surface。ページ外周は余白を取るが、**HUD と表は詰める**
- **Visual character**: 低装飾。**面の分離はトーン（明度差）と間隔で行い、線で区切らない**。shadow は浮動要素のみ。accent は状態を表すためにのみ使う

observable な言い換え（vague 語は decision 根拠にしない）:

- "technical" → tabular numerals、強い左揃え/右揃えの規律、装飾 motion なし
- "calm" → 動く要素を 1 箇所（map / replay）に限定、状態遷移は即時または ≤150ms
- "dense" → HUD は tile grid、行高さ一定、padding は 8–12px 単位、page gutter との対比で密度を作る

### 禁止する紋切り型（AI slop）

以下の表現は採用しない。提案物でこれらが現れたら invalid として差し戻す。

1. **カードの一部にアクセント色を塗る** — accent は state の表示にのみ使い、面の装飾に使わない。左端・上端・角に色帯や色片を生やす、border に accent を差す、という「要素の一部をアクセントで彩る」パターンを禁止
2. **ナビ / タブに左の accent バーを生やす** — 現在地は **icon + 選択トーン（surface の明度差）** で示す
3. **同一パネル内でテキストサイズを乱高下させて階層を作る** — 特に match-state / HUD は**数値は同サイズ**で揃え、label と値の 2 段のみ。`DAY` だけ 28px、`REVISIONS` は別サイズ、のような作りを禁止
4. **1 ページ内に section を階層として立て直す** — タブ（route）ごとに、その内容を上から階層として並べ直すことはしない。情報は**同一レベルの帯（グリッド）**として置き、視線の移動順で優先度を決める
5. **border / shadow で面を分ける** — 明度差 ≥5 L\* と gap で分ける。stroke は focus ring のみ
6. **badge を tinted fill + 同色 border で作る** — 無彩色 chip + 細い tone-matched な marker に置き換える（badge 自体の再設計を参照）

## Layout / grid system（堅牢な構造）

**グリッドを全ページ・全セクションで適用する**。セクションごとに独自の配置を作らない。

- **グリッドは 1 系統**: 列は `repeat(12, minmax(0,1fr))` を基本とし、gap は spacing scale（4/8/12/16/24/32）に一致させる。12 が合わない surface は 4 か 6 列に折るが、**gap と外周ガターは全セクション共通**
- **全セクションはグリッドに載せる**。フリーレイアウトの絶対配置・決め打ち px 幅は使わない
- **component はグリッドから独立に流し込める**こと。セクション内部の component 配置は nested grid / flex で行い、グリッドの列境界を破らない
- **情報の優先度はグリッド上の位置と面積で決める**。優先度が高い情報ほど広い列 span と上部/左上の位置を取り、優先度の低い参照・操作は後退させる
- **UI variant の選択は情報の優先度で決める** — 同じ情報でも、primary なら value として大きく、secondary なら label 付きの小さな tile、tertiary なら text link に落とす。variant を情報の重要度から外して選ばない
- 外周ガター 24 / セクション間 24 / ブロック間 12 / 密集部 8。**外周と内部を別トークンにすること自体が密度戦略**（初版はセクション間を 20 と書いたが scale 外だったので 24 に揃えた）
- コンテンツ帯は `max-width` を持たせ、**左揃え・中央寄せ禁止**。広い画面で余白を伸ばさない
- shell は header 固定 + sidebar 独立スクロール + **main のみがスクロール所有者**。オーバーラップ禁止
- **shell はサブパネルで構成する** — 上から積み重ねる flow / content として扱わない。GitHub のような developer tool の shell では、面をパネルに分け、パネル内に階層を閉じる。route 全体を「見出し→本文→本文→表」の縦積みにしない
- **sidebar は「間の抜けた構成」にしない** — 項目が並んでいるだけの状態を禁止。現在地・優先度・役割を**色と構成の両方**で示す。ただし左に accent バーを生やすのは引き続き禁止（icon + 選択トーン）
- **panel / table の注釈は最低限に** — 囲みには許容するが、それに貼り付くプレーンテキストの注釈は本当に必要かを問う。必要な場合は値を数値として意図的に見せる

### 禁止する紋切り型（v3 追加）

7. **情報をフロー・コンテンツとして上から積み重ねる shell 構成** — 「テキスト・テキスト・テキスト・テーブル」と縦に並べる構成を禁止。情報と階層が見えない。**サブパネルに分割したセクション**で見せる
8. **囲み（table / panel）に、label のように貼り付くプレーンテキスト注釈** — 過度にテキストベースの注釈を禁止。本当に必要かをまず問う。必要なら**数値や値そのものを意図的に見せる**（単位・桁・ラベルを分離した value 表示、計測値の併記など）。プレーンテキストだけの説明は説得力が弱い
9. **間の抜けた構成** — サイドバーのように、要素が並んでいるだけで情報の主従が出ない構成を禁止。**色と構成の両方**で現在地・優先度・役割を示す

## Design tone — GitHub / VSCode（v4 で新設）

> **v4**: 「GitHub っぽいのに GitHub っぽくない」— border の見た目だけ借りて、**面の使い方**を借りていなかった。tone を観察可能な規則として固定する。

**GitHub の面の使い方（学ぶ）:**

- **box は「1 つの単位」になったときにだけ囲む。** box の中ではさらに box を重ねず、**行（row）と hairline** で分ける。これが「囲みすぎ・分けすぎ」を防ぐ唯一の規則
- box をまたぐ情報は 1 行に畳む。**2 行で書けるものを 2 行に分けない。** 行は 1 つの事実を 1 行で完結させる
- label は**意味を運ぶときだけ置く**。box の名札として常に付ける飾りにしない
- metadata は muted で主情報から後退させる。密度は高いが、**group 間には呼吸する隙間**を残す
- 区切りは full border ではなく **hairline（1px、低コントラスト）**を優先。full box は最終手段

**VSCode の shell の使い方（学ぶ）:**

- 画面は activity bar / side bar / editor / panel / status bar の**有限領域**に分かれ、各領域が自分の階層を閉じる
- editor 内側には装飾 border を置かない。**chrome と content の border 密度を明確に差別化**する
- icon が主軸、text は補助。tab は icon + label で、現在地は tone で示す

**コピーしない**: GitHub の brand 色・octicon の形・box shadow の使い方。VSCode の青系 accent・tab の角丸形状。

**判定**: ある要素に border を付けたいとき、次を順に問う。
(1) これは 1 つの単位か → そうでなければ囲まない
(2) その内側をまた分ける必要があるか → 多くの場合 **row + hairline** で足りる
(3) その label は意味を運ぶか → 運ばないなら消す

### 囲み・分割の優先見直し（v4）— 最優先

**囲みすぎ・分けすぎ**を許さない。分割は「情報設計として必要な分割か」「まとめたほうが情報が見やすいか」で決める。

- **1 行にできる情報は 1 行にする。** 2 行に分かれていて 1 行で完結するものを 1 行へ畳む
- **group は意味のまとまりで作る**。同じ種類の値は同じ group に、種類が違うものは別 group に。見た目の均等分割で割らない
- **box のラベルがテキスト装飾になっていないか**を最優先で見直す。box の名札として並ぶ micro-caps label で、読み手の判断に寄与しないものは**全部削除**する。残す label は「それが無いと意味が取れない」ものだけ

## References

各 reference は **surface / 学ぶ点 / コピーしない点** を分けて扱う。

- **GitHub（主要）**: この project の UI は developer tool であり、user も developer。**学ぶ**: icon を主軸に据えた控制（text ラベルなしでも意味が通る）、選択トーンによる現在地表示、控えめな focus、密度の高い表と toolbar。
  **コピーしない**: GitHub の brand 色・border 多用（本 project は線で分けない方針）・octicon の形自体。
- **GitHub Primer（現状の theme.css の出所）**: neutral grey ramp による面分割。**学ぶ**: 中立色で content を主役にする作法。
  **コピーしない**: accent `#0969da` そのままの brand 依存。
- **Linear の pane hierarchy**: compact typography と縮小された chrome。**学ぶ**: 面を「明度差 + 極小の padding 差」だけで分ける密度設計。**コピーしない**: dark palette と紫色 accent。
- **motorsport timing board**: 数値の tabular 揃え、right-aligned delta、label は小さく大文字、値が常に最大。**学ぶ**: 数値を reading の主役に据える typographic 規律。**コピーしない**: 赤/緑の派手な delta 色、影やグラデーション。

## Color direction

- **Default mode**: **実装に合わせる** — `stores/theme.ts` は localStorage → `prefers-color-scheme` の順で決まり、いずれも無い場合のみ `dark` に落ちる。light / dark を**対等に両立**させることを要求とし、「どちらが default」とは決めない
- **Temperature / saturation**: 中性 grey を surface と text に使い、彩度は state と data にのみ許可
- **Accent strategy**: 単一 hue + その scale。**active / focus / primary action 以外に使わない**。面の装飾・カードの一部・ナビの塗りに流用しない
- **Semantic constraints**:
  - `running` / `success` / `danger` は **意味専用**。装飾に流用しない
  - **status の明度を揃える** — v1 の計測で waiting `#f5c542`(L\*81.7) と error `#d1243f`(L\*45.9) に **ΔL\* 35.8** の差があり、黄色が発光して赤が沈んでいた。色相ではなく**明度の不揃い**が「色が離れている」原因。全 status を同一 L\* に揃え、差は hue と C\* で作る
  - status 色は **dark でも反転して維持**する（現状 `live.css` は dark で反転しない）
  - **data encoding は UI chrome と完全に分離**する。`AGENT_COLORS.supply = '#0969da'` が `--app-accent` とバイト一致しているのは違反。`.map-viewer__cell` の stroke に `--app-border-strong`（UI chrome トークン）を使うのも違反

### データ符号（hex map）の色と形 — **公式図を正とする**

> **v3 で反転した判断**: v2 は「赤・紫が刺激的だから彩度を下げろ」と案に指示し、C\* 24–30 の低彩度パレットになった。結果**識別が逆に難しくなり失敗**。正解は低彩度ではなく、**公式資料の図が使う色**だった。以下を正とし、v2 の低彩度方針を撤回する。

**正本は `docs/reference/hexaudon_spec.html` に埋め込まれた公式図**（図1 マップの構成例 / 図2 スポットの配置例と系列 / 図3 エージェントのタイプと役割 / 図4 エージェントの移動）。ピクセルから抽出した実測色は次の通り。

| 役割 | 実測色 | 出典 |
|---|---|---|
| terrain 平地 | `#8ed973`（`#8ed974` の揺れあり） | 図1 / 図2 / 図4 |
| terrain 山 | `#bfbfbf` | 図2 / 図4 |
| terrain 水 | `#46b1e1`（薄い方 `#9bc2e6`） | 図2 |
| terrain 道路 | `#156082` 系（濃紺） | 図3 / 図4 |
| spot 系列 | `#cc9900` / `#c04f15` ほか | 図2 |
| agent 巡回 | `#00b050` | 図3 |
| agent 補給 | `#e59edd` | 図3 |
| 移動の強調 | `#ffff00` | 図4 |

**設計原則:**

- **パッと見で分かる色**が要求。低彩度・同明度に寄せるのを禁止する。公式図は彩度もコントラストも高いのに「刺激的」ではない — ネオンではなく**落ち着いた実用彩度**（例: 純粋な赤ではなく `#c04f15` の burnt orange、純粋な紫ではなく `#e59edd` の pastel pink）だから
- **W3C / WCAG の標準規格**を満たす。少なくとも次を下限とする:
  - UI の text / icon とその背景のコントラスト比 **4.5:1 以上**（大きな text は 3:1）
  - 非 text の UI 部品・graphical object（marker、境界線、フォーカスリング）は **3:1 以上**
  - 色だけに依存しない冗長な手がかり（形状・記号）を必ず併記する — WCAG 1.4.1
- **識別性は「同時に並んだとき」で決める**。6 series が隣接した実サイズ（Ø15px 前後）で読めるかを必ず検証する。単独スウォッチの美しさは判断材料にしない
- v2 が課した「明度を完全に揃える」は **色の識別を壊すので撤回**。明度差は許容し、**低彩度に寄せるのだけを禁止**する
- 赤・紫の「刺激の強さ」は彩度そのものではなく**ネオン寄りの純色**にあった。公式図の同系色（`#c04f15` / `#e59edd`）を基準に据えれば、識別性を保ったまま刺激を下げられる

### ビジュアライザ符号（visualizer marks）— モチーフ起点

> **v3 で反転した判断**: v2 は「幾何形ベース / 塗り+内抜き / 線画」の 3 family を提示させたが、**抽象幾何の派生であって互いの差が小さく、意味がなかった**。方針を反転する。

- **モチーフを起点にする。** まず「それが何か」を絵で表し、**その絵を簡略化する**。シンプルさから始めない
  - 山 → 山の稜線を簡略化した形。水 → 波 / 水面。道路 → 線 / 帯。巡回車 → 巡回を表す形。補給車 → 補給を表す形
  - 抽象的な三角・菱形・円の並びを主軸にしない（現行 `MapViewer` の patrol=三角 / supply=菱形 は**意味を運んでいない**）
- **family としての差異を必ず出す。** 現状の 3 family は差がほとんど無く選択の意味が無い。**意味の違う符号同士は形も違う**ことを必須条件にする。選択肢を並べるときは、同一モチーフの簡略化度を変えるのではなく、**別モチーフの簡略化案**を並べる
- **漢字ラベルはオプション**（既定オフ）。符号が主
- 記号は icon 設計と同一規則（後述）に従い、余白・光学サイズ・線の太さを揃える
- **実サイズで検証**する。拡大図だけでは判断しない

### UI icon は Primer Octicons を採用（v4 追記）

**UI chrome の icon は Primer Octicons を inline SVG で使う**（code license: MIT、765 種、`<name>-16` / `<name>-24` の 2 グリッド、<https://github.com/primer/octicons>）。**これは UI の icon だけで、ビジュアライザ符号（hex map の記号）には使わない** — 符号は引き続きモチーフ起点の独自設計。

Octicons を選ぶ理由:

- **16 / 24 の 2 グリッド**が `--icon-size-sm: 16px` / `--icon-size-lg: 24px` と一致する
- GitHub 自身の UI で使われている形なので、**GitHub tone と自然に揃う**（形を借りるのではなく同じ資産を使う）
- MIT で改変可。path 数も少なく**軽量** — 「ネット不要・RAM 軽量」の要件を満たす

利用規約として:

- **inline SVG として埋め込む**。npm install や CDN 取得・icon font は使わない（会場がオフラインでも動くこと）
- viewBox は 16 / 24 のみ。`currentColor` 継承で色を持たせない
- **形の改変は最小限**。optical 微調整は許容するが、octicon として識別できる形を維持する
- 成果物に **MIT の attribution** を残す（既存の `THIRD_PARTY_NOTICES` 等の慣習に合わせる）

#### 採用マッピング（確定）

| 役割 | octicon | 備考 |
|---|---|---|
| Play | **`triangle-fill`** | 右向きの filled triangle で、そのままで Play。16/24 両方あり |
| Pause | `pause` | 2 本の縦バー |
| Step back / forward | `chevron-left` / `chevron-right` | 1 ステップ送り |
| Rewind to start / end | `move-to-start` / `move-to-end` | 矢印＋縦バーで skip-to-start / end |
| Rate | `stopwatch` | |
| theme トグル | `moon` / `sun` | |
| nav Home | `home` | |
| nav Replays | `history` | |
| nav Solvers | `beaker` | |
| nav Live | `broadcast` | |
| renderer SVG / WebGPU | `file-media` / `cpu` | |
| DAY / SERVER / LAST UPDATE | `versions` / `server` / `clock` | |
| status running | `dot-fill` | badge の tone-matched marker に流用可 |

**採用しなかった候補と理由:**

- `triangle-right` — 細いアウトラインのため 16px で Play として弱い。`triangle-fill` を採る
- **`skip` / `skip-fill`** — 実物は「円＋斜線」で dependency skip の意味。**メディア送りではないので使わない**（この brief で一時誤って採用していた）
- `actions` という名前の icon は octicons に存在しない。`workflow`（角丸 2 箱＋線、GitHub Actions のプロダクト sidebar 用）は transport ではなく識別用なので、必要なら nav 側で検討する

transport は上表の 5 種で完結させる。`play` も `triangle-fill` で解決済みのため、独自描画は発生しない。

`play` の欠落だけは実装時に判断が要る（`triangle-right` を使う / 1 つだけ独自に描く）。どちらでも**他は octicon に揃える**こと。

### サイズ・UI 色トークンは Primer Primitives を採用（v4 追記）

**トークンは自分で振らずに Primer Primitives から取る**（<https://github.com/primer/primitives>、MIT、`@primer/primitives`）。Octicons と同じ GitHub の資産なので **tone の食い違いが構造的に起きない**。微調整のコストを払うより、公開済みの検証済みトークンに乗る。

**理由:**

- 設計済みの scale に**用途規則が書かれている**（radius の small は「button/card に使うな」等）。判断が要らない
- **light / dark / dark.dimmed / high-contrast** の複数 mode が同梱。light/dark 対応も high-contrast も自前で作らない
- data の可視化用に **`data-vis` という系列専用パレット**があり、「複数 series は隣接色の対比を確保せよ」が明示されている。hex map の系列色の議論をそこで終わらせられる
- WCAG 適合は GitHub 側で保守される

#### 採用するトークン

**Spacing**（`functional/spacing/space.json5`）— 旧 4/8/12/16/24/32 を置き換え

| token | 値 | 用途（Primer 規則） |
|---|---|---|
| `space.xxs` | 2px | form separator・極小の間隔 |
| `space.xs` | 4px | badge・list item・小さい部品の内部 |
| `space.sm` | 8px | **既定**。flex/grid gap・card padding |
| `space.md` | 12px | 中間 |
| `space.lg` | 16px | section 間 |
| `space.xl` | 24px | 外周ガター |

**Radius**（`functional/size/radius.json5`）— 旧「control 4–6 / panel 6–8」を置き換え

| token | 値 | 用途（Primer 規則） |
|---|---|---|
| `borderRadius.small` | 3px | badge / tag / label / 小さい input。**button や card に使うな** |
| `borderRadius.medium` | 6px | **既定**。button / input / card / container |
| `borderRadius.large` | 12px | 大きい面 |

**Border / elevation**（`functional/size/border.json5`）

- `borderWidth.thin` 1px / `.thick` 2px（emphasis・focus）/ `.thicker` 4px
- **`boxShadow.thin = inset 0 0 0 1px`** — **border の代わりに shadow を使う**ことで layout shift を防ぐ。面の境界はこの inset shadow を優先する（従来「線で分けない」方針と両立する実装手段）

**Data / UI 色**（`base/color/{light,dark}/display-*.json5` + `functional/color/data-vis.json5`）

- display スケールは **20 hue × 10 段（0–9）**。data-vis は各 hue に **`emphasis` = level 5**（線・bar・border）と **`muted` = level 0**（面塗り・背景）を対で持つ
- data-vis の hue 族: `blue auburn orange yellow green teal purple pink plum red gray coral brown lemon olive lime pine` ほか
- **hex map の地形・系列・agent はこの `data-vis` から取る**。`emphasis` は marker・境界に、`muted` は地形塗りに。両 mode（light/dark）が用意済み
- **意味色（success / danger / attention）は data-vis を使わず `bgColor.*` / `fgColor.*` を使う**（Primer 規則: data 色を semantic に流用するな）

#### 微調整しない方針

上記に一致する値が Primer にある場合、**project 独自の値を作らない**。合わない場合は「何が合わないか」を記録した上で例外とし、値を気分で変えない。これは size・radius・border・data 色に適用し、**例外は `DESIGN-BRIEF` に残す**こと。

### GitHub 構造の具体規格（v4 追記 — 修正）

> **ここでの失敗**: 「囲みすぎ・分けすぎ」の指示が**効きすぎて**、逆に
> (a) サイドバーが 2 行要素と文字ラベルで膨らみ、(b) 右側の情報表示が囲み不足になった。
> GitHub は**どちらもしない**。見た目だけでなく**構造とトークン**で揃えること。

#### サイドバー（1 要素 = 1 行、厳守）

- **タブ（nav 項目）を 2 行にしない。** GitHub のサイドパネルで 2 行になる項目は基本無い。`icon + label` の**1 行のみ**
- **`nav-sub` のような 2 行目の補足（例: `1 / 4 · [2, 3]`）は禁止。** メタ情報はサイドバーに置かず、右側の情報パネルへ送る
- **役割の文字ラベル（`Orient` / `Review` / `Monitor` など）を消す。** これは先に消した nameplate と同じ「意味を見失ったテキスト装飾」。役割を示す必要があるなら**アイコン**で
- **状態はアイコンで示す**（現在地・running など）。テキストで書かない。`dot-fill` 等の marker は正しい
- 現在地の表現は GitHub どおり: **選択 tone（surface の明度差）+ 太字**。左 accent バーは引き続き禁止
- 項目は icon + label を左揃えで縦に並べるだけ。group 分割は 4 項目なら不要

#### 右側の情報表示（囲む — 囲み不足も間違い）

**GitHub は情報の単位を box で囲む。** 「囲みすぎを減らせ」の指示で box を無くしてはいけない。

- 右側の metadata / 情報レールは **1 つの情報単位 = 1 つの box** で囲む（GitHub の repo sidebar の About / Releases / Packages に相当）
- box の中には**その単位の見出しを 1 つ**置く。これは「意味を見失った nameplate」ではなく**単位の題名**なので消してはいけない。消すべきなのは「box の外や隣に付く、内容を言い換えるだけのラベル」
- box 内は行 + hairline で分ける（前回までの規則はそのまま）
- **UI と情報レールで border 密度を変える**のは正しい。ただし情報レールの box は**減らす対象ではない**

#### 判定の言い換え（重要）

「label を全部消せ」は誤り。正しくは次の 2 分類で分ける。

| 分類 | 例 | 扱い |
|---|---|---|
| **単位の題名** | box 内の見出し（About / Releases 相当） | **残す**。無いと単位が何なのか分からない |
| **言い換えの装飾** | box の外や隣に付く micro-caps、役割ラベル、値を繰り返すタグ | **消す**。読み手の判断に寄与しない |

#### トークンも構造も GitHub に合わせる

**見た目だけ借りてはいけない。** Primer Primitives のトークン（space / borderRadius / borderWidth / boxShadow.thin / fgColor / bgColor / borderColor）と、GitHub の**構造**（1 行 nav・情報単位の box・box 内の題名）の両方を揃える。どちらかだけ揃えても「GitHub っぽくない」まま。

#### 符号の計測規格（v4）

色は現行の方向で概ね良好。以下は**幾何**の調整。

1. **ドットの大きさを実サイズで調整する** — 背景 tone のドットテクスチャは、小さくするとドットとしての印象が薄れる。**実描画サイズでドットがまだドットとして読める粒径**にする。拡大時の見栄えで決めない
2. **斜め線は hex の辺に平行にする** — 現状の斜線は六角形のどの辺にも並行しておき気になる。pointy-top hex の辺方向（3 方向）に揃える
3. **hex の囲み border は維持** — 見やすくなったので変更しない
4. **重心を hex の中心に揃える** — オブジェクト同士の余白が不揃いでバランスが崩れている。各符号の重心（centroid）を hex 中心に一致させる
5. **占有割合を揃える** — 定量規格として、**符号の外接円を領域と定義し、その円内の ink 面積占有率（ink area / circle area）を各符号で揃える**。ばらつきが大きい符号は描画を足し引きして合わせる
6. **小さすぎる符号を直す** — 一部の符号は実サイズで意味が取れないほど小さい。最小可読サイズを下回らないよう描画を起こし直す

検証は**計測値を併記**して行う（各符号の centroid offset [px]、占有割合 [%]、最小 bbox [px]）。

## Typography direction

- **Role split**: UI text（sans） / **numeric & metadata（tabular figures）** / command・id（mono） の 3 role を明確に分ける
- **同一 surface 内のサイズは揃える**: match-state / HUD / 表の中で、`DAY` だけ大きく `REVISIONS` は小さく、のような乱高下をしない。**label 11–12px + 値 15–16px の 2 段**に統一し、優先度はサイズではなくグリッド上の位置と列 span で示す
- **コントラストの下限を守る** — UI text / icon は背景に対し **4.5:1** 以上（大 text は 3:1）、marker / 境界線 / フォーカスリングは **3:1** 以上。低コントラストで「上品」に見せるのは禁止。**パッと見で分かること**が最優先
- **色は合っていてもテキストの印象が微妙** — 色を動かす前に、**字組みで印象を作る**。letter-spacing の規律、label の case 統一（uppercase + tracking）、和欧混植の baseline / weight 差を整える（`typesetting` 参照）
- **Numeric / metadata requirements**: `DAY 1 / 4`、スコア `30`、`LAST UPDATE 17:06:06`、`[2, 3]` は **等幅・桁揃え**（`font-variant-numeric: tabular-nums`）
- **Mixed script**: 日本語 docs / 英語 UI string が混在する前提

## Iconography（主要な UI の主軸）

**UI の主要な認識は icon で行い、text ラベルは補助に留める**。icon の方が認知負荷が低い。**GitHub を主要 reference** にする。

- **SVG inline のみ**。icon font・外部配信・画像ファイルは使わない
- **ネットワーク不要・RAM 軽量**。最適サイズの SVG（viewBox は 16 / 24 の 2 系統に限定）を使い、パス数とノード数を抑える。装飾的 detail は削る
- **icon の余白をデザインシステムに組み込む** — optical sizing、四周の clear space、icon と text の間隔、ボタン内での icon の縦横センタリングを**トークン化して規定**する。icon 単体ではなく「icon + 余白」を一つの部品として扱う
- transport set（play / pause / step / rewind）と renderer set（SVG / WebGPU）を、**同じ線の太さ・同じ角の処理・同じ光学サイズ**で一つの family として作る
- color は `currentColor` 継承のみ。icon 自体に色を持たせない

## Focus / feedback

**focus ring は派手にしない**。アクセント色の強いリングは、この用途では不要な刺激になる。

- focus 表示は **button のトーンに近い色**から取る。accent の強い色を ring に使わない
- ただし focus の**可視性そのものは機能要件**である — 非フォーカス状態と区別がつかない focus はキーボード操作が成立しない。派手にしないことと、見えなくすることは別。明度差で判断できる範囲に収める
- 太さは 2px 前後、offset は小さく。アニメーションしない

## Component character

- **Radius**: 4–6px（control） / 6–8px（panel）を厳守。現状 `--app-radius: 8px` が 15 箇所一律に掛かっていて上限を踏んでいる — **control / panel / chip で値を分ける**
- **Elevation**: 1px border は使わない（前述のとおり面の分離はトーンと間隔）。shadow は**浮動要素のみ**。現状 `.page` / `.replay-viewer` / `.app-header` が静的パネルに shadow を持っているのは違反
- **Badge / chip**: tinted fill + 同色 border をやめる。**無彩色 chip + 細い tone-matched marker（dot 等）**にする。文字は小さく、色で騒がない
- **Control**: トランスポート・renderer 切替・scrub・表など**既存の control は概ね整っている**。見た目を作り直すのではなく、icon 化と focus の調整に留める

## Motion

- **Level**: functional minimal。装飾 motion は 0
- **Where motion is useful**: replay の step 進行と scrub、renderer 切替の fade、status 変化、focus/hover の応答（≤150ms）
- **Where motion should be absent**: ページ遷移、map の常時アニメーション、scroll effect
- `prefers-reduced-motion` では即時切替にする

## Decisions made

- **desktop-first**: venue での laptop 使用が主。responsive は「壊れない」水準（ただし media query が現状 0 件なので、その実装は必要）
- **map は独立した層**: chrome と canvas の視覚的分離を保ち、map 周りの間隔で「計器の中に地図が乗っている」関係を作る
- **既存 control の見た目は大きく変えない** — 問題は装飾ではなく icon / focus / spacing
- **グリッドを全セクションに強制する**こと。section ごとの独創的配置を禁止
- **性能**: icon は最適サイズの軽量 SVG、ネットワーク取得・画像ファイル・icon font を使わない

## Explicitly unresolved

- **符号（visualizer marks）の複数パターンの最終選定** — 3 系統以上を提示し、user に選ばせる
- **projector / 2m 距離での可読性要件**: 会場展示があるか不明。あれば scale と contrast の下限を上げる
- **brand mark / logo の有無**: 現状テキストロゴのみ。identity の再定義は scope 外

## v1 からの撤回（proposal で採用してはいけない判断）

v1 の brief から、または v1 を input に作った proposal 3 案から、以下は**採用しない**。

1. **match-state でテキストサイズをバラバラに表示する**（design-system 案が「DAY の値 28px が page title 22px より上」とした判断）— 撤回。同一 surface は同サイズ 2 段のみ
2. **カードの一部にアクセント色を塗る**装飾 — 撤回。AI slop として禁止
3. **ナビ / タブの左 accent バー**（layout 案の `inset 3px` の左罫 + `surface-alt` も同種）— 撤回。icon + 選択トーンに置き換える
4. **badge を tinted fill + 同色 border で作る** — 撤回。無彩色 chip + marker に
5. **focus ring を accent 色で派手に立てる**（ux 案の 2px `--app-focus` offset 2px）— 撤回。button トーンに寄せ、控えめに
6. **タブごとの section を階層として上から立てる情報設計** — 撤回。同一レベルの帯に置き、視線順で優先度を決める
7. **SPOT_SERIES_COLORS の現行 6 色**（特に violet / red / pink）— 撤回。刺激を抑え、明度を揃え直す
8. **漢字テキストを凡例の主軸にする** — 撤回。符号が主、漢字はオプション

## Evidence（現状把握）

- 現 UI の実測: `target/uishots2/*.png`（5 ルート、console error 0）
- 既知 visual bug: WebGPU canvas が headless で白紙 = **Issue #259**
- **実バグ**: `Solvers.tsx:77,80,89` が使う `.page__renderer-toggle` / `.page__renderer-btn` の CSS が `src/styles/` に存在しない → renderer 切替が素のブラウザボタンのまま
- **計測**: `--app-muted: #7d8590` on `--app-surface: #161b22` = **3.04:1**（AA 4.5:1 未満）
- 現状の弱点: `/replays` の map が左上に寄る、`/solvers` も同様。ページはほぼ未装飾で hierarchy が弱い
