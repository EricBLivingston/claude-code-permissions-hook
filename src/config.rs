#![forbid(unsafe_code)]
#![warn(clippy::all)]

use anyhow::{Context, Result};
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use toml::{Table, Value};

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub llm_fallback: LlmFallbackConfig,
    #[serde(default)]
    pub includes: IncludesConfig,
    #[serde(flatten)]
    pub sections: HashMap<String, SectionConfig>,
}

#[derive(Debug, Deserialize, Default)]
pub struct IncludesConfig {
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct SectionConfig {
    pub description: Option<String>,
    #[serde(default = "default_priority")]
    pub priority: u32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub allow: Vec<RuleConfig>,
    #[serde(default)]
    pub deny: Vec<RuleConfig>,
}

fn default_priority() -> u32 {
    50
}

fn default_enabled() -> bool {
    true
}

pub struct CompiledConfig {
    pub logging: LoggingConfig,
    pub llm_fallback: LlmFallbackConfig,
    pub deny_rules: Vec<Rule>,
    pub allow_rules: Vec<Rule>,
}

#[derive(Debug, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_file")]
    pub log_file: PathBuf,
    #[serde(default = "default_review_log_file")]
    pub review_log_file: PathBuf,
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            log_file: default_log_file(),
            review_log_file: default_review_log_file(),
            log_level: default_log_level(),
        }
    }
}

fn default_log_file() -> PathBuf {
    PathBuf::from("/tmp/claude-tool-use.log")
}

fn default_review_log_file() -> PathBuf {
    PathBuf::from("/tmp/claude-decisions-review.log")
}

fn default_log_level() -> String {
    "info".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct LlmFallbackConfig {
    #[serde(default)]
    pub enabled: bool,
    // REQUIRED when enabled=true - no default to avoid silent misconfigurations
    pub endpoint: Option<String>,
    // REQUIRED when enabled=true - no default to avoid silent misconfigurations
    pub model: Option<String>,
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
    pub provider_preferences: Option<Vec<String>>,
}

impl LlmFallbackConfig {
    /// Validate LLM fallback configuration
    /// Returns detailed error messages if enabled but misconfigured
    pub fn validate(&self) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        // When enabled, endpoint and model are REQUIRED
        if self.endpoint.is_none() {
            anyhow::bail!(
                "LLM fallback is enabled but 'endpoint' is not specified.\n\
                 Please add: endpoint = \"https://openrouter.ai/api/v1\" (for cloud)\n\
                 or: endpoint = \"http://localhost:11434/v1\" (for Ollama)"
            );
        }

        if self.model.is_none() {
            anyhow::bail!(
                "LLM fallback is enabled but 'model' is not specified.\n\
                 Please add: model = \"anthropic/claude-haiku-4.5\" (for OpenRouter)\n\
                 or: model = \"dolphin-llama3:8b-v2.9-q8_0\" (for Ollama)"
            );
        }

        // Validate endpoint format
        let endpoint = self.endpoint.as_ref().unwrap();
        if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
            anyhow::bail!(
                "Invalid LLM endpoint '{}' - must start with http:// or https://",
                endpoint
            );
        }

        Ok(())
    }
}

impl Default for LlmFallbackConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: None,
            model: None,
            api_key: None,
            timeout_secs: default_timeout_secs(),
            temperature: default_temperature(),
            max_retries: default_max_retries(),
            system_prompt: default_system_prompt(),
            provider_preferences: None,
        }
    }
}

fn default_timeout_secs() -> u64 {
    60
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


#[derive(Debug, Deserialize)]
pub struct RuleConfig {
    // REQUIRED - validation will check this
    pub id: String,
    #[serde(default)]
    pub description: Option<String>,

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
    pub id: String,
    pub section_name: String,
    pub description: Option<String>,

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
    pub fn load_from_file(path: &Path) -> Result<CompiledConfig> {
        let merged_toml = Self::load_with_includes(path)?;

        let config: Config = toml::from_str(&merged_toml.to_string())
            .with_context(|| format!("Failed to parse TOML config: {}", path.display()))?;

        config.validate()?;
        config.compile()
    }

    fn validate(&self) -> Result<()> {
        const RESERVED_NAMES: &[&str] = &["logging", "llm_fallback", "includes"];
        let kebab_case_regex = Regex::new(r"^[a-z][a-z0-9-]*$").unwrap();

        // Check for reserved section names
        for reserved in RESERVED_NAMES {
            if self.sections.contains_key(*reserved) {
                anyhow::bail!(
                    "Invalid section name '{}' - this is a reserved name. \
                     Reserved names: logging, llm_fallback, includes",
                    reserved
                );
            }
        }

        // Validate kebab-case section names
        for section_name in self.sections.keys() {
            if !kebab_case_regex.is_match(section_name) {
                anyhow::bail!(
                    "Invalid section name '{}' - section names must be kebab-case \
                     (lowercase letters, numbers, and hyphens only, starting with a letter). \
                     Example: 'build-tools', 'file-operations'",
                    section_name
                );
            }
        }

        // Validate rule ID uniqueness globally
        let mut seen_ids = std::collections::HashSet::new();
        for (section_name, section) in &self.sections {
            for rule in section.deny.iter().chain(section.allow.iter()) {
                if !seen_ids.insert(&rule.id) {
                    anyhow::bail!(
                        "Duplicate rule ID '{}' in section '{}'. \
                         Rule IDs must be unique across all sections.",
                        rule.id,
                        section_name
                    );
                }
            }
        }

        Ok(())
    }

    fn compile(self) -> Result<CompiledConfig> {
        // Collect sections with their names and sort by priority
        let mut sections: Vec<(String, SectionConfig)> = self.sections.into_iter()
            .filter(|(_, section)| section.enabled)
            .collect();

        // Sort by priority (lower number = higher priority), then alphabetically by name
        sections.sort_by(|(name_a, section_a), (name_b, section_b)| {
            section_a.priority.cmp(&section_b.priority)
                .then_with(|| name_a.cmp(name_b))
        });

        // Flatten deny rules in priority order
        let mut deny_rules = Vec::new();
        for (section_name, section) in &sections {
            for rule_config in &section.deny {
                let rule = compile_rule(rule_config, section_name)?;
                deny_rules.push(rule);
            }
        }

        // Flatten allow rules in priority order
        let mut allow_rules = Vec::new();
        for (section_name, section) in &sections {
            for rule_config in &section.allow {
                let rule = compile_rule(rule_config, section_name)?;
                allow_rules.push(rule);
            }
        }

        Ok(CompiledConfig {
            logging: self.logging,
            llm_fallback: self.llm_fallback,
            deny_rules,
            allow_rules,
        })
    }

    fn load_with_includes(path: &Path) -> Result<Table> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        let mut toml_table: Table = toml::from_str(&contents)
            .with_context(|| format!("Failed to parse TOML config: {}", path.display()))?;

        let base_dir = path.parent().unwrap_or_else(|| Path::new("."));

        // Collect include paths first to avoid borrow checker issues
        let include_paths: Vec<PathBuf> = if let Some(Value::Table(includes_section)) = toml_table.get("includes") {
            if let Some(Value::Array(files)) = includes_section.get("files") {
                files.iter()
                    .filter_map(|file_value| {
                        if let Value::String(include_path) = file_value {
                            // Resolve path: absolute if starts with /, relative to base_dir otherwise
                            Some(if include_path.starts_with('/') {
                                PathBuf::from(include_path)
                            } else {
                                base_dir.join(include_path)
                            })
                        } else {
                            None
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        // Now load and merge includes
        for include_file in include_paths {
            let include_table = Self::load_with_includes(&include_file)
                .with_context(|| format!("Failed to load included file: {}", include_file.display()))?;

            // Merge include_table into toml_table, with toml_table taking precedence
            Self::merge_tables(&mut toml_table, include_table);
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
}

fn compile_rule(rule_config: &RuleConfig, section_name: &str) -> Result<Rule> {
    // Validate XOR: exactly one of tool or tool_regex must be specified
    match (&rule_config.tool, &rule_config.tool_regex) {
        (Some(_), Some(_)) => anyhow::bail!(
            "Rule '{}' in section '{}' cannot have both 'tool' and 'tool_regex'",
            rule_config.id,
            section_name
        ),
        (None, None) => anyhow::bail!(
            "Rule '{}' in section '{}' must have either 'tool' or 'tool_regex'",
            rule_config.id,
            section_name
        ),
        _ => {}
    }

    let tool_regex = rule_config
        .tool_regex
        .as_ref()
        .map(|s| Regex::new(s))
        .transpose()
        .with_context(|| format!("Invalid tool_regex in rule '{}' (section '{}')", rule_config.id, section_name))?;

    let tool_exclude_regex = rule_config
        .tool_exclude_regex
        .as_ref()
        .map(|s| Regex::new(s))
        .transpose()
        .with_context(|| format!("Invalid tool_exclude_regex in rule '{}' (section '{}')", rule_config.id, section_name))?;

    let file_path_regex = rule_config
        .file_path_regex
        .as_ref()
        .map(|s| Regex::new(s))
        .transpose()
        .with_context(|| format!("Invalid file_path_regex in rule '{}' (section '{}')", rule_config.id, section_name))?;

    let file_path_exclude_regex = rule_config
        .file_path_exclude_regex
        .as_ref()
        .map(|s| Regex::new(s))
        .transpose()
        .with_context(|| format!("Invalid file_path_exclude_regex in rule '{}' (section '{}')", rule_config.id, section_name))?;

    let command_regex = rule_config
        .command_regex
        .as_ref()
        .map(|s| Regex::new(s))
        .transpose()
        .with_context(|| format!("Invalid command_regex in rule '{}' (section '{}')", rule_config.id, section_name))?;

    let command_exclude_regex = rule_config
        .command_exclude_regex
        .as_ref()
        .map(|s| Regex::new(s))
        .transpose()
        .with_context(|| format!("Invalid command_exclude_regex in rule '{}' (section '{}')", rule_config.id, section_name))?;

    let subagent_type_exclude_regex = rule_config
        .subagent_type_exclude_regex
        .as_ref()
        .map(|s| Regex::new(s))
        .transpose()
        .with_context(|| format!("Invalid subagent_type_exclude_regex in rule '{}' (section '{}')", rule_config.id, section_name))?;

    let prompt_regex = rule_config
        .prompt_regex
        .as_ref()
        .map(|s| Regex::new(s))
        .transpose()
        .with_context(|| format!("Invalid prompt_regex in rule '{}' (section '{}')", rule_config.id, section_name))?;

    let prompt_exclude_regex = rule_config
        .prompt_exclude_regex
        .as_ref()
        .map(|s| Regex::new(s))
        .transpose()
        .with_context(|| format!("Invalid prompt_exclude_regex in rule '{}' (section '{}')", rule_config.id, section_name))?;

    Ok(Rule {
        id: rule_config.id.clone(),
        section_name: section_name.to_string(),
        description: rule_config.description.clone(),
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
            id: "test-read-rule".to_string(),
            description: Some("Test rule for reading home directory".to_string()),
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

        let rule = compile_rule(&rule_config, "test-section")?;
        assert_eq!(rule.id, "test-read-rule");
        assert_eq!(rule.section_name, "test-section");
        assert_eq!(rule.tool, Some("Read".to_string()));
        assert!(rule.file_path_regex.is_some());
        assert!(rule.file_path_exclude_regex.is_some());

        Ok(())
    }
}
