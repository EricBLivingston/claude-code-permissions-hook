#![forbid(unsafe_code)]
#![warn(clippy::all)]

use crate::config::LlmFallbackConfig;
use crate::hook_io::{HookInput, HookOutput};
use crate::logging::{create_llm_metadata, LlmMetadata};
use anyhow::{Context, Result};
use log::{debug, error, info, warn};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::time::timeout;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafetyAssessment {
    Allow(String),  // reasoning - operation is clearly safe, auto-approve
    Query(String),  // reasoning - needs user review (unsafe, ambiguous, or uncertain)
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
/// Returns (result, processing_time_ms)
pub async fn assess_with_llm(config: &LlmFallbackConfig, input: &HookInput) -> (AssessmentResult, u64) {
    debug!("Starting LLM assessment for {}", input.tool_name);

    let start = Instant::now();

    let result = timeout(
        Duration::from_secs(config.timeout_secs),
        call_llm(config, input),
    )
    .await;

    let processing_time_ms = start.elapsed().as_millis() as u64;

    let assessment_result = match result {
        Ok(Ok(assessment)) => {
            debug!("LLM assessment completed in {}ms: {:?}", processing_time_ms, assessment);
            AssessmentResult::Assessment(assessment)
        }
        Ok(Err(e)) => {
            error!("LLM call failed after {}ms: {}", processing_time_ms, e);
            AssessmentResult::Error(e.to_string())
        }
        Err(_) => {
            warn!("LLM timeout after {}ms", processing_time_ms);
            AssessmentResult::Timeout
        }
    };

    (assessment_result, processing_time_ms)
}

/// Apply LLM result and create metadata
/// Returns Option<(HookOutput, LlmMetadata)>
pub fn apply_llm_result(
    _input: &HookInput,
    result: (AssessmentResult, u64),
    test_mode: bool,
) -> Option<(HookOutput, LlmMetadata)> {
    use AssessmentResult::*;
    use SafetyAssessment::*;

    let (assessment_result, processing_time_ms) = result;

    // Get model from config - simplified for now
    let model = "llm-fallback".to_string();

    match assessment_result {
        Assessment(Allow(r)) => {
            let reasoning = format!("LLM: {}", r);
            info!("LLM Allow: {}", reasoning);
            let hook_output = HookOutput::allow(reasoning.clone());
            let metadata = create_llm_metadata(
                "ALLOW",
                &r,
                &model,
                Some(processing_time_ms),
                None,
            );
            Some((hook_output, metadata))
        }
        Assessment(Query(r)) => {
            let reasoning = format!("LLM Query: {}", r);
            info!("{}", reasoning);
            let hook_output = HookOutput::deny(reasoning.clone());
            let metadata = create_llm_metadata(
                "QUERY",
                &r,
                &model,
                Some(processing_time_ms),
                None,
            );
            // In test mode, output; otherwise pass through
            if test_mode {
                Some((hook_output, metadata))
            } else {
                None
            }
        }
        Timeout => {
            warn!("LLM timeout");
            let hook_output = HookOutput::deny("LLM timeout".to_string());
            let metadata = create_llm_metadata(
                "TIMEOUT",
                "Request timed out",
                &model,
                Some(processing_time_ms),
                None,
            );
            if test_mode {
                Some((hook_output, metadata))
            } else {
                None
            }
        }
        Error(e) => {
            error!("LLM error: {}", e);
            let hook_output = HookOutput::deny(format!("LLM error: {}", e));
            let metadata = create_llm_metadata(
                "ERROR",
                &e,
                &model,
                Some(processing_time_ms),
                None,
            );
            if test_mode {
                Some((hook_output, metadata))
            } else {
                None
            }
        }
    }
}

async fn call_llm(config: &LlmFallbackConfig, input: &HookInput) -> Result<SafetyAssessment> {
    // Validate configuration (should have been caught by validate command, but double-check)
    let endpoint = config.endpoint.as_ref()
        .context("LLM endpoint not configured - this should have been caught during validation")?;
    let model = config.model.as_ref()
        .context("LLM model not configured - this should have been caught during validation")?;

    let prompt = build_safety_prompt(input);

    // Retry loop for malformed JSON responses
    for attempt in 0..=config.max_retries {
        if attempt > 0 {
            info!("LLM retry attempt {}/{}", attempt, config.max_retries);
        }

        debug!("LLM prompt (attempt {}):\n{}", attempt + 1, prompt);

        // Build request JSON
        // Note: keep_alive doesn't work with OpenAI-compatible endpoint
        // Set OLLAMA_KEEP_ALIVE=1h environment variable for Ollama instead
        let mut request_json = serde_json::json!({
            "model": model,
            "temperature": config.temperature,
            "messages": [
                {
                    "role": "system",
                    "content": config.system_prompt
                },
                {
                    "role": "user",
                    "content": prompt
                }
            ]
        });
        
        // Add provider preferences if specified (OpenRouter-specific)
        if let Some(ref providers) = config.provider_preferences {
            if !providers.is_empty() {
                if let Some(obj) = request_json.as_object_mut() {
                    obj.insert(
                        "provider".to_string(),
                        serde_json::json!({"order": providers})
                    );
                }
            }
        }
        
        let request_payload = serde_json::to_string_pretty(&request_json).unwrap_or_default();
        info!("=== REQUEST PAYLOAD ===\n{}", request_payload);
        info!("=== END PAYLOAD ===");

        // Make HTTP request
        info!("Sending request to: {}/chat/completions", endpoint);
        info!("API key present: {}", config.api_key.as_ref().map_or("NO", |k| if k.is_empty() { "EMPTY" } else { "YES" }));
        info!("Timeout: {} seconds", config.timeout_secs);

        let response = reqwest::Client::new()
                    .post(format!("{}/chat/completions", endpoint))
                    .header("Content-Type", "application/json")
                    .header("Authorization", format!("Bearer {}", config.api_key.as_deref().unwrap_or("")))
                    .json(&request_json)
                    .timeout(std::time::Duration::from_secs(config.timeout_secs))
                    .send()
            .await;
        
        let response = match response {
            Ok(resp) => {
                info!("HTTP status: {}", resp.status());
                resp
            }
            Err(e) => {
                if e.is_timeout() {
                    error!("Request TIMEOUT after {} seconds", config.timeout_secs);
                } else if e.is_connect() {
                    error!("Connection failed: {}", e);
                } else {
                    error!("Request failed: {}", e);
                }
                error!("Full error details: {:?}", e);
                return Err(anyhow::anyhow!("Failed to send LLM request: {}", e));
            }
        };
        
        let response_text = match response.text().await {
            Ok(text) => {
                debug!("Response length: {} chars", text.len());
                text
            }
            Err(e) => {
                error!("Failed to read response text: {}", e);
                return Err(anyhow::anyhow!("Failed to read LLM response: {}", e));
            }
        };
        
        debug!("LLM raw API response: {}", response_text);
        
        let api_response: serde_json::Value = serde_json::from_str(&response_text)
            .context("Failed to parse LLM API response as JSON")?;
        
        let content = api_response["choices"][0]["message"]["content"]
            .as_str()
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

    format!(r#"Evaluate this tool use request:

Tool: {}
Parameters:
{}

Classify as ALLOW or QUERY following your instructions above. Respond in this exact JSON format:
{{
  "classification": "ALLOW|QUERY",
  "reasoning": "brief explanation"
}}"#,
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
        "ALLOW" => Ok(SafetyAssessment::Allow(response.reasoning)),
        "QUERY" => Ok(SafetyAssessment::Query(response.reasoning)),
        other => anyhow::bail!("Invalid classification '{}' - must be ALLOW or QUERY", other),
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
        let json = r#"{"classification": "ALLOW", "reasoning": "Read-only operation"}"#;
        let result = parse_llm_response(json).unwrap();
        assert_eq!(
            result,
            SafetyAssessment::Allow("Read-only operation".to_string())
        );
    }

    #[test]
    fn test_parse_llm_response_with_preamble() {
        let response = r#"Sure, here's my assessment:
{"classification": "QUERY", "reasoning": "Destructive command"}
Hope this helps!"#;
        let result = parse_llm_response(response).unwrap();
        assert_eq!(
            result,
            SafetyAssessment::Query("Destructive command".to_string())
        );
    }

    #[test]
    fn test_parse_llm_response_markdown() {
        let json = r#"```json
{"classification": "ALLOW", "reasoning": "Safe operation"}
```"#;
        let result = parse_llm_response(json).unwrap();
        assert_eq!(
            result,
            SafetyAssessment::Allow("Safe operation".to_string())
        );
    }

    #[test]
    fn test_parse_llm_response_malformed_json() {
        // Trailing comma - simple_json_repair should fix this
        let json = r#"{"classification": "QUERY", "reasoning": "Cannot determine",}"#;
        let result = parse_llm_response(json).unwrap();
        assert_eq!(
            result,
            SafetyAssessment::Query("Cannot determine".to_string())
        );
    }

    #[test]
    fn test_parse_llm_response_legacy_unknown() {
        // Test legacy UNKNOWN classification (maps to Query)
        let json = r#"{"classification": "UNKNOWN", "reasoning": "Cannot determine"}"#;
        let result = parse_llm_response(json).unwrap();
        assert_eq!(
            result,
            SafetyAssessment::Query("Cannot determine".to_string())
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
