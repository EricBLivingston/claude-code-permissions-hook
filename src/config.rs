#![forbid(unsafe_code)]
#![warn(clippy::all)]

use anyhow::{Context, Result};
use regex::Regex;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use toml::{Table, Value};

#[derive(Debug, Deserialize)]
pub struct Config {
    pub logging: LoggingConfig,
    #[serde(default)]
    pub allow: Vec<RuleConfig>,
    #[serde(default)]
    pub deny: Vec<RuleConfig>,
    #[serde(default)]
    pub llm_fallback: LlmFallbackConfig,
}

#[derive(Debug, Deserialize)]
struct IncludesSection {
    #[serde(default)]
    files: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoggingConfig {
    pub log_file: PathBuf,
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

fn default_log_level() -> String {
    "info".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct LlmFallbackConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
    #[serde(default)]
    pub actions: ActionPolicy,
}

impl Default for LlmFallbackConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: default_endpoint(),
            model: default_model(),
            api_key: None,
            timeout_secs: default_timeout_secs(),
            temperature: default_temperature(),
            max_retries: default_max_retries(),
            system_prompt: default_system_prompt(),
            actions: ActionPolicy::default(),
        }
    }
}

fn default_endpoint() -> String {
    "http://localhost:11434/v1".to_string()
}

fn default_model() -> String {
    "llama3.2:3b".to_string()
}

fn default_timeout_secs() -> u64 {
    5
}

fn default_temperature() -> f32 {
    0.1
}

fn default_max_retries() -> u32 {
    2
}

fn default_system_prompt() -> String {
    r#"You are a CONSERVATIVE security auditor. Your task is to assess proposed operations for risk and only classify as SAFE if you have 100% confidence.
Protecting system integrity is paramount.

CONTEXT:
You are auditing requests from an AI agent (Claude Code) that wants to interact with a developer's system.
Your job is to prevent: accidental system damage, credential theft, privilege escalation, and data destruction.
Only allow operations you're CERTAIN are safe to perform on the given (appropriate) files.

EXAMPLES:
- Read /home/user/project/README.md → SAFE: Standard project file
- Bash "rm -rf /" → UNSAFE: Destructive system-wide deletion
- Read /etc/passwd → UNSAFE: System credential file
- Bash "curl http://api.example.com" → UNKNOWN: Network operation unclear intent
- Read /home/user/../../../etc/shadow → UNSAFE: Path traversal to sensitive file

CLASSIFICATION RULES (apply in this order):

1. UNSAFE (100% confidence ONLY):
   - System paths: /etc/*, /root/*, /sys/*, /usr/*, /proc/self/*
   - Destructive: rm -rf, dd, mkfs, fdisk, shred
   - Privilege: sudo, su, chmod/chown on system files
   - Shell injection: ; && || ` $() | & (in commands)
   - Credentials: /etc/passwd, /etc/shadow, *.ssh/*, *.pem, *.key
   - Network+exec: curl|wget with |bash or &&
   - System mods: crontab, systemctl, service, useradd, usermod
   - Persistence: writes to ~/.bashrc, ~/.profile, /etc/profile.d/

2. UNKNOWN (default for doubt):
   - Path traversal: ../ anywhere
   - Ambiguous paths: /dev/*, /var/*, /opt/*, relative paths
   - Unfamiliar tools or commands
   - Context-dependent: terraform, ansible, docker, kubectl
   - Network: curl, wget (could be API or exploit)
   - Debug tools: strace, gdb, ltrace
   - ANY uncertainty

3. SAFE (100% confidence ONLY):
   - Reads: ONLY /home/<user>/project/*, /tmp/test* (NO path traversal)
   - Dev commands: cargo build|test|check|clippy|fmt, npm install|test|run|build,git status|log|diff|commit|push|pull, pytest, go test, make
   - Writes: ONLY to /home/<user>/project/*, /tmp/test*
   - Info: ls, cat, echo, ps, netstat (not redirecting to system paths)"#.to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct ActionPolicy {
    #[serde(default = "default_on_safe")]
    pub on_safe: Action,
    #[serde(default = "default_on_unsafe")]
    pub on_unsafe: Action,
    #[serde(default = "default_on_unknown")]
    pub on_unknown: Action,
    #[serde(default = "default_on_timeout")]
    pub on_timeout: Action,
    #[serde(default = "default_on_error")]
    pub on_error: Action,
}

impl Default for ActionPolicy {
    fn default() -> Self {
        Self {
            on_safe: default_on_safe(),
            on_unsafe: default_on_unsafe(),
            on_unknown: default_on_unknown(),
            on_timeout: default_on_timeout(),
            on_error: default_on_error(),
        }
    }
}

fn default_on_safe() -> Action {
    Action::Allow
}

fn default_on_unsafe() -> Action {
    Action::Deny
}

fn default_on_unknown() -> Action {
    Action::PassThrough
}

fn default_on_timeout() -> Action {
    Action::PassThrough
}

fn default_on_error() -> Action {
    Action::PassThrough
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Allow,
    Deny,
    PassThrough,
}

#[derive(Debug, Deserialize)]
pub struct RuleConfig {
    pub tool: Option<String>,
    pub tool_regex: Option<String>,
    pub tool_exclude_regex: Option<String>,
    pub file_path_regex: Option<String>,
    pub file_path_exclude_regex: Option<String>,
    pub command_regex: Option<String>,
    pub command_exclude_regex: Option<String>,
    pub subagent_type: Option<String>,
    pub subagent_type_exclude_regex: Option<String>,
    pub prompt_regex: Option<String>,
    pub prompt_exclude_regex: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub tool: Option<String>,
    pub tool_regex: Option<Regex>,
    pub tool_exclude_regex: Option<Regex>,
    pub file_path_regex: Option<Regex>,
    pub file_path_exclude_regex: Option<Regex>,
    pub command_regex: Option<Regex>,
    pub command_exclude_regex: Option<Regex>,
    pub subagent_type: Option<String>,
    pub subagent_type_exclude_regex: Option<Regex>,
    pub prompt_regex: Option<Regex>,
    pub prompt_exclude_regex: Option<Regex>,
}

impl Config {
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let merged_toml = Self::load_with_includes(path)?;
        
        let config: Config = toml::from_str(&merged_toml.to_string())
            .with_context(|| format!("Failed to parse TOML config: {}", path.display()))?;

        Ok(config)
    }

    fn load_with_includes(path: &Path) -> Result<Table> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        let mut toml_table: Table = toml::from_str(&contents)
            .with_context(|| format!("Failed to parse TOML config: {}", path.display()))?;

        let base_dir = path.parent().unwrap_or_else(|| Path::new("."));

        // Process includes if they exist
        if let Some(Value::Table(includes_section)) = toml_table.remove("includes") {
            if let Some(Value::Array(files)) = includes_section.get("files") {
                for file_value in files {
                    if let Value::String(include_path) = file_value {
                        // Resolve path: absolute if starts with /, relative to base_dir otherwise
                        let include_file = if include_path.starts_with('/') {
                            PathBuf::from(include_path)
                        } else {
                            base_dir.join(include_path)
                        };

                        let include_table = Self::load_with_includes(&include_file)
                            .with_context(|| format!("Failed to load included file: {}", include_file.display()))?;

                        // Merge include_table into toml_table, with toml_table taking precedence
                        Self::merge_tables(&mut toml_table, include_table);
                    }
                }
            }
        }

        Ok(toml_table)
    }

    fn merge_tables(base: &mut Table, other: Table) {
        for (key, value) in other {
            match (base.get_mut(&key), value) {
                (Some(Value::Table(base_table)), Value::Table(other_table)) => {
                    // Recursively merge tables
                    Self::merge_tables(base_table, other_table);
                }
                (Some(_), _) => {
                    // Base table already has this key, keep the base value (base takes precedence)
                }
                (None, value) => {
                    // Base table doesn't have this key, add it from other
                    base.insert(key, value);
                }
            }
        }
    }

    pub fn compile_rules(&self) -> Result<(Vec<Rule>, Vec<Rule>)> {
        let deny_rules = self
            .deny
            .iter()
            .map(compile_rule)
            .collect::<Result<Vec<_>>>()
            .context("Failed to compile deny rules")?;

        let allow_rules = self
            .allow
            .iter()
            .map(compile_rule)
            .collect::<Result<Vec<_>>>()
            .context("Failed to compile allow rules")?;

        Ok((deny_rules, allow_rules))
    }
}

fn compile_rule(rule_config: &RuleConfig) -> Result<Rule> {
    // Validate XOR: exactly one of tool or tool_regex must be specified
    match (&rule_config.tool, &rule_config.tool_regex) {
        (Some(_), Some(_)) => anyhow::bail!("Rule cannot have both 'tool' and 'tool_regex'"),
        (None, None) => anyhow::bail!("Rule must have either 'tool' or 'tool_regex'"),
        _ => {}
    }

    let tool_regex = rule_config
        .tool_regex
        .as_ref()
        .map(|s| Regex::new(s))
        .transpose()
        .context("Invalid tool_regex")?;

    let tool_exclude_regex = rule_config
        .tool_exclude_regex
        .as_ref()
        .map(|s| Regex::new(s))
        .transpose()
        .context("Invalid tool_exclude_regex")?;

    let file_path_regex = rule_config
        .file_path_regex
        .as_ref()
        .map(|s| Regex::new(s))
        .transpose()
        .context("Invalid file_path_regex")?;

    let file_path_exclude_regex = rule_config
        .file_path_exclude_regex
        .as_ref()
        .map(|s| Regex::new(s))
        .transpose()
        .context("Invalid file_path_exclude_regex")?;

    let command_regex = rule_config
        .command_regex
        .as_ref()
        .map(|s| Regex::new(s))
        .transpose()
        .context("Invalid command_regex")?;

    let command_exclude_regex = rule_config
        .command_exclude_regex
        .as_ref()
        .map(|s| Regex::new(s))
        .transpose()
        .context("Invalid command_exclude_regex")?;

    let subagent_type_exclude_regex = rule_config
        .subagent_type_exclude_regex
        .as_ref()
        .map(|s| Regex::new(s))
        .transpose()
        .context("Invalid subagent_type_exclude_regex")?;

    let prompt_regex = rule_config
        .prompt_regex
        .as_ref()
        .map(|s| Regex::new(s))
        .transpose()
        .context("Invalid prompt_regex")?;

    let prompt_exclude_regex = rule_config
        .prompt_exclude_regex
        .as_ref()
        .map(|s| Regex::new(s))
        .transpose()
        .context("Invalid prompt_exclude_regex")?;

    Ok(Rule {
        tool: rule_config.tool.clone(),
        tool_regex,
        tool_exclude_regex,
        file_path_regex,
        file_path_exclude_regex,
        command_regex,
        command_exclude_regex,
        subagent_type: rule_config.subagent_type.clone(),
        subagent_type_exclude_regex,
        prompt_regex,
        prompt_exclude_regex,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_compile_rule() -> Result<()> {
        let rule_config = RuleConfig {
            tool: Some("Read".to_string()),
            tool_regex: None,
            tool_exclude_regex: None,
            file_path_regex: Some(r"^/home/.*".to_string()),
            file_path_exclude_regex: Some(r"\.\.".to_string()),
            command_regex: None,
            command_exclude_regex: None,
            subagent_type: None,
            subagent_type_exclude_regex: None,
            prompt_regex: None,
            prompt_exclude_regex: None,
        };

        let rule = compile_rule(&rule_config)?;
        assert_eq!(rule.tool, Some("Read".to_string()));
        assert!(rule.file_path_regex.is_some());
        assert!(rule.file_path_exclude_regex.is_some());

        Ok(())
    }
}
