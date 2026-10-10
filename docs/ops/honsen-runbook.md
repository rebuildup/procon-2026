# 本選当日 Runbook（競技部門「ヘキサうどん」）

操作担当者向け。Rust の知識は要りません。コマンドはすべてそのまま打てます。
検証の根拠は [`docs/evidence/honsen-385/README.md`](../evidence/honsen-385/README.md)。

Windows 11 で動かす場合は [`honsen-windows.md`](honsen-windows.md)、試合記録の形式は [`match-record.md`](match-record.md) を参照。`honsen start` は `~/.procon-honsen/records/` に試合記録、`~/.procon-honsen/logs/` に HTTP 通信ログ（[`http-log.md`](http-log.md)）を残す（対応したバイナリの場合。通信ログの全文は `coordinator.log` にも流れる）。

- 対象端末: MacBook Air (Apple M5, arm64, macOS)。ビルド済みバイナリは `~/.procon-honsen/` にあり、競技中のビルドは不要
- コマンド: `~/.procon-honsen/bin/honsen`（以下 `honsen` と書く）
- 最初に一度だけ: `alias honsen=~/.procon-honsen/bin/honsen`

画面の色で接続先を見分けます。**赤い帯 = 本番 (official)**、緑の帯 = 練習 (local)。

## 0. 会場で運営から受け取るもの

| もの | 使う場所 |
|---|---|
| 回答サーバーの URL（`http://<IP>:<ポート>`） | `honsen set-server official <URL>` |
| チームのトークン | `honsen set-token official` |

試合ごとに URL やトークンが変わると案内されたら、試合のたびに上の 2 つをやり直します。

## 1. 競技開始前

```bash
# 1) 電源アダプタと LAN ケーブルを接続。Wi-Fi は切る（競技 LAN 以外へ出ないようにする）
networksetup -setairportpower en0 off

# 2) バイナリと sha256 を確認（current が v2-71a9c0f、backup が v1-5ddb153 で、どちらも OK）
honsen releases

# 3) 接続先とトークンを登録（トークンは画面にも履歴にも残りません）
honsen set-server official http://<IP>:<ポート>
honsen set-token official

# 4) 検査。何も送信しません（GET /setting を 1 回読むだけ）
honsen preflight official
```

`PREFLIGHT OK` が出れば準備完了。`FAIL` の行には直し方が書いてあります。

| preflight の表示 | 意味と対処 |
|---|---|
| `サーバー応答: 200` | 認証 OK、試合設定が出ている |
| `サーバー応答: 403 AccessTimeError` | 認証 OK、まだ試合が設定されていない。そのまま待つ |
| `サーバー応答: 401 AuthError` | トークン違い。`honsen set-token official` をやり直す |
| `サーバーに接続できません` | LAN ケーブル、IP、ポートを確認。`ping <IP>` |
| `時計がサーバーと Ns ずれています` | システム設定 → 一般 → 日付と時刻で合わせる。会場 LAN は外に出られないので、**LAN につなぐ前に** Wi-Fi で自動設定しておく |
| `別の coordinator が動いています` | Arena 用などが残っている。表示された PID を `kill <PID>` |

## 2. 競技開始時

車種の回答時間は約 1 分、各日の回答時間も約 1 分です。**車種の回答受付が始まる前に起動しておきます**（前の試合が終わって自分たちの試合の設定が出たら、すぐ）。

```bash
honsen preflight official     # 直前にもう一度
honsen start official         # 確認に OFFICIAL と入力すると開始
honsen status -w              # 監視画面（1 秒ごとに更新、Ctrl-C で監視だけ終了）
```

起動後は自動で次の順に進みます。人が操作することはありません。

1. `GET /setting` で試合設定を取得
2. 車種を決めて `POST /agent`（補給 1 台＋残り巡回、または全車巡回を 1.5 秒で試算して選ぶ）
3. 試合開始を待つ
4. 毎日 `GET /` → 計算 → `POST /` → サーバーの受理番号を確認
5. 最終日の受理後に自動終了

### 正常な受理の見分け方

`honsen status -w` で次を見ます。

- `試合状態 : running`
- `revision : [1, 2, 3, …]` — 日が進むたびに 1 つ増える。**0 以上の数字 = サーバーが受理した**
- 最新ログに `plan accepted by server day=N revision=R … margin_ms=…`（`margin_ms` は締切までの残り時間）
- 既定候補（`budget_ms=59000:adapt=off`）は毎日ほぼ時間いっぱいまで探索する。受理の行が出るのは各日の開始から約 54 秒後で、`margin_ms` は 5200 前後になる。これが正常値で、日の前半にログが進まないのは故障ではない

「送った」だけでは受理ではありません。受理の根拠はサーバーが返した `revision` だけです。

## 3. 競技中

見るのは `honsen status -w` だけです。

| 表示 | 状況 | 対応 |
|---|---|---|
| `実行中` で `revision` が毎日増える | 正常 | 何もしない |
| `再起動回数: 1` 以上 | プロセスが落ちて自動で復帰した | 何もしない。次の日から受理が続くか見る |
| `POST / got no verdict; sending the same plan again` | 通信が切れて同じ回答を送り直している | LAN ケーブルを確認。つながれば自動で受理される |
| `GET / failed; asking again` | サーバーに届いていない | LAN ケーブルを確認。復旧した日から再開する |
| `未回答の日 N 日` | その日は回答できなかった | 止めない。次の日は自動で回答する |
| `plan rejected by server` | 回答が不正と判定された | 止めない。その日は失うが次の日は回答する。試合後にログを開発へ |
| `停止（結果不明・プロセスなし）` | 監視役ごと落ちた | すぐ `honsen start official`。試合の途中から再開する |

### 判断の原則

- **試合中は止めない・切り替えない。** v2 は日単位で自動復旧します。手で止めると、その後の日をすべて失います
- 迷ったら `honsen start official` をもう一度打つ。動いていれば「二重起動はできません」と断られるだけで害はありません。落ちていれば途中から再開します
- 端末が固まった・再起動した場合も `honsen start official` で途中の日から再開します（車種はサーバーが覚えている構成をそのまま使います）

### fallback（試合と試合の間にだけ行う）

```bash
honsen stop                # 動いていれば止める
honsen rollback            # backup (v1-5ddb153) に切り替え
honsen preflight official
```

| 症状 | 切り替え先 |
|---|---|
| v2 が起動直後に落ちる、復旧処理がおかしい | `honsen rollback`（v1-5ddb153。Arena で 5 試合の実績。ただし試合途中の再開はできない） |
| fleet の回答が何日も `rejected` になる | `honsen use v2-71a9c0f-anytime`（旧候補 anytime。玉数は落ちるが別系統の計画） |
| 元に戻す | `honsen use v2-71a9c0f` |

## 4. 競技終了後

```bash
honsen status      # 「試合完了（正常終了）」と、受理した日数 = 全日数 を確認
honsen stop        # まだ動いていれば止める
honsen collect     # ログを ~/procon-honsen-logs-<日時>.tgz に保存（トークンは含まれない）
```

次の試合がある場合は「2. 競技開始時」へ戻ります。

## 5. 練習（会場で本番前に 1 回やっておく）

自分の PC で公式の簡易サーバーを動かして、同じ手順を通します。本番サーバーには何も送りません。

```bash
# 別のターミナルで簡易サーバーを起動（設定 JSON は運営配布の example.json など）
~/.procon-honsen/official-server/procon-server-darwin-arm64 \
  -config <config.json> -addr 127.0.0.1:18080 -kind-deadline 30s -match-start-delay 5s

honsen set-server local http://127.0.0.1:18080
honsen set-token local          # config.json の teams[].token を入力
honsen preflight local
honsen start local
honsen status -w
```

## 6. コマンド一覧

| コマンド | 内容 |
|---|---|
| `honsen preflight <official\|local>` | 端末・バイナリ・設定・トークン・サーバー・時計を検査。何も送信しない |
| `honsen start <official\|local>` | 起動。official は `OFFICIAL` の入力が必要。二重起動は拒否 |
| `honsen status [-w]` | 接続先、バイナリ、受理状況、最新ログ |
| `honsen stop` | 停止。試合中は `STOP` の入力が必要 |
| `honsen rollback` | backup のバイナリへ切り替え（停止中のみ） |
| `honsen use <名前>` / `honsen releases` | バイナリの切り替え / 一覧と sha256 検査 |
| `honsen set-server` / `set-token` / `pull-token official` | 接続先・トークンの登録（`pull-token` は Infisical から取得。ネット接続が必要） |
| `honsen logs` / `honsen collect` | ログの追尾 / 保存 |

誤操作を防ぐ仕組み: official に Arena のホストや自分の PC を登録できない、local に外部ホストを登録できない、official と local のトークンが同じなら preflight が FAIL、Arena はこのコマンドでは起動できない。
