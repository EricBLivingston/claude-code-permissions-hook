# Sectioned Configuration Implementation Plan

**Status**: Ready to implement
**Timeline**: 1.5-2 days (12-16 hours)
**Approach**: Implement core → Test → Migrate configs → Validate

## Confirmed Design Decisions

1. **Section Priority**: Lower number = higher priority (0 checked first)
2. **Rule IDs**: REQUIRED - error if missing
3. **Section Naming**: Enforce kebab-case only
4. **Reserved Names**: Reserve `logging`, `llm_fallback`, `includes` only
5. **Backward Compatibility**: NONE - old format configs will error (clean break)

## New Config Structure

```toml
[logging]
log_file = "/tmp/claude-tool-use.log"
review_log_file = "/tmp/claude-decisions-review.log"
log_level = "debug"

[llm_fallback]
enabled = true
endpoint = "https://openrouter.ai/api/v1"
model = "anthropic/claude-haiku-4.5"
# ... rest of LLM config

[build-tools]
description = "Build and package management tools"
priority = 10  # Lower = higher priority

[[build-tools.allow]]
id = "allow-cargo"  # REQUIRED, must be unique
description = "Allow safe cargo commands"
tool = "Bash"
command_regex = "^cargo (build|test|check|clippy|fmt|run)"
command_exclude_regex = "&|;|\\||`|\\$\\("

[[build-tools.deny]]
id = "deny-cargo-install-external"
# ... deny rules

[file-operations]
priority = 20

[[file-operations.allow]]
# ...

[other]
description = "Uncategorized rules pending organization"
priority = 100  # Checked last
```

## Implementation Phases

### Phase 1: Core Implementation (6-8 hours)

#### Step 1: Config Structure (2 hours)
**File**: `src/config.rs`

**New Structures**:
```rust
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

fn default_priority() -> u32 { 50 }
fn default_enabled() -> bool { true }

#[derive(Debug, Deserialize)]
pub struct RuleConfig {
    pub id: String,  // REQUIRED
    pub description: Option<String>,
    // ... rest of fields same as before
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub id: String,
    pub section_name: String,  // NEW
    pub description: Option<String>,
    // ... compiled regexes
}

pub struct CompiledConfig {
    pub logging: LoggingConfig,
    pub llm_fallback: LlmFallbackConfig,
    pub deny_rules: Vec<Rule>,
    pub allow_rules: Vec<Rule>,
}
```

**Key Functions**:
```rust
impl Config {
    pub fn load_from_file(path: &Path) -> Result<CompiledConfig> {
        let config = Self::load_with_includes(path)?;
        config.validate()?;
        config.compile()
    }

    fn validate(&self) -> Result<()> {
        // Check reserved names
        // Validate kebab-case
        // Validate rule ID uniqueness
    }

    fn compile(self) -> Result<CompiledConfig> {
        // Sort sections by priority
        // Filter disabled sections
        // Compile rules in order
    }
}

fn compile_rule(config: RuleConfig, section_name: &str) -> Result<Rule> {
    // XOR validation
    // Compile regexes
}
```

**Testing Checkpoints**:
- [ ] Deserializes new format
- [ ] Validates reserved names
- [ ] Validates kebab-case
- [ ] Validates rule ID uniqueness
- [ ] Compiles in priority order
- [ ] Filters disabled sections

#### Step 2: Matcher Integration (1 hour)
**File**: `src/matcher.rs`

**Changes**:
```rust
#[derive(Debug, Clone)]
pub struct DecisionInfo {
    pub decision: Decision,
    pub rule_id: String,      // NEW
    pub section_name: String, // NEW
}

pub fn check_rules(rules: &[Rule], input: &HookInput) -> Option<DecisionInfo> {
    for rule in rules {
        if check_rule(rule, input) {
            return Some(DecisionInfo {
                decision: Decision::Allow,
                rule_id: rule.id.clone(),
                section_name: rule.section_name.clone(),
            });
        }
    }
    None
}
```

**Testing Checkpoints**:
- [ ] Returns section name
- [ ] Returns rule ID
- [ ] Matching logic unchanged

#### Step 3: Logging Integration (1 hour)
**File**: `src/logging.rs`

**Changes**:
```rust
#[derive(Serialize)]
pub struct RuleMetadata {
    pub rule_id: String,      // NEW
    pub section_name: String, // NEW
    pub rule_type: String,
}
```

**Testing Checkpoints**:
- [ ] Logs include section_name
- [ ] Logs include rule_id

#### Step 4: Main Integration (1 hour)
**File**: `src/main.rs`

**Changes**:
```rust
fn run(config_path: &Path, test_mode: bool) -> Result<()> {
    let compiled = Config::load_from_file(config_path)?;
    let input = HookInput::read_from_stdin()?;

    if let Some(decision_info) = check_rules(&compiled.deny_rules, &input) {
        log_rule_decision(&compiled.logging.log_file, &input, &decision_info, "deny");
        output_decision(Decision::Deny)?;
        return Ok(());
    }

    if let Some(decision_info) = check_rules(&compiled.allow_rules, &input) {
        log_rule_decision(&compiled.logging.log_file, &input, &decision_info, "allow");
        output_decision(Decision::Allow)?;
        return Ok(());
    }

    // LLM fallback unchanged
    Ok(())
}
```

### Phase 2: Migration Tool (3-4 hours)

#### Step 5: Build Migration Tool (2.5 hours)
**File**: `src/bin/migrate_config.rs`

**Section Inference Logic**:
```
^cargo|^npm|^pip → "build-tools" (priority 10)
^git\s+ → "git-commands" (priority 15)
^docker|^kubectl → "infrastructure" (priority 20)
^(ps|top|htop) → "system-monitoring" (priority 25)
^(ls|cat|head) → "file-viewing" (priority 30)
^(grep|awk|sed) → "text-processing" (priority 35)
^(ping|curl|netstat) → "networking" (priority 40)
tool=Read|Write|Edit → "file-operations" (priority 20)
tool_regex=^mcp__ → "mcp-tools" (priority 25)
^rm.*-rf|\.env|\.secret → "security-critical" (priority 5)
Everything else → "other" (priority 100)
```

**Rule ID Generation**:
- Format: `{action}-{description-summary}` or `{action}-{tool}-{pattern}`
- Ensure uniqueness with numeric suffixes

**Testing Checkpoints**:
- [ ] Infers sections correctly
- [ ] Generates unique IDs
- [ ] Preserves all rules
- [ ] Output is valid TOML

#### Step 6: Test Migration (1 hour)

Create unit and integration tests for migration tool.

### Phase 3: Testing & Validation (2-3 hours)

#### Step 7: Behavioral Equivalence Tests (2 hours)
**File**: `tests/equivalence_test.rs`

Test that new config produces identical decisions to old config for all test cases.

#### Step 8: Config Migration Validation (1 hour)

**Procedure**:
```bash
# Backup
cp ~/.config/claude-code-permissions-hook.toml \
   ~/.config/claude-code-permissions-hook.toml.backup

# Migrate
cargo run --bin migrate_config -- \
  --input ~/.config/claude-code-permissions-hook.toml.backup \
  --output ~/.config/claude-code-permissions-hook.toml.new

# Validate
cargo run -- validate --config ~/.config/claude-code-permissions-hook.toml.new

# Test equivalence
for test in tests/*.json; do
  cat "$test" | cargo run -- run --config old.toml > /tmp/old.out
  cat "$test" | cargo run -- run --config new.toml > /tmp/new.out
  diff /tmp/old.out /tmp/new.out
done

# Deploy if all pass
mv ~/.config/claude-code-permissions-hook.toml.new \
   ~/.config/claude-code-permissions-hook.toml
```

### Phase 4: Documentation (1 hour)

Update CLAUDE.md, example.toml, test-llm-config.toml

## Timeline Breakdown

**Day 1 (8 hours)**:
- Hour 1-2: Config structures and validation
- Hour 3: Compilation logic
- Hour 4: Matcher integration
- Hour 5: Logging integration
- Hour 6: Main integration + basic testing
- Hour 7-8: Migration tool core logic

**Day 2 (6-8 hours)**:
- Hour 1: Migration tool testing
- Hour 2: Behavioral equivalence tests
- Hour 3: Run full tests, fix issues
- Hour 4: Migrate production config
- Hour 5: LLM test suite validation
- Hour 6: Documentation
- Hour 7-8: Buffer for issues

## Files to Modify

1. `src/config.rs` - New structures, validation, compilation
2. `src/matcher.rs` - Section context in DecisionInfo
3. `src/logging.rs` - Section metadata
4. `src/main.rs` - Use compiled config
5. `src/bin/migrate_config.rs` - NEW
6. `example.toml` - Migrate
7. `test-llm-config.toml` - Migrate
8. `tests/equivalence_test.rs` - NEW
9. `CLAUDE.md` - Document sections

## Pre-Deployment Checklist

- [ ] All unit tests pass
- [ ] Behavioral equivalence tests pass
- [ ] Production config migrates without errors
- [ ] Validation passes on new config
- [ ] Sample inputs produce identical outputs
- [ ] LLM test suite passes at same rate
- [ ] Logs include section and rule_id
- [ ] Documentation updated
- [ ] Backup of old config saved

## Post-Deployment Monitoring

```bash
# Monitor logs for section metadata
tail -f /tmp/claude-tool-use.log | jq '.section_name'

# Verify no errors
tail -100 /tmp/claude-tool-use.log | jq 'select(.error != null)'
```

## Success Criteria

- Section names appear in 100% of rule logs
- Rule IDs appear in 100% of rule logs
- Allow/deny rate unchanged (±5%)
- LLM fallback rate unchanged (±5%)
- No hook execution errors

## Rollback Procedure

```bash
# Restore backup
cp ~/.config/claude-code-permissions-hook.toml.backup \
   ~/.config/claude-code-permissions-hook.toml
```

## Next Steps (Week 2)

After successful deployment:
- Build decision_reviewer with section-aware queries
- Implement write operations (add/remove/update rules)
- Create analysis and reporting features
