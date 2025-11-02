#![forbid(unsafe_code)]
#![warn(clippy::all)]

use crate::config::Rule;
use crate::hook_io::HookInput;
use chrono::{DateTime, Utc};
use log::warn;
use nix::fcntl::{Flock, FlockArg};
use serde::Serialize;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

// ========== OPERATIONAL LOG (SIMPLIFIED) ==========
// Purpose: Quick monitoring with minimal overhead
// Location: /tmp/claude-tool-use.log (or configured path)

#[derive(Debug, Serialize)]
struct OperationalLogEntry {
    timestamp: DateTime<Utc>,
    session_id: String,
    tool_name: String,
    tool_input: serde_json::Value,
    decision: String,          // "allow", "deny", or "passthrough"
    decision_source: String,   // "rule", "llm", or "passthrough"
}

// ========== REVIEW LOG (ENRICHED) ==========
// Purpose: Comprehensive audit trail for post-processing analysis
// Location: /tmp/claude-decisions-review.log

#[derive(Debug, Serialize)]
struct ReviewLogEntry {
    timestamp: DateTime<Utc>,
    session_id: String,
    tool_name: String,
    tool_input: serde_json::Value,
    cwd: String,

    // Decision context
    decision: String,          // "allow", "deny", or "passthrough"
    decision_source: String,   // "rule", "llm", or "passthrough"
    reasoning: String,

    // Rule-based enrichment (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    rule_metadata: Option<RuleMetadata>,

    // LLM-based enrichment (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    llm_metadata: Option<LlmMetadata>,

    // Review flags
    review_flags: ReviewFlags,
}

#[derive(Debug, Serialize)]
pub struct RuleMetadata {
    pub rule_id: String,           // Human-readable identifier (REQUIRED in new format)
    pub section_name: String,      // Section name (NEW in Phase 1)
    pub rule_type: String,         // "allow" or "deny"
    pub rule_index: usize,         // Position in ruleset (0-based)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_description: Option<String>,
    pub config_file: String,       // Path to config file
    pub matched_pattern: String,   // Which pattern triggered (e.g., "command_regex")
}

#[derive(Debug, Serialize)]
pub struct LlmMetadata {
    pub assessment: String,        // "ALLOW" or "QUERY"
    pub reasoning: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>, // "high", "medium", "low" (future enhancement)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processing_time_ms: Option<u64>,
    pub model: String,
}

#[derive(Debug, Serialize)]
pub struct ReviewFlags {
    pub needs_review: bool,
    pub risk_level: String,        // "low", "medium", "high"
    pub reasons: Vec<String>,      // Why flagged for review
}

// ========== PUBLIC LOGGING API ==========

/// Log a decision to BOTH operational and review logs
pub fn log_decision(
    operational_log: &Path,
    review_log: &Path,
    input: &HookInput,
    decision: &str,
    decision_source: &str,
    reasoning: &str,
    rule_metadata: Option<RuleMetadata>,
    llm_metadata: Option<LlmMetadata>,
) {
    // Compute review flags
    let review_flags = compute_review_flags(
        decision,
        decision_source,
        &input.tool_name,
        &input.tool_input,
        reasoning,
        &llm_metadata,
    );

    // Log to operational log (simple)
    let op_entry = OperationalLogEntry {
        timestamp: Utc::now(),
        session_id: input.session_id.clone(),
        tool_name: input.tool_name.clone(),
        tool_input: input.tool_input.clone(),
        decision: decision.to_string(),
        decision_source: decision_source.to_string(),
    };
    if let Err(e) = write_log_entry(operational_log, &op_entry) {
        warn!("Failed to log to operational log: {}", e);
    }

    // Log to review log (detailed)
    let review_entry = ReviewLogEntry {
        timestamp: Utc::now(),
        session_id: input.session_id.clone(),
        tool_name: input.tool_name.clone(),
        tool_input: input.tool_input.clone(),
        cwd: input.cwd.clone(),
        decision: decision.to_string(),
        decision_source: decision_source.to_string(),
        reasoning: reasoning.to_string(),
        rule_metadata,
        llm_metadata,
        review_flags,
    };
    if let Err(e) = write_log_entry(review_log, &review_entry) {
        warn!("Failed to log to review log: {}", e);
    }
}

/// Helper to create RuleMetadata from a matched rule
pub fn create_rule_metadata(
    rule: &Rule,
    rule_index: usize,
    rule_type: &str,
    config_path: &Path,
    matched_pattern: &str,
) -> RuleMetadata {
    RuleMetadata {
        rule_id: rule.id.clone(),
        section_name: rule.section_name.clone(),
        rule_type: rule_type.to_string(),
        rule_index,
        rule_description: rule.description.clone(),
        config_file: config_path.display().to_string(),
        matched_pattern: matched_pattern.to_string(),
    }
}

/// Helper to create LlmMetadata
pub fn create_llm_metadata(
    assessment: &str,
    reasoning: &str,
    model: &str,
    processing_time_ms: Option<u64>,
    confidence: Option<String>,
) -> LlmMetadata {
    LlmMetadata {
        assessment: assessment.to_string(),
        reasoning: reasoning.to_string(),
        confidence,
        processing_time_ms,
        model: model.to_string(),
    }
}

// ========== INTERNAL HELPERS ==========

/// Compute review flags based on decision context
fn compute_review_flags(
    decision: &str,
    decision_source: &str,
    tool_name: &str,
    tool_input: &serde_json::Value,
    reasoning: &str,
    _llm_metadata: &Option<LlmMetadata>,
) -> ReviewFlags {
    let mut needs_review = false;
    let mut reasons = Vec::new();
    let mut risk_level = "low".to_string();

    // Flag LLM allows for risky patterns
    if decision == "allow" && decision_source == "llm" {
        let input_str = tool_input.to_string().to_lowercase();
        let reasoning_lower = reasoning.to_lowercase();

        // Check for risky patterns
        if tool_name == "Bash" {
            if input_str.contains("rm ") || input_str.contains("delete") {
                needs_review = true;
                risk_level = "high".to_string();
                reasons.push("LLM allowed Bash command with deletion".to_string());
            }
            if input_str.contains("curl") && input_str.contains("|") {
                needs_review = true;
                risk_level = "high".to_string();
                reasons.push("LLM allowed piped shell execution".to_string());
            }
            if input_str.contains("sudo") {
                needs_review = true;
                risk_level = "high".to_string();
                reasons.push("LLM allowed sudo command".to_string());
            }
        }

        // Check for low confidence indicators in reasoning
        if reasoning_lower.contains("uncertain")
            || reasoning_lower.contains("unclear")
            || reasoning_lower.contains("might") {
            needs_review = true;
            if risk_level == "low" {
                risk_level = "medium".to_string();
            }
            reasons.push("LLM reasoning indicates uncertainty".to_string());
        }
    }

    // Flag LLM queries of common safe patterns (might be too conservative)
    if decision == "deny" && decision_source == "llm" {
        let input_str = tool_input.to_string().to_lowercase();
        if input_str.contains("cargo test")
            || input_str.contains("npm install")
            || input_str.contains("git status") {
            needs_review = true;
            risk_level = "medium".to_string();
            reasons.push("LLM queried common safe development command".to_string());
        }
    }

    // Flag passthroughs for audit (no rule or LLM decision made)
    if decision_source == "passthrough" {
        needs_review = true;
        risk_level = "medium".to_string();
        reasons.push("No rule or LLM decision - passed through to user".to_string());
    }

    ReviewFlags {
        needs_review,
        risk_level,
        reasons,
    }
}

/// Generic log writer with file locking
fn write_log_entry<T: Serialize>(log_path: &Path, entry: &T) -> anyhow::Result<()> {
    let json_line = serde_json::to_string(entry)?;

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;

    let mut flock = Flock::lock(file, FlockArg::LockExclusive).map_err(|(_, e)| e)?;

    writeln!(flock, "{}", json_line)?;

    flock.unlock().map_err(|(_, e)| e)?;

    Ok(())
}

