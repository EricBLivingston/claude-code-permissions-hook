#![forbid(unsafe_code)]
#![warn(clippy::all)]
#![warn(rust_2018_idioms)]
#![warn(rust_2024_compatibility)]
#![warn(deprecated_safe)]

pub mod config;
pub mod hook_io;
pub mod llm_safety;
pub mod logging;
pub mod matcher;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use env_logger::Env;
use log::info;
use std::path::PathBuf;

use crate::config::Config;
use crate::hook_io::{HookInput, HookOutput};
use crate::logging::{log_decision, create_rule_metadata};
use crate::matcher::{check_rules, DecisionType};

#[derive(Debug, Parser)]
#[clap(author, version, about = "Claude Code command permissions hook")]
struct Opts {
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run the hook (reads JSON from stdin, outputs decision to stdout)
    Run {
        #[clap(short, long, value_parser)]
        config: PathBuf,
        /// Test mode: always output decisions (including Query/Timeout/Error) for testing
        #[clap(long)]
        test_mode: bool,
    },
    /// Validate a configuration file
    Validate {
        #[clap(short, long, value_parser)]
        config: PathBuf,
    },
}

async fn run_hook(config_path: PathBuf, test_mode: bool) -> Result<()> {
    let compiled = Config::load_from_file(&config_path).context("Failed to load configuration")?;

    let input = HookInput::read_from_stdin().context("Failed to read hook input")?;

    // Check deny rules first
    if let Some(decision_info) = check_rules(&compiled.deny_rules, &input) {
        let output = HookOutput::deny(decision_info.reasoning.clone());

        let rule_metadata = create_rule_metadata(
            &compiled.deny_rules[decision_info.rule_index],
            decision_info.rule_index,
            "deny",
            &config_path,
            &decision_info.matched_pattern,
        );

        log_decision(
            &compiled.logging.log_file,
            &compiled.logging.review_log_file,
            &input,
            "deny",
            "rule",
            &decision_info.reasoning,
            Some(rule_metadata),
            None,
        );

        output.write_to_stdout()?;
        return Ok(());
    }

    // Check allow rules
    if let Some(decision_info) = check_rules(&compiled.allow_rules, &input) {
        let decision_str = match decision_info.decision {
            DecisionType::Allow => "allow",
            DecisionType::Deny => "deny",
        };

        let output = match decision_info.decision {
            DecisionType::Allow => HookOutput::allow(decision_info.reasoning.clone()),
            DecisionType::Deny => HookOutput::deny(decision_info.reasoning.clone()),
        };

        let rule_metadata = create_rule_metadata(
            &compiled.allow_rules[decision_info.rule_index],
            decision_info.rule_index,
            "allow",
            &config_path,
            &decision_info.matched_pattern,
        );

        log_decision(
            &compiled.logging.log_file,
            &compiled.logging.review_log_file,
            &input,
            decision_str,
            "rule",
            &decision_info.reasoning,
            Some(rule_metadata),
            None,
        );

        output.write_to_stdout()?;
        return Ok(());
    }

    // No match - check LLM fallback if enabled
    if compiled.llm_fallback.enabled {
        info!("No rules matched - using LLM fallback");
        let result = llm_safety::assess_with_llm(&compiled.llm_fallback, &input).await;
        if let Some((output, llm_metadata)) = llm_safety::apply_llm_result(&input, result, test_mode) {
            let decision_str = if output.hook_specific_output.permission_decision == "allow" {
                "allow"
            } else {
                "deny"
            };

            log_decision(
                &compiled.logging.log_file,
                &compiled.logging.review_log_file,
                &input,
                decision_str,
                "llm",
                &output.hook_specific_output.permission_decision_reason,
                None,
                Some(llm_metadata),
            );

            output.write_to_stdout()?;
            return Ok(());
        }
    }

    // No match and no LLM decision - passthrough
    log_decision(
        &compiled.logging.log_file,
        &compiled.logging.review_log_file,
        &input,
        "passthrough",
        "passthrough",
        "No rule or LLM decision - passed to user",
        None,
        None,
    );

    Ok(())
}

fn validate_config(config_path: PathBuf) -> Result<()> {
    let compiled = Config::load_from_file(&config_path).context("Failed to load configuration")?;

    // Validate LLM fallback configuration if enabled
    compiled.llm_fallback.validate().context("Invalid LLM fallback configuration")?;

    info!("Configuration is valid!");
    info!("  Deny rules: {}", compiled.deny_rules.len());
    info!("  Allow rules: {}", compiled.allow_rules.len());
    info!("  Operational log: {}", compiled.logging.log_file.display());
    info!("  Review log: {}", compiled.logging.review_log_file.display());
    info!("  Log level: {}", compiled.logging.log_level);
    if compiled.llm_fallback.enabled {
        info!("  LLM fallback: ENABLED");
        info!("    Endpoint: {}", compiled.llm_fallback.endpoint.as_ref().unwrap());
        info!("    Model: {}", compiled.llm_fallback.model.as_ref().unwrap());
        info!("    Timeout: {}s", compiled.llm_fallback.timeout_secs);
    } else {
        info!("  LLM fallback: disabled");
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let opts = Opts::parse();

    // Load config to get log level
    let config_path = match &opts.command {
        Commands::Run { config, .. } | Commands::Validate { config } => config,
    };

    let config = Config::load_from_file(config_path).context("Failed to load configuration")?;

    // Initialize logger with config log_level, unless RUST_LOG is already set
    env_logger::Builder::from_env(Env::default().default_filter_or(&config.logging.log_level))
        .init();

    match opts.command {
        Commands::Run { config, test_mode } => run_hook(config, test_mode).await,
        Commands::Validate { config } => validate_config(config),
    }
}
