#![forbid(unsafe_code)]
#![warn(clippy::all)]
#![warn(rust_2018_idioms)]
#![warn(rust_2024_compatibility)]
#![warn(deprecated_safe)]

pub mod config;
pub mod hook_io;
pub mod logging;
pub mod matcher;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use env_logger::Env;
use log::info;
use std::path::PathBuf;

use crate::config::Config;
use crate::hook_io::{HookInput, HookOutput};
use crate::logging::log_tool_use;
use crate::matcher::{Decision, check_rules};

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
    },
    /// Validate a configuration file
    Validate {
        #[clap(short, long, value_parser)]
        config: PathBuf,
    },
}

fn run_hook(config_path: PathBuf) -> Result<()> {
    let config = Config::load_from_file(&config_path).context("Failed to load configuration")?;

    let (deny_rules, allow_rules) = config.compile_rules().context("Failed to compile rules")?;

    let input = HookInput::read_from_stdin().context("Failed to read hook input")?;

    // Log tool use (non-fatal)
    log_tool_use(&config.logging.log_file, &input);

    // Check deny rules first
    if let Some(decision) = check_rules(&deny_rules, &input) {
        let reason = match decision {
            Decision::Deny(r) | Decision::Allow(r) => r,
        };
        let output = HookOutput::deny(reason);
        output.write_to_stdout()?;
        return Ok(());
    }

    // Check allow rules
    if let Some(decision) = check_rules(&allow_rules, &input) {
        match decision {
            Decision::Allow(reason) => {
                let output = HookOutput::allow(reason);
                output.write_to_stdout()?;
                return Ok(());
            }
            Decision::Deny(reason) => {
                let output = HookOutput::deny(reason);
                output.write_to_stdout()?;
                return Ok(());
            }
        }
    }

    // No match - exit with no output (normal flow)
    Ok(())
}

fn validate_config(config_path: PathBuf) -> Result<()> {
    let config = Config::load_from_file(&config_path).context("Failed to load configuration")?;

    let (deny_rules, allow_rules) = config.compile_rules().context("Failed to compile rules")?;

    info!("Configuration is valid!");
    info!("  Deny rules: {}", deny_rules.len());
    info!("  Allow rules: {}", allow_rules.len());
    info!("  Log file: {}", config.logging.log_file.display());
    info!("  Log level: {}", config.logging.log_level);

    Ok(())
}

fn main() -> Result<()> {
    let opts = Opts::parse();

    // Load config to get log level
    let config_path = match &opts.command {
        Commands::Run { config } | Commands::Validate { config } => config,
    };

    let config = Config::load_from_file(config_path).context("Failed to load configuration")?;

    // Initialize logger with config log_level, unless RUST_LOG is already set
    env_logger::Builder::from_env(Env::default().default_filter_or(&config.logging.log_level))
        .init();

    match opts.command {
        Commands::Run { config } => run_hook(config),
        Commands::Validate { config } => validate_config(config),
    }
}
