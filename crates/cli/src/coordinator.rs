// Deprecated path retained for #D; will be removed.
#![allow(dead_code)]
//! Contest-runtime coordinator.
//!
//! 本番ブースで動かすソルバの LAN 側エントリポイント。
//! 試合サーバ (match server) を自動検出し、`/agent` への POST を
//! リトライ・タイムアウト付きで発行する。
//!
//! ## 役割分担
//!
//! - 試合進行のルール本体 (`engine` crate) は別 crate。
//! - ここで扱うのは「LAN 上でサーバと話す I/O 層」と「env からの起動設定」だけ。
//! - 行動計画の生成は別 ticket (#45 coordinator) で実装する想定。
//!
//! ## 環境変数
//!
//! | 変数 | 既定値 | 用途 |
//! |---|---|---|
//! | `PROCON_SERVER` | `localhost:7878` | 試合サーバの host:port (auto-detect の起点) |
//! | `PROCON_AGENT_URL` | `$PROCON_SERVER/agent` | ソルバ行動の POST 先 |
//! | `PROCON_SETTING_URL` | `$PROCON_SERVER/setting` | 試合設定 / day info の GET 先 |
//! | `PROCON_TOKEN` | (なし) | 試合サーバへの bearer token (任意) |
//! | `PROCON_HTTP_TIMEOUT_MS` | `5000` | 1 リクエストのタイムアウト |
//! | `PROCON_HTTP_RETRIES` | `3` | 失敗時の最大リトライ回数 |
//! | `PROCON_HTTP_BACKOFF_MS` | `250` | リトライ初期待ち時間 (指数バックオフ) |
//!
//! ## auto-detect 仕様
//!
//! 1. `PROCON_AGENT_URL` / `PROCON_SETTING_URL` が直接設定されていればそれを使う。
//! 2. 無ければ `PROCON_SERVER` (host:port) を取り出し、scheme を `http://` とみなして
//!    `<host>/agent` / `<host>/setting` を組み立てる。
//! 3. `PROCON_SERVER` も無ければ `localhost:7878` を既定として扱う。
//! 4. URL に scheme が含まれていればそのまま尊重する (`http://` / `https://` どちらも可)。

use std::env;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{debug, info, warn};

/// Coordinator の起動設定。env と CLI フラグから組み立てる。
#[derive(Debug, Clone)]
pub struct CoordinatorConfig {
    /// 試合サーバの URL (origin 部分。scheme + host + port)。
    pub server_origin: String,
    /// `/agent` POST 先。
    pub agent_url: String,
    /// `/setting` GET 先。
    pub setting_url: String,
    /// 任意の bearer token。
    pub token: Option<String>,
    /// 1 リクエストのタイムアウト。
    pub timeout: Duration,
    /// 最大リトライ回数 (初回失敗後の追加試行数)。
    pub max_retries: u32,
    /// リトライ初期待ち (指数バックオフの base)。
    pub backoff: Duration,
}

impl CoordinatorConfig {
    /// env + 任意の CLI override から組み立てる。
    ///
    /// `cli_overrides` はテストや本選リハーサル時に CLI から値を差し込むために使う。
    pub fn from_env(cli_overrides: &CliOverrides) -> Result<Self> {
        let server_raw = cli_overrides
            .server
            .clone()
            .or_else(|| env::var("PROCON_SERVER").ok())
            .unwrap_or_else(|| "localhost:7878".to_owned());

        let server_origin = normalize_origin(&server_raw)?;

        let agent_url = cli_overrides
            .agent_url
            .clone()
            .or_else(|| env::var("PROCON_AGENT_URL").ok())
            .unwrap_or_else(|| format!("{server_origin}/agent"));

        let setting_url = cli_overrides
            .setting_url
            .clone()
            .or_else(|| env::var("PROCON_SETTING_URL").ok())
            .unwrap_or_else(|| format!("{server_origin}/setting"));

        let token = cli_overrides
            .token
            .clone()
            .or_else(|| env::var("PROCON_TOKEN").ok())
            .filter(|s| !s.is_empty());

        let timeout_ms = parse_env_u64("PROCON_HTTP_TIMEOUT_MS", 5_000)?;
        let retries = u32::try_from(parse_env_u64("PROCON_HTTP_RETRIES", 3)?)
            .context("PROCON_HTTP_RETRIES does not fit in u32")?;
        let backoff_ms = parse_env_u64("PROCON_HTTP_BACKOFF_MS", 250)?;

        Ok(Self {
            server_origin,
            agent_url,
            setting_url,
            token,
            timeout: Duration::from_millis(timeout_ms),
            max_retries: retries,
            backoff: Duration::from_millis(backoff_ms),
        })
    }

    /// 起動時の自己解決結果を確認用に dumpsys する。
    #[must_use]
    pub fn describe(&self) -> serde_json::Value {
        json!({
            "server_origin": self.server_origin,
            "agent_url": self.agent_url,
            "setting_url": self.setting_url,
            "has_token": self.token.is_some(),
            "timeout_ms": self.timeout.as_millis(),
            "max_retries": self.max_retries,
            "backoff_ms": self.backoff.as_millis(),
        })
    }
}

/// CLI から渡される任意の override。`None` なら env にフォールスルーする。
#[derive(Debug, Default, Clone)]
pub struct CliOverrides {
    pub server: Option<String>,
    pub agent_url: Option<String>,
    pub setting_url: Option<String>,
    pub token: Option<String>,
}

/// `PROCON_SERVER` の値を origin (scheme://host:port) 形式に正規化する。
///
/// - `localhost:7878` → `http://localhost:7878`
/// - `http://10.0.0.5` → `http://10.0.0.5`
/// - `https://host` → `https://host`
fn normalize_origin(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("PROCON_SERVER is empty"));
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Ok(trimmed.trim_end_matches('/').to_owned())
    } else if let Some((host, _port)) = trimmed.split_once(':') {
        // host:port → 既定で http
        if host.is_empty() {
            return Err(anyhow!("PROCON_SERVER host is empty: {trimmed}"));
        }
        Ok(format!("http://{trimmed}"))
    } else {
        // host のみ (port 省略) → 既定 port 7878 を補完
        Ok(format!("http://{trimmed}:7878"))
    }
}

fn parse_env_u64(name: &str, default: u64) -> Result<u64> {
    match env::var(name) {
        Ok(s) => s.parse::<u64>().with_context(|| format!("invalid {name}={s}")),
        Err(_) => Ok(default),
    }
}

/// `/agent` に POST する body。`#45` で詳細が決まるまでは最小形。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRequest {
    /// エージェント ID (本選で配布される ID を入れる)。
    pub agent_id: String,
    /// day index (0-based)。
    pub day: u32,
    /// turn index (0-based)。
    pub turn: u32,
    /// payload は server-driven に拡張する想定 (今は空で OK)。
    #[serde(default)]
    pub payload: serde_json::Value,
}

/// `/agent` からの最小レスポンス。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    /// 受理された turn id。
    pub turn_id: String,
    #[serde(default)]
    pub info: serde_json::Value,
}

/// HTTP error + retry budget。
#[derive(Debug, thiserror::Error)]
pub enum CoordinatorError {
    #[error("HTTP error after {attempts} attempts: {source}")]
    Http {
        attempts: u32,
        #[source]
        source: anyhow::Error,
    },
}

/// POST /agent with retry + per-request timeout.
///
/// リトライ方針:
/// - 接続失敗 / timeout / 5xx / 429 を retryable とみなす。
/// - 4xx (上記以外) は retry せず即座に失敗を返す。
/// - バックオフは `backoff * 2^attempt`、最大 `backoff * 8` で cap。
pub fn post_agent_with_retry(
    cfg: &CoordinatorConfig,
    body: &AgentRequest,
) -> Result<AgentResponse> {
    let agent =
        ureq::AgentBuilder::new().timeout_read(cfg.timeout).timeout_write(cfg.timeout).build();

    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0..=cfg.max_retries {
        if attempt > 0 {
            let delay = backoff_delay(cfg.backoff, attempt);
            warn!(attempt, delay_ms = delay.as_millis(), "retrying /agent POST");
            thread::sleep(delay);
        }
        debug!(attempt, url = %cfg.agent_url, "POST /agent");

        let mut req = agent.post(&cfg.agent_url);
        if let Some(token) = &cfg.token {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }

        match req.send_json(body) {
            Ok(resp) => {
                let status = resp.status();
                if (200..300).contains(&status) {
                    let parsed: AgentResponse =
                        resp.into_json().context("decode /agent response")?;
                    info!(%status, "POST /agent ok");
                    return Ok(parsed);
                }
                if (500..600).contains(&status) || status == 429 {
                    last_err = Some(anyhow!("server returned status {status}"));
                    continue;
                }
                // 4xx (429 以外) は即失敗
                return Err(anyhow!("POST /agent failed with status {status}"));
            }
            Err(ureq::Error::Status(status, response)) => {
                if (500..600).contains(&status) || status == 429 {
                    last_err = Some(anyhow!("server returned status {status}"));
                    continue;
                }
                let body = response.into_string().unwrap_or_else(|_| "<no body>".to_owned());
                return Err(anyhow!("POST /agent failed with status {status}: {body}"));
            }
            Err(ureq::Error::Transport(t)) => {
                last_err = Some(anyhow!(t).context("transport error"));
                // transport は必ず retry → 次の attempt へ
            }
        }
    }

    Err(CoordinatorError::Http {
        attempts: cfg.max_retries + 1,
        source: last_err.unwrap_or_else(|| anyhow!("unknown failure")),
    }
    .into())
}

fn backoff_delay(base: Duration, attempt: u32) -> Duration {
    let exp = attempt.saturating_sub(1).min(3); // cap at 8x
    let factor = 1u32 << exp;
    base.saturating_mul(factor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_host_port_adds_http() {
        assert_eq!(normalize_origin("10.0.0.5:7878").unwrap(), "http://10.0.0.5:7878");
    }

    #[test]
    fn normalize_host_only_adds_default_port() {
        assert_eq!(normalize_origin("match-server").unwrap(), "http://match-server:7878");
    }

    #[test]
    fn normalize_preserves_explicit_scheme() {
        assert_eq!(
            normalize_origin("https://match.example.com").unwrap(),
            "https://match.example.com"
        );
        assert_eq!(normalize_origin("http://10.0.0.5:7878/").unwrap(), "http://10.0.0.5:7878");
    }

    #[test]
    fn normalize_rejects_empty() {
        assert!(normalize_origin("").is_err());
        assert!(normalize_origin("   ").is_err());
    }

    #[test]
    fn normalize_rejects_empty_host_with_port() {
        assert!(normalize_origin(":7878").is_err());
    }

    #[test]
    fn backoff_grows_exponentially() {
        let base = Duration::from_millis(250);
        assert_eq!(backoff_delay(base, 1), Duration::from_millis(250));
        assert_eq!(backoff_delay(base, 2), Duration::from_millis(500));
        assert_eq!(backoff_delay(base, 3), Duration::from_millis(1000));
        assert_eq!(backoff_delay(base, 4), Duration::from_millis(2000));
    }

    #[test]
    fn backoff_caps_at_8x() {
        let base = Duration::from_millis(250);
        // attempt=10 → cap at 2^3 = 8x
        assert_eq!(backoff_delay(base, 10), Duration::from_millis(2000));
    }
}
