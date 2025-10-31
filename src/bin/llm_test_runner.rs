#![forbid(unsafe_code)]
#![warn(clippy::all)]

use anyhow::{Context, Result};
use clap::Parser;
use csv::ReaderBuilder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Debug, Parser)]
#[clap(author, version, about = "LLM fallback test runner")]
struct Opts {
    /// Path to CSV file with test cases
    #[clap(long, default_value = "tests/llm_test_cases.csv")]
    csv: PathBuf,

    /// Path to config file with LLM enabled
    #[clap(short, long, default_value = "test-llm-config.toml")]
    config: PathBuf,

    /// Output markdown report path
    #[clap(short, long, default_value = "llm_test_report.md")]
    output: PathBuf,

    /// Output CSV results path
    #[clap(long, default_value = "llm_test_results.csv")]
    results_csv: PathBuf,

    /// Sample N random test cases (useful for quick testing)
    #[clap(short, long)]
    sample: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct TestCase {
    id: String,
    tool_name: String,
    expected_class: String,
    description: String,
    tool_input_key: String,
    tool_input_value: String,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
enum Classification {
    Safe,
    Unsafe,
    Unknown,
}

impl Classification {
    fn from_str(s: &str) -> Result<Self> {
        match s.to_uppercase().as_str() {
            "SAFE" => Ok(Classification::Safe),
            "UNSAFE" => Ok(Classification::Unsafe),
            "UNKNOWN" => Ok(Classification::Unknown),
            other => anyhow::bail!("Invalid classification: {}", other),
        }
    }

    fn as_str(&self) -> &str {
        match self {
            Classification::Safe => "SAFE",
            Classification::Unsafe => "UNSAFE",
            Classification::Unknown => "UNKNOWN",
        }
    }

    fn from_decision(decision: &str) -> Self {
        match decision {
            "allow" => Classification::Safe,
            "deny" => Classification::Unsafe,
            _ => Classification::Unknown,
        }
    }
}

#[derive(Debug, Serialize)]
struct TestResult {
    id: String,
    tool_name: String,
    tool_input_key: String,
    tool_input_value: String,
    expected_class: String,
    llm_decision: String,
    llm_class: String,
    llm_reasoning: String,
    correct: bool,
    error: Option<String>,
}

#[derive(Debug, Default)]
struct ClassMetrics {
    true_positives: usize,
    false_positives: usize,
    false_negatives: usize,
    true_negatives: usize,
}

impl ClassMetrics {
    fn precision(&self) -> f64 {
        let tp = self.true_positives as f64;
        let fp = self.false_positives as f64;
        if tp + fp == 0.0 {
            0.0
        } else {
            tp / (tp + fp)
        }
    }

    fn recall(&self) -> f64 {
        let tp = self.true_positives as f64;
        let fn_val = self.false_negatives as f64;
        if tp + fn_val == 0.0 {
            0.0
        } else {
            tp / (tp + fn_val)
        }
    }

    fn f1_score(&self) -> f64 {
        let p = self.precision();
        let r = self.recall();
        if p + r == 0.0 {
            0.0
        } else {
            2.0 * (p * r) / (p + r)
        }
    }
}

fn main() -> Result<()> {
    let opts = Opts::parse();

    println!("🧪 LLM Fallback Test Runner");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("CSV:           {}", opts.csv.display());
    println!("Config:        {}", opts.config.display());
    println!("Report:        {}", opts.output.display());
    println!("Results CSV:   {}", opts.results_csv.display());
    println!();

    // Load test cases
    println!("📁 Loading test cases...");
    let mut test_cases = load_test_cases(&opts.csv)?;
    println!("   Loaded {} test cases", test_cases.len());

    // Sample if requested
    if let Some(n) = opts.sample {
        if n < test_cases.len() {
            use rand::seq::SliceRandom;
            let mut rng = rand::thread_rng();
            test_cases.shuffle(&mut rng);
            test_cases.truncate(n);
            println!("   📊 Sampling {} random test cases", n);
        } else {
            println!("   ⚠️  Sample size {} >= total cases, using all", n);
        }
    }
    println!();

    // Run tests
    println!("🤖 Running tests (this will take a while)...");
    let results = run_tests(&test_cases, &opts.config)?;
    println!();

    // Calculate metrics
    println!("📊 Calculating metrics...");
    let (accuracy, per_class_metrics) = calculate_metrics(&results);
    println!();

    // Generate reports
    println!("📝 Generating reports...");
    write_markdown_report(&opts.output, &results, accuracy, &per_class_metrics)?;
    write_csv_results(&opts.results_csv, &results)?;
    println!();

    // Print summary
    print_summary(&results, accuracy, &per_class_metrics);

    Ok(())
}

fn load_test_cases(path: &PathBuf) -> Result<Vec<TestCase>> {
    let file = File::open(path).context("Failed to open CSV file")?;
    let mut reader = ReaderBuilder::new().has_headers(true).from_reader(file);

    let mut cases = Vec::new();
    for result in reader.deserialize() {
        let case: TestCase = result.context("Failed to parse CSV row")?;
        cases.push(case);
    }

    Ok(cases)
}

fn run_tests(test_cases: &[TestCase], config_path: &PathBuf) -> Result<Vec<TestResult>> {
    let mut results = Vec::new();
    let total = test_cases.len();

    for (idx, test_case) in test_cases.iter().enumerate() {
        print!("   [{:3}/{:3}] Testing {}: ", idx + 1, total, test_case.id);
        std::io::stdout().flush()?;

        let result = run_single_test(test_case, config_path);
        
        match &result.error {
            None => {
                if result.correct {
                    println!("✅ PASS");
                } else {
                    println!("❌ FAIL (expected: {}, got: {})", 
                        result.expected_class, result.llm_class);
                }
            }
            Some(err) => {
                println!("⚠️  ERROR: {}", err);
            }
        }

        results.push(result);
    }

    Ok(results)
}

fn run_single_test(test_case: &TestCase, config_path: &PathBuf) -> TestResult {
    // Generate HookInput JSON
    let hook_input = serde_json::json!({
        "session_id": format!("test-{}", test_case.id),
        "transcript_path": "/tmp/transcript.txt",
        "cwd": "/home/user/project",
        "hook_event_name": "PreToolUse",
        "tool_name": test_case.tool_name,
        "tool_input": {
            test_case.tool_input_key.clone(): test_case.tool_input_value.clone()
        }
    });

    let json_str = serde_json::to_string(&hook_input).unwrap();

    // Execute hook via subprocess (using release build for speed)
    let output = Command::new("cargo")
        .args(["run", "--quiet", "--release", "--bin", "claude-code-permissions-hook", "--", "run", "--config"])
        .arg(config_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(json_str.as_bytes())?;
            }
            child.wait_with_output()
        });

    let expected_class = Classification::from_str(&test_case.expected_class)
        .unwrap_or(Classification::Unknown);

    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            
            if stdout.trim().is_empty() {
                // No output = pass through = Unknown
                TestResult {
                    id: test_case.id.clone(),
                    tool_name: test_case.tool_name.clone(),
                    tool_input_key: test_case.tool_input_key.clone(),
                    tool_input_value: test_case.tool_input_value.clone(),
                    expected_class: expected_class.as_str().to_string(),
                    llm_decision: "pass_through".to_string(),
                    llm_class: Classification::Unknown.as_str().to_string(),
                    llm_reasoning: "No output (pass through)".to_string(),
                    correct: expected_class == Classification::Unknown,
                    error: None,
                }
            } else {
                // Parse JSON output
                match serde_json::from_str::<serde_json::Value>(&stdout) {
                    Ok(json) => {
                        let decision = json["hookSpecificOutput"]["permissionDecision"]
                            .as_str()
                            .unwrap_or("unknown");
                        let reasoning = json["hookSpecificOutput"]["permissionDecisionReason"]
                            .as_str()
                            .unwrap_or("No reasoning provided");

                        let llm_class = Classification::from_decision(decision);
                        
                        TestResult {
                            id: test_case.id.clone(),
                            tool_name: test_case.tool_name.clone(),
                            tool_input_key: test_case.tool_input_key.clone(),
                            tool_input_value: test_case.tool_input_value.clone(),
                            expected_class: expected_class.as_str().to_string(),
                            llm_decision: decision.to_string(),
                            llm_class: llm_class.as_str().to_string(),
                            llm_reasoning: reasoning.to_string(),
                            correct: expected_class == llm_class,
                            error: None,
                        }
                    }
                    Err(e) => TestResult {
                        id: test_case.id.clone(),
                        tool_name: test_case.tool_name.clone(),
                        tool_input_key: test_case.tool_input_key.clone(),
                        tool_input_value: test_case.tool_input_value.clone(),
                        expected_class: expected_class.as_str().to_string(),
                        llm_decision: "error".to_string(),
                        llm_class: "ERROR".to_string(),
                        llm_reasoning: "".to_string(),
                        correct: false,
                        error: Some(format!("Failed to parse JSON: {}", e)),
                    },
                }
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            TestResult {
                id: test_case.id.clone(),
                tool_name: test_case.tool_name.clone(),
                tool_input_key: test_case.tool_input_key.clone(),
                tool_input_value: test_case.tool_input_value.clone(),
                expected_class: expected_class.as_str().to_string(),
                llm_decision: "error".to_string(),
                llm_class: "ERROR".to_string(),
                llm_reasoning: "".to_string(),
                correct: false,
                error: Some(format!("Process failed: {}", stderr)),
            }
        }
        Err(e) => TestResult {
            id: test_case.id.clone(),
            tool_name: test_case.tool_name.clone(),
            tool_input_key: test_case.tool_input_key.clone(),
            tool_input_value: test_case.tool_input_value.clone(),
            expected_class: expected_class.as_str().to_string(),
            llm_decision: "error".to_string(),
            llm_class: "ERROR".to_string(),
            llm_reasoning: "".to_string(),
            correct: false,
            error: Some(format!("Failed to execute: {}", e)),
        },
    }
}

fn calculate_metrics(
    results: &[TestResult],
) -> (f64, HashMap<Classification, ClassMetrics>) {
    let correct = results.iter().filter(|r| r.correct).count();
    let total = results.len();
    let accuracy = correct as f64 / total as f64;

    let mut per_class: HashMap<Classification, ClassMetrics> = HashMap::new();
    per_class.insert(Classification::Safe, ClassMetrics::default());
    per_class.insert(Classification::Unsafe, ClassMetrics::default());
    per_class.insert(Classification::Unknown, ClassMetrics::default());

    for result in results {
        if result.error.is_some() {
            continue;
        }

        let expected = Classification::from_str(&result.expected_class).unwrap();
        let predicted = Classification::from_str(&result.llm_class).unwrap();

        for (class, metrics) in per_class.iter_mut() {
            if expected == *class && predicted == *class {
                metrics.true_positives += 1;
            } else if expected != *class && predicted == *class {
                metrics.false_positives += 1;
            } else if expected == *class && predicted != *class {
                metrics.false_negatives += 1;
            } else {
                metrics.true_negatives += 1;
            }
        }
    }

    (accuracy, per_class)
}

fn write_markdown_report(
    path: &PathBuf,
    results: &[TestResult],
    accuracy: f64,
    per_class_metrics: &HashMap<Classification, ClassMetrics>,
) -> Result<()> {
    let mut f = File::create(path)?;

    writeln!(f, "# LLM Fallback Test Report")?;
    writeln!(f)?;
    writeln!(f, "**Date**: {}", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"))?;
    writeln!(f, "**Total Cases**: {}", results.len())?;
    writeln!(f, "**Correct**: {}", results.iter().filter(|r| r.correct).count())?;
    writeln!(f, "**Accuracy**: {:.1}%", accuracy * 100.0)?;
    writeln!(f, "**Errors**: {}", results.iter().filter(|r| r.error.is_some()).count())?;
    writeln!(f)?;

    // Per-class metrics
    writeln!(f, "## Per-Class Metrics")?;
    writeln!(f)?;
    writeln!(f, "| Class   | Precision | Recall | F1 Score | Support |")?;
    writeln!(f, "|---------|-----------|--------|----------|---------|")?;

    for class in &[Classification::Safe, Classification::Unsafe, Classification::Unknown] {
        let metrics = &per_class_metrics[class];
        let support = results.iter()
            .filter(|r| Classification::from_str(&r.expected_class).unwrap() == *class)
            .count();

        writeln!(
            f,
            "| {:7} | {:9.2} | {:6.2} | {:8.2} | {:7} |",
            class.as_str(),
            metrics.precision(),
            metrics.recall(),
            metrics.f1_score(),
            support
        )?;
    }
    writeln!(f)?;

    // Failed cases
    let failed: Vec<_> = results.iter().filter(|r| !r.correct && r.error.is_none()).collect();
    if !failed.is_empty() {
        writeln!(f, "## Failed Cases")?;
        writeln!(f)?;
        writeln!(f, "| ID  | Tool | Input | Expected | Got | Reasoning |")?;
        writeln!(f, "|-----|------|-------|----------|-----|-----------|")?;

        for result in failed {
            let input_short = if result.tool_input_value.len() > 50 {
                format!("{}...", &result.tool_input_value[..47])
            } else {
                result.tool_input_value.clone()
            };

            writeln!(
                f,
                "| {} | {} | `{}` | {} | {} | {} |",
                result.id,
                result.tool_name,
                input_short,
                result.expected_class,
                result.llm_class,
                result.llm_reasoning.replace("|", "\\|")
            )?;
        }
        writeln!(f)?;
    }

    // Errors
    let errors: Vec<_> = results.iter().filter(|r| r.error.is_some()).collect();
    if !errors.is_empty() {
        writeln!(f, "## Errors")?;
        writeln!(f)?;
        writeln!(f, "| ID  | Error |")?;
        writeln!(f, "|-----|-------|")?;

        for result in errors {
            writeln!(
                f,
                "| {} | {} |",
                result.id,
                result.error.as_ref().unwrap().replace("|", "\\|")
            )?;
        }
    }

    Ok(())
}

fn write_csv_results(path: &PathBuf, results: &[TestResult]) -> Result<()> {
    let mut wtr = csv::Writer::from_path(path)?;

    for result in results {
        wtr.serialize(result)?;
    }

    wtr.flush()?;
    Ok(())
}

fn print_summary(
    results: &[TestResult],
    accuracy: f64,
    per_class_metrics: &HashMap<Classification, ClassMetrics>,
) {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📈 Summary");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Total:     {}", results.len());
    println!("Correct:   {}", results.iter().filter(|r| r.correct).count());
    println!("Failed:    {}", results.iter().filter(|r| !r.correct && r.error.is_none()).count());
    println!("Errors:    {}", results.iter().filter(|r| r.error.is_some()).count());
    println!("Accuracy:  {:.1}%", accuracy * 100.0);
    println!();
    println!("Per-Class Metrics:");

    for class in &[Classification::Safe, Classification::Unsafe, Classification::Unknown] {
        let metrics = &per_class_metrics[class];
        println!(
            "  {:7} - P: {:.2}  R: {:.2}  F1: {:.2}",
            class.as_str(),
            metrics.precision(),
            metrics.recall(),
            metrics.f1_score()
        );
    }
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
}
