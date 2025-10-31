#![forbid(unsafe_code)]
#![warn(clippy::all)]

use crate::hook_io::{HookInput, HookOutput};
use chrono::{DateTime, Utc};
use log::warn;
use nix::fcntl::{Flock, FlockArg};
use serde::Serialize;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Serialize)]
struct LogEntry {
    timestamp: DateTime<Utc>,
    session_id: String,
    tool_name: String,
    tool_input: serde_json::Value,
    cwd: String,
}

#[derive(Debug, Serialize)]
struct LlmDecisionLogEntry {
    timestamp: DateTime<Utc>,
    session_id: String,
    tool_name: String,
    tool_input: serde_json::Value,
    llm_assessment: String,
    decision: String,
    reasoning: String,
}

pub fn log_tool_use(log_path: &Path, input: &HookInput) {
    if let Err(e) = try_log_tool_use(log_path, input) {
        warn!("Failed to log tool use: {}", e);
    }
}

fn try_log_tool_use(log_path: &Path, input: &HookInput) -> anyhow::Result<()> {
    let entry = LogEntry {
        timestamp: Utc::now(),
        session_id: input.session_id.clone(),
        tool_name: input.tool_name.clone(),
        tool_input: input.tool_input.clone(),
        cwd: input.cwd.clone(),
    };

    let json_line = serde_json::to_string(&entry)?;

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;

    let mut flock = Flock::lock(file, FlockArg::LockExclusive).map_err(|(_, e)| e)?;

    writeln!(flock, "{}", json_line)?;

    flock.unlock().map_err(|(_, e)| e)?;

    Ok(())
}

pub fn log_llm_decision(log_path: &Path, input: &HookInput, assessment: &str, output: &HookOutput) {
    if let Err(e) = try_log_llm_decision(log_path, input, assessment, output) {
        warn!("Failed to log LLM decision: {}", e);
    }
}

fn try_log_llm_decision(
    log_path: &Path,
    input: &HookInput,
    assessment: &str,
    output: &HookOutput,
) -> anyhow::Result<()> {
    let entry = LlmDecisionLogEntry {
        timestamp: Utc::now(),
        session_id: input.session_id.clone(),
        tool_name: input.tool_name.clone(),
        tool_input: input.tool_input.clone(),
        llm_assessment: assessment.to_string(),
        decision: output.hook_specific_output.permission_decision.clone(),
        reasoning: output
            .hook_specific_output
            .permission_decision_reason
            .clone(),
    };

    let json_line = serde_json::to_string(&entry)?;

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;

    let mut flock = Flock::lock(file, FlockArg::LockExclusive).map_err(|(_, e)| e)?;

    writeln!(flock, "{}", json_line)?;

    flock.unlock().map_err(|(_, e)| e)?;

    Ok(())
}
