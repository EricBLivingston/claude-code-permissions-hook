#![forbid(unsafe_code)]
#![warn(clippy::all)]

use crate::hook_io::HookInput;
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
    writeln!(flock, "---")?;

    flock.unlock().map_err(|(_, e)| e)?;

    Ok(())
}
