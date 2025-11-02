# Decision Reviewer: Write Operations Design

**Status:** Design Phase  
**Author:** System Design (based on Gemini recommendations)  
**Date:** 2025-11-01  
**Version:** 1.0

---

## Executive Summary

This document specifies the design for write operations in the `decision_reviewer` tool, enabling both Claude AI and human users to safely modify the security configuration based on learned patterns from decision logs.

**Key Design Principles:**
- **Safety First:** Explicit permission gates, validation, and rollback capabilities
- **Atomic Operations:** All-or-nothing transactions prevent partial failures
- **Audit Trail:** Git integration for full change history
- **Explicit Over Implicit:** Require explicit target file specification
- **Fast Validation:** < 1 second for typical operations

---

## Table of Contents

1. [Command Interface](#command-interface)
2. [Safety Model](#safety-model)
3. [Validation Strategy](#validation-strategy)
4. [Rollback Mechanism](#rollback-mechanism)
5. [Operation Types](#operation-types)
6. [Response Format](#response-format)
7. [Implementation Plan](#implementation-plan)
8. [Timeline](#timeline)
9. [Security Considerations](#security-considerations)

---

## Command Interface

### Subcommand: `apply`

**Rationale:** Clear intent, industry precedent (kubectl, terraform), avoids ambiguity.

```bash
decision_reviewer apply [OPTIONS] --json '<operation_json>'
```

**Options:**
- `--json <JSON>` - Operation specification (required)
- `--dry-run` - Preview changes without applying
- `--force` - Bypass non-blocking warnings
- `--target-file <PATH>` - Config file to modify (required for multi-file setups)
- `--no-commit` - Skip Git commit (use manual commit workflow)

**Environment Variable:**
- `DECISION_REVIEWER_ALLOW_WRITES=1` - REQUIRED for all write operations

### Examples

```bash
# Dry-run to preview changes
export DECISION_REVIEWER_ALLOW_WRITES=1
decision_reviewer apply --dry-run --json '{
  "type": "add_rule",
  "rule_type": "allow",
  "target_file": "~/.config/claude-code-permissions-hook.toml",
  "rule": {
    "id": "allow-npm-install",
    "description": "Allow npm install commands",
    "tool": "Bash",
    "command_regex": "^npm install",
    "command_exclude_regex": "&|;|\\||`|\\$\\("
  }
}'

# Apply for real
decision_reviewer apply --json '{ ... }'

# Batch operations
decision_reviewer apply --json '{
  "type": "batch",
  "operations": [
    {"type": "add_rule", ...},
    {"type": "update_llm_config", ...}
  ]
}'

# Rollback to previous state
decision_reviewer apply --json '{
  "type": "rollback",
  "to_commit": "abc123def"
}'
```

---

## Safety Model

### Multi-Layer Protection

**Layer 1: Environment Variable Gate**

```bash
# REQUIRED - commands fail without this
export DECISION_REVIEWER_ALLOW_WRITES=1
```

**Rationale:**
- Explicit, deliberate act for both humans and AI agents
- Prevents accidental writes during read-only analysis
- Can be scoped to specific shell sessions
- Programmatic guardrail for AI agent execution environments

**Error Message (without env var):**
```
ERROR: Write operations disabled. Set DECISION_REVIEWER_ALLOW_WRITES=1 to enable.
       This is a safety mechanism to prevent accidental configuration changes.
```

**Layer 2: Dry-Run First Workflow**

Recommended workflow (especially for AI agents):

1. Query recent decisions: `decision_reviewer query --json '{"type":"log","decision":"deny","last_n":1}'`
2. Analyze and determine needed changes
3. **Dry-run:** `decision_reviewer apply --dry-run --json '{...}'`
4. Review validation results and impact analysis
5. **Apply:** `decision_reviewer apply --json '{...}'` (explicit confirmation)

**Layer 3: Validation Before Write**

All changes validated before file modification:
- Syntax validation (TOML parsing)
- Regex compilation
- Constraint checking (XOR, required fields)
- Full config load test
- Conflict detection

**Layer 4: Automatic Backups + Git**

- **Primary:** Git commit with descriptive message
- **Fallback:** Timestamped backup if not in Git repo
- Never lose ability to rollback

**Layer 5: Atomic Writes**

- Write to temporary file first
- Validate temporary file
- Atomic rename (POSIX guarantee)
- Never leave corrupted config

### Why This Model?

**From Gemini Analysis:**
> "For a security-critical system, an explicit, out-of-band 'safety switch' is the strongest protection against accidental or unauthorized modifications. The `ALLOW_WRITES` variable is the global safety lock, while the absence of `--dry-run` is the per-command confirmation. This two-factor approach provides a robust balance of safety and convenience."

**Trade-offs Considered:**
- ❌ Filesystem permissions only → No protection against user error
- ❌ Selective env var (only for risky ops) → Complex mental model, error-prone
- ✅ Universal env var + dry-run workflow → Clear, predictable, safe

---

## Validation Strategy

### Three-Tier Validation

**Tier 1: Blocking (Must Pass)**

Operations FAIL if these validations fail:

| Validation | Description | Example Error |
|------------|-------------|---------------|
| **TOML Syntax** | Valid TOML structure | `Expected '=' after key at line 42` |
| **Regex Compilation** | All patterns compile | `command_regex invalid: unmatched parenthesis` |
| **Required Fields** | All mandatory fields present | `Missing required field: tool or tool_regex` |
| **XOR Constraints** | Exactly one of `tool` or `tool_regex` | `Rule must specify exactly one of 'tool' or 'tool_regex', not both` |
| **Config Load Test** | `Config::load_from_file()` succeeds | `Failed to load config: circular include detected` |

**Rationale (from Gemini):**
> "The system should never allow a write that results in a syntactically invalid, unloadable, or logically inconsistent configuration. This is a critical requirement to prevent the system from entering a broken state."

**Tier 2: Warnings (Show But Allow)**

Operations SUCCEED but display warnings:

| Warning | Description | Override Flag |
|---------|-------------|---------------|
| **Rule Overlap** | Two rules partially cover same tools | `--force` |
| **Broad Pattern** | Regex matches too many cases | `--force` |
| **Performance Impact** | Complex regex with high cost | `--force` |

**Example:**
```json
{
  "success": true,
  "warnings": [
    {
      "type": "rule_overlap",
      "message": "New rule may overlap with existing rule 'allow-cargo-safe'",
      "suggestion": "Consider narrowing the command_regex pattern"
    }
  ]
}
```

**Tier 3: Optional Analysis (Separate Command)**

Expensive validation NOT part of default write path:

```bash
# Behavior simulation (test rule against sample inputs)
decision_reviewer test --rule-id allow-npm-install --input '{"command":"npm install"}'

# Performance profiling
decision_reviewer profile --rule-id complex-regex-rule

# Semantic analysis (experimental)
decision_reviewer analyze-rule --rule-id suspicious-allow
```

**Rationale:**
- Keeps write operations fast (< 1 second target)
- Allows deep analysis when needed
- Prevents blocking on slow operations

---

## Rollback Mechanism

### Primary: Git Integration

**Auto-commit on every write:**

```bash
# After successful write
git add ~/.config/claude-code-permissions-hook.toml
git commit -m "feat(config): Add allow rule for 'npm install'

Operation: add_rule
Rule ID: allow-npm-install
Applied by: claude-v2-alpha
Timestamp: 2025-11-01T10:23:45Z
Validation: Passed (0 warnings)

🤖 Generated with decision_reviewer"
```

**Standard Git Workflow:**
```bash
# View history
git log --oneline ~/.config/claude-code-permissions-hook.toml

# Review changes
git show HEAD

# Rollback to previous commit
git revert abc123

# Rollback to specific version
decision_reviewer apply --json '{
  "type": "rollback",
  "to_commit": "abc123def"
}'
```

### Fallback: Timestamped Backups

**When Git unavailable:**

```bash
# Before write
cp config.toml config.toml.20251101T102345Z.bak

# Keep last 10 backups (auto-cleanup)
ls -t *.bak | tail -n +11 | xargs rm
```

**Rationale (from Gemini):**
> "Using Git is the idiomatic and most powerful solution. It provides a perfect, immutable audit trail. Commit messages can capture why a change was made, who made it, and validation results. This is invaluable for human review."

---

## Operation Types

### 1. Rule Management

#### Add Rule

```json
{
  "type": "add_rule",
  "rule_type": "allow",
  "target_file": "~/.config/claude-code-permissions-hook.toml",
  "position": "end",
  "rule": {
    "id": "allow-npm-install",
    "description": "Allow npm install commands (safe package manager)",
    "tool": "Bash",
    "command_regex": "^npm install",
    "command_exclude_regex": "&|;|\\||`|\\$\\("
  }
}
```

#### Remove Rule

```json
{
  "type": "remove_rule",
  "id": "allow-old-pattern",
  "target_file": "~/.config/claude-code-permissions-hook.toml"
}
```

#### Update Rule

```json
{
  "type": "update_rule",
  "id": "allow-safe-cargo",
  "target_file": "~/.config/claude-code-permissions-hook.toml",
  "changes": {
    "description": "Allow safe cargo commands (build, test, check, clippy, fmt, run, doc)",
    "command_regex": "^cargo (build|test|check|clippy|fmt|run|doc)"
  }
}
```

#### Toggle Rule (Enable/Disable)

```json
{
  "type": "toggle_rule",
  "id": "allow-experimental-feature",
  "enabled": false,
  "target_file": "~/.config/claude-code-permissions-hook.toml"
}
```

#### Reorder Rule

```json
{
  "type": "reorder_rule",
  "id": "deny-critical-security",
  "move_to": "start",
  "target_file": "~/.config/claude-code-permissions-hook.toml"
}
```

### 2. LLM Configuration

```json
{
  "type": "update_llm_config",
  "target_file": "~/.config/llm-fallback-config.toml",
  "changes": {
    "enabled": true,
    "model": "anthropic/claude-haiku-4.5",
    "timeout_secs": 60,
    "temperature": 0.2
  }
}
```

### 3. Batch Operations

```json
{
  "type": "batch",
  "target_file": "~/.config/claude-code-permissions-hook.toml",
  "operations": [
    {"type": "add_rule", "rule_type": "allow", "rule": {...}},
    {"type": "remove_rule", "id": "allow-old-pattern"},
    {"type": "update_llm_config", "changes": {...}}
  ]
}
```

**All-or-Nothing Semantics:**
- Parse all operations
- Apply all to in-memory model
- Validate entire resulting config
- If ANY validation fails → abort entire batch
- If all pass → write entire result atomically

**Rationale (from Gemini):**
> "For a system where correctness is critical, atomicity is paramount. A partial application of changes can leave the configuration in an inconsistent, unpredictable, and potentially dangerous state."

---

## Response Format

### Success Response

```json
{
  "success": true,
  "operation": "add_rule",
  "changes": [
    "Added rule 'allow-npm-install' to /home/user/.config/claude-code-permissions-hook.toml at line 42"
  ],
  "rollback_info": {
    "type": "git",
    "commit": "abc123def456",
    "message": "feat(config): Add allow rule for 'npm install'",
    "rollback_command": "decision_reviewer apply --json '{\"type\":\"rollback\",\"to_commit\":\"abc123def456\"}'"
  },
  "validation": {
    "blocking_passed": true,
    "warnings": []
  },
  "dry_run": false
}
```

### Error Response

```json
{
  "success": false,
  "operation": "add_rule",
  "error": "Validation failed",
  "error_type": "blocking_validation",
  "details": [
    {
      "field": "rule.command_regex",
      "error": "Regex compilation failed: unmatched parenthesis at position 15",
      "suggestion": "Check regex syntax. Expected closing ')' for group."
    }
  ],
  "changes_applied": "none (validation failed before write)"
}
```

### Dry-Run Response

```json
{
  "success": true,
  "dry_run": true,
  "operation": "add_rule",
  "would_change": true,
  "target_file": "/home/user/.config/claude-code-permissions-hook.toml",
  "changes": [
    "Would add rule 'allow-npm-install' at line 42"
  ],
  "validation": {
    "blocking_passed": true,
    "warnings": []
  },
  "impact_analysis": {
    "rules_affected": 0,
    "would_change_behavior": false
  },
  "next_step": "Run without --dry-run to apply changes"
}
```

---

## Implementation Plan

### Phase 1: Core Infrastructure (Week 1)

**Goal:** Basic write operations with safety mechanisms

**Tasks:**
1. Create `src/bin/decision_reviewer.rs` skeleton
2. Implement `apply` subcommand CLI parsing
3. Add environment variable check (`DECISION_REVIEWER_ALLOW_WRITES`)
4. Implement TOML writing with `toml_edit` crate
5. Add Git detection and commit logic
6. Add backup creation/cleanup logic
7. Implement atomic write (temp file + rename)

**Dependencies:**
```toml
[dependencies]
toml_edit = "0.21"  # Preserves formatting
fs2 = "0.4"         # File locking
git2 = "0.18"       # Git operations
chrono = "0.4"      # Timestamps for backups
```

**Deliverables:**
- Basic `add_rule` operation working
- Git auto-commit functional
- Backup fallback working
- All safety layers in place

### Phase 2: Validation Framework (Week 2)

**Goal:** Comprehensive validation before writes

**Tasks:**
1. Implement blocking validations (TOML, regex, XOR, required fields, config load)
2. Implement warning system (overlap detection, broad patterns)
3. Add `--dry-run` mode
4. Add `--force` flag for bypassing warnings

**Deliverables:**
- All Tier 1 validations working
- Warning system functional
- Dry-run provides accurate preview

### Phase 3: Full Operation Set (Week 3)

**Goal:** All operation types implemented

**Tasks:**
1. Implement rule operations (remove, update, toggle, reorder)
2. Implement LLM config operations
3. Implement include operations
4. Implement batch operations with all-or-nothing semantics
5. Implement rollback operation

**Deliverables:**
- All operation types working
- Batch operations atomic
- Rollback functional

### Phase 4: Integration & Testing (Week 4)

**Goal:** Production-ready quality

**Tasks:**
1. Write comprehensive unit tests
2. Write integration tests
3. Add error recovery tests
4. Performance testing (< 1 second target)

**Deliverables:**
- >90% test coverage
- All edge cases handled
- Performance targets met

### Phase 5: Documentation (Week 5)

**Goal:** User-ready documentation

**Tasks:**
1. Update `CLAUDE.md` with write operations section
2. Create tutorial documentation
3. Add examples for each operation type
4. Create workflow documentation for Claude AI agent usage

**Deliverables:**
- Complete documentation
- Example workflows
- Clear error messages

---

## Timeline

**Total Duration:** 5 weeks (assuming 1 developer, part-time)

| Phase | Duration | Priority |
|-------|----------|----------|
| Phase 1: Core Infrastructure | Week 1 | CRITICAL |
| Phase 2: Validation Framework | Week 2 | HIGH |
| Phase 3: Full Operation Set | Week 3 | HIGH |
| Phase 4: Integration & Testing | Week 4 | HIGH |
| Phase 5: Documentation | Week 5 | MEDIUM |

**Fast-Track Option (2 weeks):**
- Week 1: Phases 1 + 2 (core + validation)
- Week 2: Phase 3 (operations) + essential testing
- Defer: Comprehensive testing, documentation polish

---

## Security Considerations

### Threat Model

| Threat | Mitigation |
|--------|------------|
| Accidental misconfiguration | Blocking validations, dry-run workflow |
| AI agent runaway writes | Environment variable gate, Git audit trail |
| JSON injection | Schema validation, input sanitization |
| Concurrent modifications | File locking (fs2), atomic writes |
| Loss of audit trail | Git commits with detailed messages |

### Audit Logging

**File:** `~/.config/decision-reviewer-audit.log`

**Format:**
```json
{
  "timestamp": "2025-11-01T10:23:45Z",
  "operation": "add_rule",
  "applied_by": "claude-v2-alpha",
  "target_file": "/home/user/.config/claude-code-permissions-hook.toml",
  "changes": {...},
  "validation": {...},
  "rollback_info": {...},
  "success": true
}
```

---

## Appendix: Claude AI Agent Workflow Example

**Scenario:** Claude attempts `npm install` → LLM denies as uncertain → Claude learns and adds hard rule

```bash
# Step 1: Query recent denial
decision_reviewer query --json '{"type":"log","decision":"deny","last_n":1}'

# Step 2: Claude analyzes
# - "npm install" is standard package manager operation
# - No shell metacharacters or suspicious patterns
# - Decision: Create hard allow rule

# Step 3: Dry-run to validate
export DECISION_REVIEWER_ALLOW_WRITES=1
decision_reviewer apply --dry-run --json '{
  "type": "add_rule",
  "rule_type": "allow",
  "target_file": "~/.config/claude-code-permissions-hook.toml",
  "rule": {
    "id": "allow-npm-install",
    "description": "Allow npm install commands",
    "tool": "Bash",
    "command_regex": "^npm install( |$)",
    "command_exclude_regex": "&|;|\\||`|\\$\\("
  }
}'

# Step 4: Apply for real
decision_reviewer apply --json '{...}'

# Step 5: Verify with match test
decision_reviewer query --json '{
  "type": "match_test",
  "tool": "Bash",
  "input": {"command": "npm install"}
}'

# Result: Next time Claude tries "npm install", it will be instantly allowed
# by hard rule (no LLM query needed)
```

**Result:**
- ✅ Security maintained (exclude patterns block injection)
- ✅ Performance improved (hard rule faster than LLM)
- ✅ Audit trail preserved (Git commit)
- ✅ Reversible (can rollback if needed)
- ✅ Claude learned and adapted configuration autonomously

---

**End of Document**

**Version:** 1.0  
**Last Updated:** 2025-11-01  
**Status:** Design Phase - Ready for Implementation
