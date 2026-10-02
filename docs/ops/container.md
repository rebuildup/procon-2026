# コンテナ運用ガイド (Issue #51)

本選ブースでソルバを動かすためのコンテナ運用手順。
本リポの `procon` バイナリを 1 本の OCI イメージに閉じ込め、
持ち込み端末 / OS 差を吸収することを目的とする。

## 関連

- Issue: [#51 ops: docker sandbox (競技本番用コンテナ)](https://github.com/rebuildup/procon-2026/issues/51)
- 依存: [#45 coordinator](https://github.com/rebuildup/procon-2026/issues/45)
  （行動計画本体は #45 で実装する。本チケットは「コンテナに乗せて起動できる
  ところまで」をスコープとする。）
- 募集要項「通信方法」(有線 LAN / `http POST/GET`)
- `docs/contest/strategy.md` — 戦略ノート（実装と独立）

## イメージ構成

`Containerfile` は 4 段の multi-stage:

| stage | 役割 | ベース |
|---|---|---|
| `toolchain` | Rust + Bun + Node + Cargo registry pre-warm | `rust:1.85-bookworm` |
| `dev` | `devcontainer.json` から使う開発用 (source マウント) | `toolchain` |
| `coordinator-builder` | `procon` release バイナリをビルド + strip | `rust:1.85-slim-bookworm` |
| `coordinator` | 本番 runtime。非 root + tini + `/app/procon` のみ | `debian:bookworm-slim` |

devcontainer は `dev` 段を、競技本番は `coordinator` 段を使う。

## ビルド

```bash
# 本番用 coordinator イメージ (--target coordinator が既定の最終段)
docker build -t procon-2026-coordinator:local --target coordinator .

# dev 段 (devcontainer / 開発用)
docker build -t procon-2026-dev:local --target dev .
```

GitHub Container Registry 等に push する場合はタグ規約に従う:

```bash
docker build -t ghcr.io/rebuildup/procon-2026-coordinator:0.2.0 --target coordinator .
docker push ghcr.io/rebuildup/procon-2026-coordinator:0.2.0
```

## 起動

`procon coordinator` サブコマンドがコンテナ起動時のデフォルト CMD。
判定性のため `--dry-run` を既定にしている (設定の自己解決だけして終了)。
本番では `--dry-run` を外して常駐させる:

```bash
docker run --rm \
  --name procon-coordinator \
  --network host \
  -e PROCON_SERVER=match-server.local:7878 \
  -e PROCON_TOKEN=... \
  procon-2026-coordinator:local \
  coordinator
```

### `--network host` を使う理由

本選 LAN では **mDNS / リンクローカル解決** や、DHCP でもらう予測不能な
IP アドレス帯の試合サーバに対して TCP を張る必要がある。
bridge ネットワークだと host から渡ってきた `localhost` 表記が解決できない
ケースがあるため、`--network host` を既定とする。

### シグナル / グレースフル停止

`coordinator` ステージは `tini` を PID 1 として使うので、`docker stop` から
送られた `SIGTERM` が `procon` プロセスに正しく転送される。
本番の行動ループ (#45) は `SIGTERM` を受け取った時点で「現在の turn を
完了 → 次 turn を取りに行くループを抜ける」設計を想定している (#45 側)。

## 環境変数リファレンス

すべて `procon coordinator` 起動時に env から読まれる。
CLI フラグ (`--server`, `--agent-url`, ... ) で上書き可能だが、
本番運用では env 経由を推奨 (compose / k8s と相性が良い)。

| 変数 | 既定値 | 必須? | 用途 |
|---|---|---|---|
| `PROCON_SERVER` | `localhost:7878` | 任意 | 試合サーバの `host:port` もしくは URL。auto-detect の起点 |
| `PROCON_AGENT_URL` | `$PROCON_SERVER/agent` | 任意 | ソルバ行動の POST 先 |
| `PROCON_SETTING_URL` | `$PROCON_SERVER/setting` | 任意 | 試合設定 / day info の GET 先 |
| `PROCON_TOKEN` | (なし) | 任意 | `Authorization: Bearer <token>` ヘッダとして付与 |
| `PROCON_HTTP_TIMEOUT_MS` | `5000` | 任意 | 1 リクエストの read/write タイムアウト |
| `PROCON_HTTP_RETRIES` | `3` | 任意 | 失敗時の最大リトライ回数 (初回 + retries 回まで) |
| `PROCON_HTTP_BACKOFF_MS` | `250` | 任意 | リトライ初期待ち (指数バックオフで `×2` ずつ伸び、`×8` で cap) |
| `RUST_LOG` | `info` | 任意 | `tracing` のログレベル |

### auto-detect の解決順序

1. `PROCON_AGENT_URL` / `PROCON_SETTING_URL` が直接設定されていればそれを使う。
2. 無ければ `PROCON_SERVER` を parse して `<origin>/agent` `<origin>/setting` を組み立てる。
   - `host:port` → `http://host:port`
   - `host` のみ → `http://host:7878` を補完
   - `http://...` / `https://...` → そのまま尊重
3. `PROCON_SERVER` も無ければ `localhost:7878`。

### 例: 公式 LAN サーバに向けた最小設定

```bash
PROCON_SERVER=match.kosen-procon.jp:7878
PROCON_TOKEN=$(cat /run/secrets/procon-token)
PROCON_HTTP_TIMEOUT_MS=3000   # 1 日の制限時間 60s に収めるため少し短く
```

### 例: ローカル (devcontainer 内の engine サーバ) 向け

```bash
PROCON_SERVER=engine:7878       # compose service name
PROCON_TOKEN=                   # ローカルでは空でも OK
```

## リトライ / タイムアウトの挙動

| 失敗種別 | リトライ | 備考 |
|---|---|---|
| 接続失敗 / timeout | yes | バックオフ = `backoff * 2^(attempt-1)` 最大 8 倍 |
| `5xx` | yes | バックオフは同じ |
| `429 Too Many Requests` | yes | バックオフは同じ |
| `4xx` (429 以外) | no | 即座にエラーを返す (バグ or 認証失敗) |

タイムアウトは read / write 両方に `PROCON_HTTP_TIMEOUT_MS` を適用する。
curl 的には `curl --connect-timeout N --max-time N` 相当。

## ローカル開発 (`docker compose`)

リポジトリ直下の `docker-compose.yml` を使うと、
試合サーバ (engine) と coordinator を 1 つの bridge network で起動できる。
これは **engine サーバ実装 (#45 周辺) が手元にない開発フェーズ** 用の暫定で、
将来のチケットで本物の engine image と差し替える。

```bash
docker compose up --build coordinator
```

## トラブルシューティング

| 症状 | まず疑う点 |
|---|---|
| 起動直後に `Connection refused` | `PROCON_SERVER` の host / port が違う。`docker exec ... ping <host>` で名前解決確認 |
| `HTTP error after N attempts` で即終了 | 試合サーバの電源 / LAN ケーブル。`RUST_LOG=debug` でどの status code で失敗しているか詳細が出る |
| `PROCON_HTTP_RETRIES=0` でもリトライが走る | 起動時の env が古い。`docker inspect <container>` で `Config.Env` を確認 |
| イメージサイズが大きい | `--target coordinator` を付け忘れて `dev` 段を pull している可能性 |

## CI との統合

`Containerfile.coordinator` の最終段が壊れていないかは、CI で `docker build` の
みを dry-run するジョブが望ましいが、本チケットでは未実装 (#45 完了後に追加)。