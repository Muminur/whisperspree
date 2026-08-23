//! LLM prompt assembly and the Anthropic processor boundary (T4.1).

pub mod anthropic;
pub mod prompts;
pub mod verifier;

use crate::error::Error;
use async_trait::async_trait;

/// The quality/latency model selection required by PRD §4.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTier {
    Fast,
    Quality,
}

/// A fully assembled request sent through a [`TextProcessor`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessRequest {
    pub raw_text: String,
    pub glossary: Vec<String>,
    pub style: prompts::StyleId,
    pub rewrite: prompts::RewriteBlock,
    pub language_rule: prompts::LanguageRule,
    pub app_name: String,
    pub model_tier: ModelTier,
}

/// The text returned by the provider after §7.4 verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessedText {
    pub text: String,
}

/// The provider boundary from PRD §9.3. Implementations must never cause a
/// network request until `process` is explicitly called.
#[async_trait]
pub trait TextProcessor: Send + Sync {
    async fn process(&self, req: ProcessRequest) -> Result<ProcessedText, Error>;
}
