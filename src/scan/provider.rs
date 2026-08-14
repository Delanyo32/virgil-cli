//! Provider selection for the scan agents.
//!
//! ponytail: provider construction only — no retry/config layers until needed.
//!
//! Cersei's agent runner reads the model id off the *agent* builder
//! (`AgentBuilder::model`), never off the provider, so `resolve_model` is what
//! callers must feed to `Agent::builder().model(..)`. Only the two
//! `OpenAi::builder()` arms below are also told the model, because that
//! builder requires one; the `from_env` arms take no model at all.

use anyhow::{Context, Result};
use cersei::prelude::Provider;
use cersei::{Anthropic, OpenAi};
use clap::ValueEnum;

use crate::cli::ProviderKind;

pub fn default_model(kind: ProviderKind) -> Option<&'static str> {
    match kind {
        ProviderKind::Anthropic => Some("claude-opus-5"),
        // openai/ollama/openrouter have no sane default: the user must pass --model
        _ => None,
    }
}

/// Resolve the model id: `--model` wins, else the provider default, else error.
pub fn resolve_model(kind: ProviderKind, model: Option<&str>) -> Result<String> {
    model
        .map(str::to_string)
        .or_else(|| default_model(kind).map(str::to_string))
        // Name the provider the way the user spelled it on the command
        // line, not the way Rust debug-prints the enum.
        .with_context(|| format!("--model is required for provider {}", flag_name(kind)))
}

/// The `--provider` spelling clap accepts for this variant.
fn flag_name(kind: ProviderKind) -> String {
    kind.to_possible_value()
        .map(|v| v.get_name().to_string())
        .unwrap_or_else(|| format!("{kind:?}"))
}

/// Build the LLM client. Boxed because the four arms are two different concrete
/// types; `AgentBuilder::provider_boxed` takes exactly this.
///
/// Construction is offline — no request is sent until the agent runs.
pub fn build(kind: ProviderKind, model: &str) -> Result<Box<dyn Provider>> {
    Ok(match kind {
        ProviderKind::Anthropic => {
            require_key("ANTHROPIC_API_KEY")?;
            Box::new(Anthropic::from_env()?)
        }
        ProviderKind::Openai => {
            require_key("OPENAI_API_KEY")?;
            Box::new(OpenAi::from_env()?)
        }
        ProviderKind::Ollama => Box::new(
            OpenAi::builder()
                .base_url("http://localhost:11434/v1")
                .api_key("ollama") // Ollama ignores the key but the builder requires one
                .model(model)
                .build()?,
        ),
        ProviderKind::Openrouter => Box::new(
            OpenAi::builder()
                .base_url("https://openrouter.ai/api/v1")
                .api_key(require_key("OPENROUTER_API_KEY")?)
                .model(model)
                .build()?,
        ),
    })
}

/// Read an API key, rejecting one that is missing *or* set to an empty
/// string. Cersei's `from_env` only rejects the first case: an empty key
/// sails through construction and comes back as one HTTP 401 per review,
/// several seconds and one whole parse later.
fn require_key(var: &str) -> Result<String> {
    check_key(var, std::env::var(var).ok())
}

/// The check itself, split out so a test can hit both arms without
/// mutating process-wide environment state (which is `unsafe` in edition
/// 2024, and racy across parallel test threads either way).
fn check_key(var: &str, value: Option<String>) -> Result<String> {
    let key = value.unwrap_or_default();
    anyhow::ensure!(
        !key.trim().is_empty(),
        "{var} is not set, or is set to an empty value — export a real key, \
or pass --provider ollama to run locally"
    );
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_default_model() {
        assert_eq!(
            resolve_model(ProviderKind::Anthropic, None).unwrap(),
            "claude-opus-5"
        );
    }

    #[test]
    fn explicit_model_wins_over_default() {
        assert_eq!(
            resolve_model(ProviderKind::Anthropic, Some("claude-sonnet-4-6")).unwrap(),
            "claude-sonnet-4-6"
        );
    }

    /// The error must name the provider the way `--provider` spells it,
    /// so the message can be pasted straight back onto the command line.
    #[test]
    fn openai_requires_model_flag() {
        let err = resolve_model(ProviderKind::Openai, None)
            .unwrap_err()
            .to_string();
        assert_eq!(err, "--model is required for provider openai");
    }

    #[test]
    fn ollama_provider_constructs_without_network() {
        let p = build(ProviderKind::Ollama, "qwen3").unwrap();
        assert_eq!(p.name(), "openai");
    }

    /// An unset key and a key set to "" must both fail here, before the
    /// project is parsed — not later as one HTTP 401 per review. The
    /// message has to name the variable, since that is what the user
    /// must go and export.
    #[test]
    fn a_missing_or_empty_key_is_rejected_by_name() {
        for absent in [None, Some(String::new()), Some("   ".to_string())] {
            let err = check_key("ANTHROPIC_API_KEY", absent.clone())
                .unwrap_err()
                .to_string();
            assert!(err.contains("ANTHROPIC_API_KEY"), "got: {err} ({absent:?})");
        }
    }

    #[test]
    fn a_real_key_passes_through_unchanged() {
        assert_eq!(
            check_key("OPENAI_API_KEY", Some("sk-abc".into())).unwrap(),
            "sk-abc"
        );
    }
}
