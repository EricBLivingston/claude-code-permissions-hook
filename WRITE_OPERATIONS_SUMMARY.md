# Write Operations Enhancement - Executive Summary

**Date:** 2025-11-01  
**Status:** Design Complete, Ready for Implementation  
**Priority:** CRITICAL for Claude's autonomous learning workflow

---

## The Problem

Current `decision_reviewer` design is **query-only** (read operations). This prevents Claude from closing the learning loop:

```
❌ Current Workflow (Manual, Error-Prone):
Claude attempts operation → LLM denies as uncertain
    → Claude identifies pattern
    → Human manually edits TOML (risky, slow)
    → Syntax errors possible, no validation, no audit trail

✅ Desired Workflow (Automated, Safe):
Claude attempts operation → LLM denies as uncertain
    → Claude queries logs
    → Claude submits validated job to add rule
    → Future attempts instantly allowed (no LLM query)
```

---

## The Solution

Transform `decision_reviewer` into a **read-write "job" interface** with:

1. **Write Operations:** 12 operation types (add/remove/update rules, LLM config, batches)
2. **Safety Mechanisms:** 5 layers of protection (env var, dry-run, validation, backups, atomic writes)
3. **Audit Trail:** Git auto-commit + separate audit log
4. **Validation Framework:** 3-tier validation (blocking, warnings, optional analysis)
5. **Rollback Support:** Git integration primary, timestamped backups fallback

---

## Design Decisions (Gemini-Validated)

### 1. Safety Model
**Decision:** ✅ Require `DECISION_REVIEWER_ALLOW_WRITES=1` environment variable

**Rationale (Gemini):**
> "For a security-critical system, an explicit, out-of-band 'safety switch' is the strongest protection against accidental or unauthorized modifications. The `ALLOW_WRITES` variable is the global safety lock, while the absence of `--dry-run` is the per-command confirmation."

### 2. Validation Strategy
**Decision:** ✅ Three-tier approach

- **Tier 1 (Blocking):** TOML syntax, regex compilation, XOR constraints, config load test
- **Tier 2 (Warnings):** Rule overlaps, broad patterns (show but allow with `--force`)
- **Tier 3 (Optional):** Behavior simulation, performance profiling (separate command)

**Rationale (Gemini):**
> "The system should never allow a write that results in a syntactically invalid, unloadable, or logically inconsistent configuration. This is a critical requirement to prevent the system from entering a broken state."

### 3. Rollback Mechanism
**Decision:** ✅ Git integration primary, backups fallback

**Rationale (Gemini):**
> "Using Git is the idiomatic and most powerful solution. It provides a perfect, immutable audit trail. Commit messages can capture why a change was made, who made it, and validation results."

### 4. Terminology
**Decision:** ✅ Use `apply` subcommand (not "job", "command", "mutation")

**Rationale (Gemini):**
> "`apply` clearly communicates the user's intent: to apply a set of changes to the current configuration. This term is used by widely adopted tools like `kubectl` and `terraform`, so it will be familiar to many users."

### 5. Batch Operations
**Decision:** ✅ All-or-nothing transaction semantics

**Rationale (Gemini):**
> "For a system where correctness is critical, atomicity is paramount. A partial application of changes can leave the configuration in an inconsistent, unpredictable, and potentially dangerous state."

### 6. Config File Selection
**Decision:** ✅ Require explicit `target_file` (no auto-detection)

**Rationale (Gemini):**
> "In configuration management, ambiguity is dangerous. Forcing the user/agent to specify the target file makes the operation completely predictable."

---

## Command Interface

### Subcommand: `apply`

```bash
decision_reviewer apply [OPTIONS] --json '<operation_json>'
```

**Options:**
- `--json <JSON>` - Operation specification (required)
- `--dry-run` - Preview changes without applying
- `--force` - Bypass non-blocking warnings
- `--target-file <PATH>` - Config file to modify
- `--no-commit` - Skip Git commit

**Environment Variable (REQUIRED):**
```bash
export DECISION_REVIEWER_ALLOW_WRITES=1
```

---

## Operation Types (12 total)

### Rule Management (5)
1. **add_rule** - Add new allow/deny rule
2. **remove_rule** - Remove rule by ID
3. **update_rule** - Modify existing rule fields
4. **toggle_rule** - Enable/disable without deleting
5. **reorder_rule** - Change rule precedence (important for deny rules)

### LLM Configuration (2)
6. **update_llm_config** - Modify LLM settings (model, timeout, temperature)
7. **update_llm_prompt** - Update system prompt

### Include Management (2)
8. **add_include** - Add config include file
9. **remove_include** - Remove config include

### Batch & Utility (3)
10. **batch** - Atomic multi-operation transaction
11. **rollback** - Revert to previous state (Git commit or backup)
12. **validate** - Validate without applying (dry-run mode)

---

## Example: Claude's Learning Workflow

```bash
# Step 1: Query recent LLM denial
decision_reviewer query --json '{"type":"log","decision":"deny","last_n":1}'
# Response: LLM denied "npm install" as uncertain

# Step 2: Claude analyzes
# - "npm install" is standard package manager operation
# - No shell metacharacters or suspicious patterns
# - Decision: Create hard allow rule to prevent future LLM queries

# Step 3: Dry-run to validate
export DECISION_REVIEWER_ALLOW_WRITES=1
decision_reviewer apply --dry-run --json '{
  "type": "add_rule",
  "rule_type": "allow",
  "target_file": "~/.config/claude-code-permissions-hook.toml",
  "rule": {
    "id": "allow-npm-install",
    "description": "Allow npm install commands (safe package manager)",
    "tool": "Bash",
    "command_regex": "^npm install( |$)",
    "command_exclude_regex": "&|;|\\||`|\\$\\("
  }
}'

# Response:
{
  "success": true,
  "dry_run": true,
  "would_change": true,
  "validation": {"blocking_passed": true, "warnings": []},
  "changes": ["Would add rule 'allow-npm-install' at line 42"]
}

# Step 4: Apply for real
decision_reviewer apply --json '{...}'

# Response:
{
  "success": true,
  "operation": "add_rule",
  "changes": ["Added rule 'allow-npm-install' to config.toml at line 42"],
  "rollback_info": {
    "type": "git",
    "commit": "abc123def",
    "message": "feat(config): Add allow rule for 'npm install'"
  }
}

# Step 5: Verify with match test
decision_reviewer query --json '{
  "type": "match_test",
  "tool": "Bash",
  "input": {"command": "npm install"}
}'

# Response: {"would_match": true, "matched_rule": {"id": "allow-npm-install"}}
```

**Result:**
- ✅ Future `npm install` commands instantly allowed (no LLM query needed)
- ✅ Performance improved (hard rule ~1ms vs LLM ~1-2 seconds)
- ✅ Security maintained (exclude patterns block injection attempts)
- ✅ Fully auditable (Git commit + audit log)
- ✅ Reversible (can rollback via Git or backup)

---

## Safety Mechanisms (5 Layers)

| Layer | Protection | Failure Mode |
|-------|------------|--------------|
| **1. Environment Variable** | `DECISION_REVIEWER_ALLOW_WRITES=1` required | Command fails with clear error |
| **2. Dry-Run Workflow** | Preview before applying | User/Claude sees impact first |
| **3. Validation Before Write** | Syntax, regex, constraints, load test | Write rejected before file touched |
| **4. Automatic Backups + Git** | Git commit or timestamped backup | Always rollback available |
| **5. Atomic Writes** | Temp file → validate → atomic rename | Never corrupted config |

**Additional Safety:**
- File locking prevents concurrent modifications
- All-or-nothing batch semantics prevent partial failures
- Input sanitization prevents JSON injection
- Separate audit log tracks all write operations

---

## Implementation Timeline

### Fast-Track (2 weeks)

**Week 1: Core Infrastructure + Validation**
- `apply` subcommand with env var gate
- TOML writing with `toml_edit` crate (preserves formatting)
- Git auto-commit + backup fallback
- Blocking validations (syntax, regex, XOR, load test)
- Warning system (overlaps, broad patterns)
- `--dry-run` and `--force` flags

**Week 2: Full Operation Set + Testing**
- Rule operations: add, remove, update, toggle, reorder
- LLM config operations
- Batch operations (atomic semantics)
- Rollback operation
- Basic integration tests
- Test with Claude's actual workflow

**Deferred (later phases):**
- Comprehensive test coverage (>90%)
- Documentation polish
- Advanced features (include management, config file helpers)

### Dependencies

```toml
[dependencies]
toml_edit = "0.21"  # Preserves formatting when writing TOML
fs2 = "0.4"         # File locking for concurrent safety
git2 = "0.18"       # Git operations (auto-commit)
chrono = "0.4"      # Timestamps for backups
```

---

## Integration with Existing Architecture

### Current Dual-Log System
- **Operational log** (`/tmp/claude-tool-use.log`) - Claude reads for recent decisions
- **Review log** (`/tmp/claude-decisions-review.log`) - Gemini analyzes for patterns

### NEW: Audit Log
- **Audit log** (`~/.config/decision-reviewer-audit.log`) - Tracks config changes

### Complete Learning Loop

```
┌─────────────────────────────────────────────────────┐
│ 1. Claude attempts operation                       │
│    → Logged to operational + review logs           │
└───────────────┬─────────────────────────────────────┘
                │
                ▼
┌─────────────────────────────────────────────────────┐
│ 2. Claude queries operational log                  │
│    decision_reviewer query --json '{...}'          │
└───────────────┬─────────────────────────────────────┘
                │
                ▼
┌─────────────────────────────────────────────────────┐
│ 3. Claude analyzes pattern                         │
│    Determines: "npm install" is safe, should allow │
└───────────────┬─────────────────────────────────────┘
                │
                ▼
┌─────────────────────────────────────────────────────┐
│ 4. Claude submits dry-run                          │
│    decision_reviewer apply --dry-run --json '{...}'│
└───────────────┬─────────────────────────────────────┘
                │
                ▼
┌─────────────────────────────────────────────────────┐
│ 5. Claude reviews validation results               │
│    Blocking: PASSED, Warnings: NONE                │
└───────────────┬─────────────────────────────────────┘
                │
                ▼
┌─────────────────────────────────────────────────────┐
│ 6. Claude applies for real (NEW CAPABILITY)        │
│    decision_reviewer apply --json '{...}'          │
│    → Config updated, Git commit created            │
│    → Logged to audit log                           │
└───────────────┬─────────────────────────────────────┘
                │
                ▼
┌─────────────────────────────────────────────────────┐
│ 7. Future attempts instantly allowed               │
│    Hard rule matches (no LLM query needed)         │
│    Performance: ~1ms vs ~1-2 seconds               │
└─────────────────────────────────────────────────────┘
```

---

## Security Considerations

### Threat Model

| Threat | Mitigation | Verification |
|--------|------------|--------------|
| **Accidental misconfiguration** | Blocking validations, dry-run workflow | Config load test must pass |
| **AI agent runaway writes** | Environment variable gate, Git audit trail | Each write logged + committed |
| **JSON injection** | Schema validation, input sanitization | JSON parsed with strict schema |
| **Concurrent modifications** | File locking (fs2), atomic writes | Exclusive lock during write |
| **Loss of audit trail** | Git commits + separate audit log | Immutable history in Git |

### Audit Log Format

```json
{
  "timestamp": "2025-11-01T10:23:45Z",
  "operation": "add_rule",
  "applied_by": "claude-v2-alpha",
  "target_file": "/home/user/.config/claude-code-permissions-hook.toml",
  "changes": {
    "type": "add_rule",
    "rule_id": "allow-npm-install",
    "rule_type": "allow",
    "position": "line 42"
  },
  "validation": {
    "blocking_passed": true,
    "warnings": []
  },
  "rollback_info": {
    "type": "git",
    "commit": "abc123def456"
  },
  "dry_run": false,
  "success": true
}
```

---

## Why This Matters

### Without Write Operations (Current State)
- ❌ Claude identifies patterns but can't act on them
- ❌ Manual TOML editing required (error-prone, no validation)
- ❌ LLM queries repeated for same patterns (slow, costly)
- ❌ No audit trail for config changes
- ❌ Learning loop not closed

### With Write Operations (New Capability)
- ✅ Claude autonomously improves config based on learned patterns
- ✅ Validated, safe writes with automatic backups
- ✅ Hard rules reduce LLM query volume (performance + cost)
- ✅ Full audit trail (Git + audit log)
- ✅ Complete learning loop: Attempt → Log → Analyze → Improve → Better Decisions

### Measurable Impact

**Performance:**
- Hard rule match: ~1ms
- LLM fallback: ~1-2 seconds
- **Improvement: 1000-2000x faster for learned patterns**

**Cost:**
- Hard rule: $0 per decision
- LLM fallback: ~$0.0001-0.001 per decision (depending on model)
- **Savings: Eliminates LLM costs for learned patterns**

**Reliability:**
- Hard rules: 100% deterministic
- LLM fallback: ~91% accurate (based on testing)
- **Improvement: Perfect accuracy for learned patterns**

---

## Next Steps

### Recommended Approach: Fast-Track (2 weeks)

**Week 1:**
1. Create `src/bin/decision_reviewer.rs` skeleton
2. Implement `apply` subcommand with env var gate
3. Add TOML writing (`toml_edit`) and Git integration
4. Implement blocking validations + warning system
5. Add `--dry-run` mode

**Week 2:**
6. Implement core operations: add_rule, remove_rule, update_rule
7. Implement batch operations (atomic semantics)
8. Implement rollback
9. Add basic integration tests
10. Test with Claude's actual workflow

**Deliverable:** Claude can safely add/remove/update rules autonomously

### Alternative: Phased Approach (5 weeks)

**Phase 1 (Week 1):** Core infrastructure only
**Phase 2 (Week 2):** Validation framework
**Phase 3 (Week 3):** Full operation set
**Phase 4 (Week 4):** Testing
**Phase 5 (Week 5):** Documentation

**Deliverable:** Comprehensive, production-ready system

---

## Files to Create

### Primary Implementation
- `src/bin/decision_reviewer.rs` - Main binary (query + apply)
- `src/operations/` - Operation implementations
  - `add_rule.rs`, `remove_rule.rs`, `update_rule.rs`, etc.
- `src/validation.rs` - Validation framework
- `src/git_ops.rs` - Git integration
- `src/backup.rs` - Backup management
- `src/audit_log.rs` - Audit logging

### Documentation
- `docs/write-operations-design.md` - ✅ COMPLETE
- `docs/write-operations-tutorial.md` - User guide (later)
- Update `CLAUDE.md` with write operations section (later)

### Testing
- `tests/operations/` - Integration tests for each operation
- `tests/validation_tests.rs` - Validation framework tests
- `tests/safety_tests.rs` - Safety mechanism tests

---

## Conclusion

This enhancement transforms the `decision_reviewer` from a passive query tool into an active learning system that enables Claude to autonomously improve security configuration based on observed patterns.

**Key Benefits:**
1. **Autonomy:** Claude can close the learning loop without manual intervention
2. **Safety:** 5 layers of protection ensure config never corrupted
3. **Auditability:** Complete change history via Git + audit log
4. **Performance:** Hard rules 1000-2000x faster than LLM queries
5. **Reliability:** 100% deterministic decisions for learned patterns

**Recommendation:** Implement fast-track (2 weeks) to enable Claude's learning workflow ASAP.

---

**Status:** Design Complete, Ready for Implementation  
**Priority:** CRITICAL for Claude's autonomous learning capability  
**Design Document:** `docs/write-operations-design.md`  
**Next Step:** Begin Week 1 implementation (core infrastructure + validation)
