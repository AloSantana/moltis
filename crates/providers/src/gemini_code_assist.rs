//! Gemini Code Assist provider.
//!
//! Authentication reads OAuth credentials written by the Gemini CLI
//! (`~/.gemini/oauth_credentials.json`). The Gemini OpenAI-compatible
//! endpoint (`https://generativelanguage.googleapis.com/v1beta/openai`)
//! accepts the same Bearer token, so no new HTTP client is required.
//!
//! Token refresh is performed against Google's standard OAuth 2.0 token
//! endpoint when the stored access token is within five minutes of expiry.

use std::pin::Pin;

use {
    async_trait::async_trait,
    futures::StreamExt,
    moltis_oauth::{OAuthTokens, TokenStore},
    secrecy::{ExposeSecret, Secret},
    tokio_stream::Stream,
    tracing::{debug, trace, warn},
};

use {
    super::openai_compat::{
        SseLineResult, StreamingToolState, finalize_stream, parse_openai_compat_usage_from_payload,
        parse_tool_calls, process_openai_sse_line, to_openai_tools,
    },
    moltis_agents::model::{ChatMessage, CompletionResponse, LlmProvider, StreamEvent},
};

// ── Constants ────────────────────────────────────────────────────────────────

/// OpenAI-compatible base URL for the Gemini REST API.
const GEMINI_OPENAI_BASE: &str =
    "https://generativelanguage.googleapis.com/v1beta/openai";

/// Google OAuth token refresh endpoint.
const GOOGLE_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

/// Moltis token-store key for Gemini Code Assist.
pub const PROVIDER_NAME: &str = "gemini-code-assist";

/// Refresh threshold: 5 minutes before expiry.
const REFRESH_THRESHOLD_SECS: u64 = 300;

// ── Default model catalog ────────────────────────────────────────────────────

/// Models accessible via the Gemini API with OAuth credentials.
/// <https://ai.google.dev/gemini-api/docs/models>
pub const GEMINI_CODE_ASSIST_MODELS: &[(&str, &str)] = &[
    ("gemini-2.5-pro-preview-05-06", "Gemini 2.5 Pro Preview (Code Assist)"),
    ("gemini-2.5-flash-preview-05-20", "Gemini 2.5 Flash Preview (Code Assist)"),
    ("gemini-2.0-flash", "Gemini 2.0 Flash (Code Assist)"),
    ("gemini-2.0-flash-lite", "Gemini 2.0 Flash Lite (Code Assist)"),
];

// ── Credentials file format (Gemini CLI) ─────────────────────────────────────

/// Structure of `~/.gemini/oauth_credentials.json` written by the Gemini CLI.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCliCredentials {
    /// Short-lived access token (Bearer).
    token: String,
    /// Long-lived refresh token.
    refresh_token: Option<String>,
    /// RFC 3339 expiry timestamp, e.g. `"2025-03-11T19:04:00.000Z"`.
    expiry: Option<String>,
    /// OAuth 2.0 client ID stored by the Gemini CLI.
    client_id: Option<String>,
    /// OAuth 2.0 client secret stored by the Gemini CLI.
    client_secret: Option<String>,
}

// ── Provider ─────────────────────────────────────────────────────────────────

pub struct GeminiCodeAssistProvider {
    model: String,
    base_url: String,
    client: &'static reqwest::Client,
    token_store: TokenStore,
}

impl GeminiCodeAssistProvider {
    pub fn new(model: String) -> Self {
        Self {
            model,
            base_url: GEMINI_OPENAI_BASE.to_string(),
            client: crate::shared_http_client(),
            token_store: TokenStore::new(),
        }
    }

    /// Load OAuth tokens from the Moltis token store, falling back to the
    /// Gemini CLI credentials file.
    ///
    /// When the token store holds a token that was originally imported from the
    /// CLI file (i.e. `expires_at` is `None`), we re-read the CLI file so that
    /// any freshly-refreshed token written by the Gemini CLI is picked up and
    /// expiry is honoured for proactive refresh.
    fn load_tokens(&self) -> Option<OAuthTokens> {
        match self.token_store.load(PROVIDER_NAME) {
            // Prefer the CLI file when the stored token has no expiry: it was
            // probably imported without expiry info and the file may be fresher.
            Some(stored) if stored.expires_at.is_some() => Some(stored),
            _ => load_gemini_cli_tokens().or_else(|| self.token_store.load(PROVIDER_NAME)),
        }
    }

    /// Return a valid access token, refreshing if the stored token is near
    /// expiry or the API responds with `401 Unauthorized`.
    async fn get_access_token(&self) -> anyhow::Result<String> {
        let tokens = self.load_tokens().ok_or_else(|| {
            anyhow::anyhow!(
                "not authenticated for gemini-code-assist — \
                 install the Gemini CLI (`npm i -g @google/gemini-cli`) and run \
                 `gemini auth login`, or run `moltis auth login --provider gemini-code-assist`"
            )
        })?;

        // Proactively refresh when within the expiry window.
        if let Some(expires_at) = tokens.expires_at {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if now + REFRESH_THRESHOLD_SECS >= expires_at {
                if let Some(ref refresh_token) = tokens.refresh_token {
                    debug!("proactively refreshing gemini-code-assist token");
                    return self.refresh_and_store(refresh_token.expose_secret()).await;
                }
                return Err(anyhow::anyhow!(
                    "gemini-code-assist token expired and no refresh token available"
                ));
            }
        }

        Ok(tokens.access_token.expose_secret().clone())
    }

    async fn refresh_and_store(&self, refresh_token: &str) -> anyhow::Result<String> {
        // Try to find the client credentials from the CLI file so we can call
        // the Google token endpoint without hardcoding secrets.
        let (client_id, client_secret) = load_gemini_cli_client_credentials()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "gemini-code-assist: cannot refresh token — \
                     client credentials not found in ~/.gemini/oauth_credentials.json"
                )
            })?;

        let new_tokens = refresh_google_access_token(
            self.client,
            &client_id,
            &client_secret,
            refresh_token,
        )
        .await?;

        let access = new_tokens.access_token.expose_secret().clone();
        // Best-effort persist; failure is non-fatal.
        if let Err(e) = self.token_store.save(PROVIDER_NAME, &new_tokens) {
            debug!(error = %e, "gemini-code-assist: failed to persist refreshed tokens");
        }
        Ok(access)
    }
}

// ── Token helpers ─────────────────────────────────────────────────────────────

/// Return the path to the Gemini CLI credentials file, or `None` if `HOME` is
/// not set.
pub fn gemini_cli_credentials_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(std::path::PathBuf::from(home).join(".gemini").join("oauth_credentials.json"))
}

/// Return `true` when the Gemini CLI credentials file exists and contains a
/// non-empty access token.
pub fn gemini_cli_has_access_token(path: &std::path::Path) -> bool {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(creds) = serde_json::from_str::<GeminiCliCredentials>(&raw) else {
        return false;
    };
    !creds.token.trim().is_empty()
}

/// Try to load OAuth tokens from the Gemini CLI credentials file.
pub fn load_gemini_cli_tokens() -> Option<OAuthTokens> {
    let path = gemini_cli_credentials_path()?;
    let raw = std::fs::read_to_string(&path).ok()?;
    parse_gemini_cli_tokens(&raw)
}

/// Parse Gemini CLI `oauth_credentials.json` content into [`OAuthTokens`].
pub fn parse_gemini_cli_tokens(data: &str) -> Option<OAuthTokens> {
    let creds: GeminiCliCredentials = serde_json::from_str(data).ok()?;
    if creds.token.trim().is_empty() {
        return None;
    }

    let expires_at = creds.expiry.as_deref().and_then(parse_rfc3339_to_unix_secs);

    Some(OAuthTokens {
        access_token: Secret::new(creds.token),
        refresh_token: creds.refresh_token.map(Secret::new),
        id_token: None,
        account_id: None,
        expires_at,
    })
}

/// Return the `(client_id, client_secret)` pair from the Gemini CLI credentials
/// file. These are needed to call the Google token refresh endpoint.
fn load_gemini_cli_client_credentials() -> Option<(String, String)> {
    let path = gemini_cli_credentials_path()?;
    let raw = std::fs::read_to_string(&path).ok()?;
    let creds: GeminiCliCredentials = serde_json::from_str(&raw).ok()?;
    let client_id = creds.client_id.filter(|s| !s.trim().is_empty())?;
    let client_secret = creds.client_secret.filter(|s| !s.trim().is_empty())?;
    Some((client_id, client_secret))
}

/// Whether Gemini Code Assist has stored tokens in either the token store or
/// the Gemini CLI credentials file.
pub fn has_stored_tokens() -> bool {
    TokenStore::new().load(PROVIDER_NAME).is_some()
        || gemini_cli_credentials_path()
            .as_deref()
            .is_some_and(gemini_cli_has_access_token)
}

// ── Google OAuth token refresh ───────────────────────────────────────────────

/// Call the Google OAuth 2.0 token endpoint to exchange a refresh token for a
/// new access token.
pub async fn refresh_google_access_token(
    client: &reqwest::Client,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> anyhow::Result<OAuthTokens> {
    let resp = client
        .post(GOOGLE_TOKEN_ENDPOINT)
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("gemini-code-assist token refresh failed: {body}");
    }

    #[derive(serde::Deserialize)]
    struct RefreshResponse {
        access_token: String,
        expires_in: Option<u64>,
    }

    let body: RefreshResponse = resp.json().await?;
    let expires_at = body.expires_in.map(|secs| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            + secs
    });

    Ok(OAuthTokens {
        access_token: Secret::new(body.access_token),
        // Refresh tokens are long-lived; preserve the one we used so next
        // refresh also works.
        refresh_token: Some(Secret::new(refresh_token.to_string())),
        id_token: None,
        account_id: None,
        expires_at,
    })
}

// ── RFC 3339 → Unix timestamp ─────────────────────────────────────────────────

/// Parse an RFC 3339 datetime string (e.g. `"2025-03-11T19:04:00.000Z"`) into
/// a Unix timestamp (seconds since epoch).  Returns `None` on failure.
fn parse_rfc3339_to_unix_secs(s: &str) -> Option<u64> {
    use time::format_description::well_known::Rfc3339;
    let dt = time::OffsetDateTime::parse(s, &Rfc3339).ok()?;
    u64::try_from(dt.unix_timestamp()).ok()
}

// ── LlmProvider impl ──────────────────────────────────────────────────────────

#[async_trait]
impl LlmProvider for GeminiCodeAssistProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn id(&self) -> &str {
        &self.model
    }

    fn supports_tools(&self) -> bool {
        true
    }

    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        let token = self.get_access_token().await?;

        let openai_messages: Vec<serde_json::Value> =
            messages.iter().map(ChatMessage::to_openai_value).collect();
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
        });
        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(to_openai_tools(tools));
        }

        debug!(
            model = %self.model,
            messages_count = messages.len(),
            tools_count = tools.len(),
            "gemini-code-assist complete request"
        );
        trace!(
            body = %serde_json::to_string(&body).unwrap_or_default(),
            "gemini-code-assist request body"
        );

        let http_resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = http_resp.status();
        if !status.is_success() {
            let retry_after_ms = super::retry_after_ms_from_headers(http_resp.headers());
            let body_text = http_resp.text().await.unwrap_or_default();
            warn!(
                status = %status,
                body = %body_text,
                "gemini-code-assist API error"
            );
            anyhow::bail!(
                "{}",
                super::with_retry_after_marker(
                    format!("Gemini Code Assist API error HTTP {status}: {body_text}"),
                    retry_after_ms,
                )
            );
        }

        let resp = http_resp.json::<serde_json::Value>().await?;
        trace!(response = %resp, "gemini-code-assist raw response");

        let message = &resp["choices"][0]["message"];
        let text = message["content"].as_str().map(str::to_string);
        let tool_calls = parse_tool_calls(message);
        let usage = parse_openai_compat_usage_from_payload(&resp).unwrap_or_default();

        Ok(CompletionResponse {
            text,
            tool_calls,
            usage,
        })
    }

    fn stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        self.stream_with_tools(messages, vec![])
    }

    fn stream_with_tools(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(async_stream::stream! {
            let token = match self.get_access_token().await {
                Ok(t) => t,
                Err(e) => {
                    yield StreamEvent::Error(e.to_string());
                    return;
                }
            };

            let openai_messages: Vec<serde_json::Value> =
                messages.iter().map(ChatMessage::to_openai_value).collect();
            let mut body = serde_json::json!({
                "model": self.model,
                "messages": openai_messages,
                "stream": true,
                "stream_options": { "include_usage": true },
            });
            if !tools.is_empty() {
                body["tools"] = serde_json::Value::Array(to_openai_tools(&tools));
            }

            debug!(
                model = %self.model,
                messages_count = openai_messages.len(),
                tools_count = tools.len(),
                "gemini-code-assist stream_with_tools request"
            );
            trace!(
                body = %serde_json::to_string(&body).unwrap_or_default(),
                "gemini-code-assist stream request body"
            );

            let resp = match self
                .client
                .post(format!("{}/chat/completions", self.base_url))
                .header("Authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => {
                    if let Err(e) = r.error_for_status_ref() {
                        let status = e.status().map(|s| s.as_u16()).unwrap_or(0);
                        let retry_after_ms = super::retry_after_ms_from_headers(r.headers());
                        let body_text = r.text().await.unwrap_or_default();
                        yield StreamEvent::Error(super::with_retry_after_marker(
                            format!("HTTP {status}: {body_text}"),
                            retry_after_ms,
                        ));
                        return;
                    }
                    r
                }
                Err(e) => {
                    yield StreamEvent::Error(e.to_string());
                    return;
                }
            };

            let mut byte_stream = resp.bytes_stream();
            let mut buf = String::new();
            let mut state = StreamingToolState::default();

            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        yield StreamEvent::Error(e.to_string());
                        return;
                    }
                };
                buf.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = buf.find('\n') {
                    let line = buf[..pos].trim().to_string();
                    buf = buf[pos + 1..].to_string();

                    if line.is_empty() {
                        continue;
                    }

                    let Some(data) = line
                        .strip_prefix("data: ")
                        .or_else(|| line.strip_prefix("data:"))
                    else {
                        continue;
                    };

                    match process_openai_sse_line(data, &mut state) {
                        SseLineResult::Done => {
                            for event in finalize_stream(&mut state) {
                                yield event;
                            }
                            return;
                        }
                        SseLineResult::Events(events) => {
                            for event in events {
                                yield event;
                            }
                        }
                        SseLineResult::Skip => {}
                    }
                }
            }

            for event in finalize_stream(&mut state) {
                yield event;
            }
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_gemini_cli_tokens_valid() {
        let json = r#"{
            "token": "ya29.test",
            "refreshToken": "1//refresh",
            "expiry": "2025-03-11T19:04:00Z",
            "clientId": "cid.apps.googleusercontent.com",
            "clientSecret": "csec"
        }"#;
        let tokens = parse_gemini_cli_tokens(json).expect("should parse");
        assert_eq!(tokens.access_token.expose_secret(), "ya29.test");
        assert!(tokens.refresh_token.is_some());
        assert!(tokens.expires_at.is_some());
    }

    #[test]
    fn parse_gemini_cli_tokens_missing_token_returns_none() {
        let json = r#"{ "token": "", "refreshToken": "1//r" }"#;
        assert!(parse_gemini_cli_tokens(json).is_none());
    }

    #[test]
    fn parse_gemini_cli_tokens_no_expiry_ok() {
        let json = r#"{ "token": "ya29.x" }"#;
        let tokens = parse_gemini_cli_tokens(json).expect("should parse");
        assert!(tokens.expires_at.is_none());
    }

    #[test]
    fn parse_rfc3339_to_unix_secs_valid() {
        // 2025-01-01T00:00:00Z → 1735689600
        let secs = parse_rfc3339_to_unix_secs("2025-01-01T00:00:00Z");
        assert_eq!(secs, Some(1_735_689_600));
    }

    #[test]
    fn parse_rfc3339_invalid_returns_none() {
        assert!(parse_rfc3339_to_unix_secs("not-a-date").is_none());
    }

    #[test]
    fn gemini_cli_has_access_token_empty_returns_false() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oauth_credentials.json");
        std::fs::write(&path, r#"{"token": "  "}"#).unwrap();
        assert!(!gemini_cli_has_access_token(&path));
    }

    #[test]
    fn gemini_cli_has_access_token_valid_returns_true() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oauth_credentials.json");
        std::fs::write(&path, r#"{"token": "ya29.real"}"#).unwrap();
        assert!(gemini_cli_has_access_token(&path));
    }
}
