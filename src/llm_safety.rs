#![forbid(unsafe_code)]
#![warn(clippy::all)]

use crate::config::{Action, ActionPolicy, LlmFallbackConfig};
use crate::hook_io::{HookInput, HookOutput};
use crate::logging::log_llm_decision;
use anyhow::{Context, Result};
use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestUserMessageArgs, CreateChatCompletionRequestArgs,
    },
    Client,
};
use log::{debug, error, info, warn};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::timeout;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafetyAssessment {
    Safe(String),    // reasoning
    Unsafe(String),  // reasoning
    Unknown(String), // reasoning
}

#[derive(Debug)]
pub enum AssessmentResult {
    Assessment(SafetyAssessment),
    Timeout,
    Error(String),
}

#[derive(Debug, Serialize, Deserialize)]
struct LlmResponse {
    classification: String,
    reasoning: String,
}

/// Main entry point for LLM safety assessment
pub async fn assess_with_llm(config: &LlmFallbackConfig, input: &HookInput) -> AssessmentResult {
    debug!(
        "Starting LLM safety assessment for tool: {}",
        input.tool_name
    );

    let result = timeout(
        Duration::from_secs(config.timeout_secs),
        call_llm(config, input),
    )
    .await;

    match result {
        Ok(Ok(assessment)) => {
            debug!("LLM assessment completed: {:?}", assessment);
            AssessmentResult::Assessment(assessment)
        }
        Ok(Err(e)) => {
            error!("LLM call failed: {}", e);
            AssessmentResult::Error(e.to_string())
        }
        Err(_) => {
            warn!("LLM call timed out after {} seconds", config.timeout_secs);
            AssessmentResult::Timeout
        }
    }
}

/// Apply LLM result based on configured action policy
pub fn apply_llm_result(
    log_path: &std::path::Path,
    input: &HookInput,
    policy: &ActionPolicy,
    result: AssessmentResult,
) -> Option<HookOutput> {
    use AssessmentResult::*;
    use SafetyAssessment::*;

    let (action, reasoning, assessment_type) = match result {
        Assessment(Safe(r)) => (
            policy.on_safe,
            format!("LLM-SAFE: {}", r),
            "SAFE",
        ),
        Assessment(Unsafe(r)) => (
            policy.on_unsafe,
            format!("LLM-UNSAFE: {}", r),
            "UNSAFE",
        ),
        Assessment(Unknown(r)) => (
            policy.on_unknown,
            format!("LLM-UNKNOWN: {}", r),
            "UNKNOWN",
        ),
        Timeout => (
            policy.on_timeout,
            "LLM request timed out".to_string(),
            "TIMEOUT",
        ),
        Error(e) => (policy.on_error, format!("LLM error: {}", e), "ERROR"),
    };

    let output = match action {
        Action::Allow => {
            debug!("LLM result: ALLOW - {}", reasoning);
            Some(HookOutput::allow(reasoning))
        }
        Action::Deny => {
            debug!("LLM result: DENY - {}", reasoning);
            Some(HookOutput::deny(reasoning))
        }
        Action::PassThrough => {
            debug!("LLM result: PASS_THROUGH - {}", reasoning);
            None
        }
    };

    // Log the decision if we're making one
    if let Some(ref hook_output) = output {
        log_llm_decision(log_path, input, assessment_type, hook_output);
    }

    output
}

async fn call_llm(config: &LlmFallbackConfig, input: &HookInput) -> Result<SafetyAssessment> {
    // Configure OpenAI-compatible client
    let mut openai_config = OpenAIConfig::new().with_api_base(&config.endpoint);

    // Set API key if provided (not needed for local Ollama)
    if let Some(api_key) = &config.api_key
        && !api_key.is_empty()
    {
        openai_config = openai_config.with_api_key(api_key);
    }

    let client = Client::with_config(openai_config);
    let prompt = build_safety_prompt(input);
    
    // Retry loop for malformed JSON responses
    for attempt in 0..=config.max_retries {
        if attempt > 0 {
            info!("LLM retry attempt {}/{}", attempt, config.max_retries);
        }
        
        debug!("LLM prompt (attempt {}):\n{}", attempt + 1, prompt);

        let request = CreateChatCompletionRequestArgs::default()
            .model(&config.model)
            .temperature(config.temperature)
            .messages(vec![
                ChatCompletionRequestMessage::System(
                    ChatCompletionRequestSystemMessageArgs::default()
                        .content(config.system_prompt.clone())
                        .build()?,
                ),
                ChatCompletionRequestMessage::User(
                    ChatCompletionRequestUserMessageArgs::default()
                        .content(prompt.clone())
                        .build()?,
                ),
            ])
            .build()?;

        let response = client
            .chat()
            .create(request)
            .await
            .context("Failed to call LLM API")?;

        let content = response
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_ref())
            .context("No response content from LLM")?;

        debug!("LLM raw response (attempt {}): {}", attempt + 1, content);

        match parse_llm_response(content) {
            Ok(assessment) => {
                if attempt > 0 {
                    info!("LLM succeeded after {} retries", attempt);
                }
                return Ok(assessment);
            }
            Err(e) => {
                if attempt < config.max_retries {
                    warn!("Failed to parse LLM response (attempt {}): {}", attempt + 1, e);
                    continue;
                } else {
                    return Err(e).context(format!(
                        "Failed to parse LLM response after {} attempts",
                        config.max_retries + 1
                    ));
                }
            }
        }
    }

    unreachable!()
}

fn build_safety_prompt(input: &HookInput) -> String {
    let params =
        serde_json::to_string_pretty(&input.tool_input).unwrap_or_else(|_| "{}".to_string());

    format!(r#"Classify the following tool request as SAFE, UNSAFE, or UNKNOWN based on the above rules.

Tool: {}
Parameters:
{}

CRITICAL: When uncertain between SAFE and UNKNOWN, choose UNKNOWN.
CRITICAL: When uncertain between UNSAFE and UNKNOWN, choose UNKNOWN.

Respond in this exact JSON format:
{{
  "classification": "SAFE|UNSAFE|UNKNOWN",
  "reasoning": "brief explanation"
}}

Respond ONLY with valid JSON."#,
        input.tool_name, params
    )
}

fn parse_llm_response(content: &str) -> Result<SafetyAssessment> {
    // Extract JSON object using regex (finds content between outermost { })
    let json_regex = Regex::new(r"(?s)\{.*\}").context("Failed to compile JSON regex")?;
    
    let json_str = json_regex
        .find(content)
        .map(|m| m.as_str())
        .context("No JSON object found in LLM response")?;

    debug!("Extracted JSON candidate: {}", json_str);

    // Try direct parse first
    let response = match serde_json::from_str::<LlmResponse>(json_str) {
        Ok(r) => r,
        Err(e) => {
            // Try simple repairs for common issues
            let repaired = simple_json_repair(json_str);
            debug!("Applied simple repairs: {}", repaired);
            
            serde_json::from_str::<LlmResponse>(&repaired)
                .with_context(|| format!("Failed to parse JSON even after repair. Original error: {}", e))?
        }
    };

    // Validate and classify
    match response.classification.to_uppercase().as_str() {
        "SAFE" => Ok(SafetyAssessment::Safe(response.reasoning)),
        "UNSAFE" => Ok(SafetyAssessment::Unsafe(response.reasoning)),
        "UNKNOWN" => Ok(SafetyAssessment::Unknown(response.reasoning)),
        other => anyhow::bail!("Invalid classification '{}' - must be SAFE, UNSAFE, or UNKNOWN", other),
    }
}

/// Apply simple JSON repairs for common LLM mistakes
fn simple_json_repair(json: &str) -> String {
    json
        // Remove trailing commas before } or ]
        .replace(",}", "}")
        .replace(",]", "]")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_llm_response_plain() {
        let json = r#"{"classification": "SAFE", "reasoning": "Read-only operation"}"#;
        let result = parse_llm_response(json).unwrap();
        assert_eq!(
            result,
            SafetyAssessment::Safe("Read-only operation".to_string())
        );
    }

    #[test]
    fn test_parse_llm_response_with_preamble() {
        let response = r#"Sure, here's my assessment:
{"classification": "UNSAFE", "reasoning": "Destructive command"}
Hope this helps!"#;
        let result = parse_llm_response(response).unwrap();
        assert_eq!(
            result,
            SafetyAssessment::Unsafe("Destructive command".to_string())
        );
    }

    #[test]
    fn test_parse_llm_response_markdown() {
        let json = r#"```json
{"classification": "SAFE", "reasoning": "Safe operation"}
```"#;
        let result = parse_llm_response(json).unwrap();
        assert_eq!(
            result,
            SafetyAssessment::Safe("Safe operation".to_string())
        );
    }

    #[test]
    fn test_parse_llm_response_malformed_json() {
        // Trailing comma - simple_json_repair should fix this
        let json = r#"{"classification": "UNKNOWN", "reasoning": "Cannot determine",}"#;
        let result = parse_llm_response(json).unwrap();
        assert_eq!(
            result,
            SafetyAssessment::Unknown("Cannot determine".to_string())
        );
    }

    #[test]
    fn test_parse_llm_response_unknown() {
        let json = r#"{"classification": "UNKNOWN", "reasoning": "Cannot determine"}"#;
        let result = parse_llm_response(json).unwrap();
        assert_eq!(
            result,
            SafetyAssessment::Unknown("Cannot determine".to_string())
        );
    }

    #[test]
    fn test_parse_llm_response_invalid_classification() {
        let json = r#"{"classification": "MAYBE", "reasoning": "Unsure"}"#;
        assert!(parse_llm_response(json).is_err());
    }

    #[test]
    fn test_parse_llm_response_no_json() {
        let response = "This is just plain text without any JSON";
        assert!(parse_llm_response(response).is_err());
    }
}
