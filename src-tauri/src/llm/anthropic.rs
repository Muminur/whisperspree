use crate::{
    error::Error,
    llm::{prompts, verifier, ModelTier, ProcessRequest, ProcessedText, TextProcessor},
    network::guard::NetworkGuard,
};
use async_trait::async_trait;
use std::time::{Duration, Instant};

const MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
const FAST_MODEL: &str = "claude-haiku-4-5";
const QUALITY_MODEL: &str = "claude-sonnet-4-6";

/// Explicit, dormant-until-called Anthropic boundary. Construction creates no
/// socket; `process` is the sole method that can submit a request.
pub struct AnthropicProcessor {
    api_key: String,
    network: NetworkGuard,
    timeout: Duration,
}
impl AnthropicProcessor {
    pub fn new(api_key: impl Into<String>, timeout: Duration) -> Result<Self, Error> {
        Ok(Self {
            api_key: api_key.into(),
            network: NetworkGuard::new().map_err(|e| Error::LlmTimeout(e.to_string()))?,
            timeout,
        })
    }
}

#[async_trait]
impl TextProcessor for AnthropicProcessor {
    async fn process(&self, req: ProcessRequest) -> Result<ProcessedText, Error> {
        let prompt = prompts::assemble(&req);
        let budget = max_tokens(req.raw_text.len());
        let deadline = RetryDeadline::new(self.timeout);
        let first = self
            .call(&prompt, req.model_tier, budget, &deadline)
            .await?;
        let text = if first.1 {
            self.call(&prompt, req.model_tier, budget.saturating_mul(2), &deadline)
                .await?
                .0
        } else {
            first.0
        };
        verifier::verify(&req.raw_text, &text)?;
        Ok(ProcessedText { text })
    }
}

impl AnthropicProcessor {
    async fn call(
        &self,
        prompt: &prompts::AssembledPrompt,
        tier: ModelTier,
        budget: u32,
        deadline: &RetryDeadline,
    ) -> Result<(String, bool), Error> {
        let body = request_body(prompt, tier, budget);
        for attempt in 0..2 {
            let request = self
                .network
                .request_method(MESSAGES_URL, reqwest::Method::POST)
                .map_err(|e| Error::LlmTimeout(e.to_string()))?
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .body(serde_json::to_vec(&body).expect("request JSON is serializable"));
            let response = tokio::time::timeout(deadline.remaining()?, request.send())
                .await
                .map_err(|_| {
                    Error::LlmTimeout("Anthropic request exceeded postprocess.timeoutMs".into())
                })?
                .map_err(|e| Error::LlmTimeout(e.to_string()))?;
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(Error::LlmAuth("Anthropic API key was rejected".into()));
            }
            if (response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
                || response.status().is_server_error())
                && attempt == 0
            {
                tokio::time::sleep(deadline.retry_backoff()?).await;
                continue;
            }
            if !response.status().is_success() {
                return Err(Error::LlmTimeout(format!(
                    "Anthropic returned HTTP {}",
                    response.status()
                )));
            }
            let value =
                tokio::time::timeout(deadline.remaining()?, response.json::<serde_json::Value>())
                    .await
                    .map_err(|_| {
                        Error::LlmTimeout(
                            "Anthropic response parsing exceeded postprocess.timeoutMs".into(),
                        )
                    })?
                    .map_err(|e| Error::LlmTimeout(e.to_string()))?;
            let maxed =
                value.get("stop_reason").and_then(serde_json::Value::as_str) == Some("max_tokens");
            return Ok((extract_text(&value)?, maxed));
        }
        unreachable!("retry loop returns on its final attempt")
    }
}

/// A monotonic deadline shared by every attempt for one `process` call.
struct RetryDeadline {
    ends_at: Instant,
}

impl RetryDeadline {
    const RETRY_BACKOFF: Duration = Duration::from_millis(500);

    fn new(timeout: Duration) -> Self {
        Self {
            ends_at: Instant::now() + timeout,
        }
    }

    fn remaining(&self) -> Result<Duration, Error> {
        self.ends_at
            .checked_duration_since(Instant::now())
            .ok_or_else(|| {
                Error::LlmTimeout("Anthropic request exceeded postprocess.timeoutMs".into())
            })
    }

    fn retry_backoff(&self) -> Result<Duration, Error> {
        Self::retry_backoff_for(self.remaining()?)
    }

    fn retry_backoff_for(remaining: Duration) -> Result<Duration, Error> {
        if remaining < Self::RETRY_BACKOFF {
            Err(Error::LlmTimeout(
                "Anthropic retry would exceed postprocess.timeoutMs".into(),
            ))
        } else {
            Ok(Self::RETRY_BACKOFF)
        }
    }
}

pub fn max_tokens(chars: usize) -> u32 {
    1024_u32.max(((chars / 3) as u32).saturating_mul(2))
}
fn request_body(
    prompt: &prompts::AssembledPrompt,
    tier: ModelTier,
    budget: u32,
) -> serde_json::Value {
    serde_json::json!({"model": match tier { ModelTier::Fast => FAST_MODEL, ModelTier::Quality => QUALITY_MODEL }, "max_tokens": budget, "temperature": match tier { ModelTier::Fast => 0.2, ModelTier::Quality => 0.4 }, "system": prompt.system, "messages": [{"role":"user", "content":prompt.user}]})
}
fn extract_text(value: &serde_json::Value) -> Result<String, Error> {
    let text = value
        .get("content")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(serde_json::Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(serde_json::Value::as_str))
        .collect::<String>();
    if text.is_empty() {
        Err(Error::LlmVerify(
            "Anthropic response contained no text blocks".into(),
        ))
    } else {
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::prompts::AssembledPrompt;

    #[test]
    fn section_10_1_request_uses_pinned_messages_shape_and_fast_parameters() {
        let body = request_body(
            &AssembledPrompt {
                system: "system".into(),
                user: "user".into(),
            },
            ModelTier::Fast,
            max_tokens(1),
        );
        assert_eq!(body["model"], "claude-haiku-4-5");
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "user");
    }

    #[test]
    fn section_10_1_quality_uses_quality_model_and_transform_temperature() {
        let body = request_body(
            &AssembledPrompt {
                system: "system".into(),
                user: "user".into(),
            },
            ModelTier::Quality,
            max_tokens(2_100),
        );
        assert_eq!(body["model"], "claude-sonnet-4-6");
        assert_eq!(body["temperature"], 0.4);
        assert_eq!(body["max_tokens"], 1400);
    }

    #[test]
    fn section_10_1_concatenates_all_text_response_blocks() {
        let response = serde_json::json!({"content":[{"type":"thinking","thinking":"..."},{"type":"text","text":"Hello"},{"type":"text","text":" world"}]});
        assert_eq!(extract_text(&response).unwrap(), "Hello world");
    }

    #[test]
    fn section_10_1_retry_is_not_started_when_its_500ms_backoff_exceeds_overall_deadline() {
        assert!(RetryDeadline::retry_backoff_for(Duration::from_millis(499)).is_err());
    }

    #[test]
    fn section_10_1_retry_uses_the_single_remaining_overall_budget() {
        assert_eq!(
            RetryDeadline::retry_backoff_for(Duration::from_millis(500)).unwrap(),
            Duration::from_millis(500)
        );
    }
}
