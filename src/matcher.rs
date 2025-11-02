#![forbid(unsafe_code)]
#![warn(clippy::all)]

use crate::config::Rule;
use crate::hook_io::HookInput;
use log::{debug, trace};

#[derive(Debug, Clone)]
pub struct DecisionInfo {
    pub decision: DecisionType,
    pub reasoning: String,
    pub rule_index: usize,
    pub matched_pattern: String,
    pub rule_id: String,
    pub section_name: String,
}

#[derive(Debug, Clone)]
pub enum DecisionType {
    Allow,
    Deny,
}

pub fn check_rules(rules: &[Rule], input: &HookInput) -> Option<DecisionInfo> {
    trace!("Checking {} rules for {}", rules.len(), input.tool_name);

    for (idx, rule) in rules.iter().enumerate() {
        // Check if tool matches (exact or regex)
        let tool_matches = if let Some(ref exact_tool) = rule.tool {
            exact_tool == &input.tool_name
        } else if let Some(ref regex_tool) = rule.tool_regex {
            if !regex_tool.is_match(&input.tool_name) {
                false
            } else if let Some(ref exclude_regex) = rule.tool_exclude_regex {
                if exclude_regex.is_match(&input.tool_name) {
                    debug!("Rule {} tool matched but excluded: {}", idx, input.tool_name);
                    false
                } else {
                    true
                }
            } else {
                true
            }
        } else {
            false
        };

        if !tool_matches {
            trace!("Rule {} skipped - tool mismatch", idx);
            continue;
        }

        trace!("Evaluating rule {} for {}", idx, input.tool_name);
        if let Some((reasoning, pattern)) = check_rule(rule, input) {
            debug!("Rule {} matched: {}", idx, pattern);
            return Some(DecisionInfo {
                decision: DecisionType::Allow,
                reasoning,
                rule_index: idx,
                matched_pattern: pattern,
                rule_id: rule.id.clone(),
                section_name: rule.section_name.clone(),
            });
        }
    }
    trace!("No rules matched for {}", input.tool_name);
    None
}

fn check_rule(rule: &Rule, input: &HookInput) -> Option<(String, String)> {
    match input.tool_name.as_str() {
        "Read" | "Write" | "Edit" | "Glob" => {
            if let Some(file_path) = input.extract_field("file_path")
                && check_field_with_exclude(
                    &file_path,
                    &rule.file_path_regex,
                    &rule.file_path_exclude_regex,
                )
            {
                let reasoning = format!("Rule {}, file_path: {}", input.tool_name, file_path);
                return Some((reasoning, "file_path_regex".to_string()));
            }
        }
        "Bash" => {
            if let Some(command) = input.extract_field("command")
                && check_field_with_exclude(
                    &command,
                    &rule.command_regex,
                    &rule.command_exclude_regex,
                )
            {
                let reasoning = format!("Bash, command: {}", command);
                return Some((reasoning, "command_regex".to_string()));
            }
        }
        "Task" => {
            if let Some(subagent_type) = input.extract_field("subagent_type")
                && check_subagent_type(rule, &subagent_type)
            {
                let reasoning = format!("Task, subagent: {}", subagent_type);
                return Some((reasoning, "subagent_type".to_string()));
            }
            if let Some(prompt) = input.extract_field("prompt")
                && check_field_with_exclude(&prompt, &rule.prompt_regex, &rule.prompt_exclude_regex)
            {
                let reasoning = "Task, prompt pattern matched".to_string();
                return Some((reasoning, "prompt_regex".to_string()));
            }
        }
        _ => {
            // MCP tools: auto-allow if no field patterns specified
            if rule.file_path_regex.is_none()
                && rule.command_regex.is_none()
                && rule.subagent_type.is_none()
                && rule.prompt_regex.is_none()
            {
                let reasoning = format!("Tool: {}", input.tool_name);
                return Some((reasoning, "tool_regex".to_string()));
            }
        }
    }

    None
}

fn check_field_with_exclude(
    value: &str,
    main_regex: &Option<regex::Regex>,
    exclude_regex: &Option<regex::Regex>,
) -> bool {
    if let Some(regex) = main_regex {
        if !regex.is_match(value) {
            trace!("Main regex no match: {}", value);
            return false;
        }
        if let Some(exclude) = exclude_regex
            && exclude.is_match(value)
        {
            trace!("Exclude regex matched: {}", value);
            return false;
        }
        true
    } else {
        false
    }
}

fn check_subagent_type(rule: &Rule, subagent_type: &str) -> bool {
    if let Some(ref expected) = rule.subagent_type {
        if expected != subagent_type {
            return false;
        }
        if let Some(ref exclude_regex) = rule.subagent_type_exclude_regex
            && exclude_regex.is_match(subagent_type)
        {
            trace!("Subagent type excluded: {}", subagent_type);
            return false;
        }
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Rule;
    use regex::Regex;

    #[test]
    fn test_check_field_with_exclude() {
        let main_regex = Some(Regex::new(r"^/home/").unwrap());
        let exclude_regex = Some(Regex::new(r"\.\.").unwrap());

        assert!(check_field_with_exclude(
            "/home/user/file.txt",
            &main_regex,
            &exclude_regex
        ));
        assert!(!check_field_with_exclude(
            "/home/user/../etc/passwd",
            &main_regex,
            &exclude_regex
        ));
        assert!(!check_field_with_exclude(
            "/etc/passwd",
            &main_regex,
            &exclude_regex
        ));
    }

    #[test]
    fn test_check_subagent_type() {
        let rule = Rule {
            id: "test-task".to_string(),
            section_name: "test-section".to_string(),
            description: None,
            tool: Some("Task".to_string()),
            tool_regex: None,
            tool_exclude_regex: None,
            file_path_regex: None,
            file_path_exclude_regex: None,
            command_regex: None,
            command_exclude_regex: None,
            subagent_type: Some("Explore".to_string()),
            subagent_type_exclude_regex: None,
            prompt_regex: None,
            prompt_exclude_regex: None,
        };

        assert!(check_subagent_type(&rule, "Explore"));
        assert!(!check_subagent_type(&rule, "Plan"));
    }
}
