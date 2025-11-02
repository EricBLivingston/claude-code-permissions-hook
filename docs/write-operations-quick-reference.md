# Write Operations Quick Reference

**Last Updated:** 2025-11-01  
**For:** Claude AI agent and human operators

---

## Prerequisites

```bash
# REQUIRED for all write operations
export DECISION_REVIEWER_ALLOW_WRITES=1
```

---

## Basic Workflow Pattern

```bash
# 1. Query logs to understand what needs changing
decision_reviewer query --json '{"type":"log","decision":"deny","last_n":5}'

# 2. Dry-run to preview and validate changes
decision_reviewer apply --dry-run --json '{
  "type": "add_rule",
  "rule_type": "allow",
  "target_file": "~/.config/claude-code-permissions-hook.toml",
  "rule": { ... }
}'

# 3. Review dry-run output for validation results

# 4. Apply for real
decision_reviewer apply --json '{ ... }'  # Same JSON, without --dry-run

# 5. Verify with match test
decision_reviewer query --json '{
  "type": "match_test",
  "tool": "Bash",
  "input": {"command": "npm install"}
}'
```

---

## Common Operations

### Add Allow Rule

```json
{
  "type": "add_rule",
  "rule_type": "allow",
  "target_file": "~/.config/claude-code-permissions-hook.toml",
  "position": "end",
  "rule": {
    "id": "allow-npm-install",
    "description": "Allow npm install commands",
    "tool": "Bash",
    "command_regex": "^npm install( |$)",
    "command_exclude_regex": "&|;|\\||`|\\$\\("
  }
}
```

### Add Deny Rule (Security-Critical)

```json
{
  "type": "add_rule",
  "rule_type": "deny",
  "target_file": "~/.config/claude-code-permissions-hook.toml",
  "position": "start",
  "rule": {
    "id": "deny-destructive-rm",
    "description": "Block destructive rm commands on system paths",
    "tool": "Bash",
    "command_regex": "^rm -rf (/|/etc|/usr|/var)"
  }
}
```

**Important:** Deny rules should typically be added at "start" to ensure they're checked first.

### Remove Rule

```json
{
  "type": "remove_rule",
  "id": "allow-old-pattern",
  "target_file": "~/.config/claude-code-permissions-hook.toml"
}
```

### Update Rule

```json
{
  "type": "update_rule",
  "id": "allow-safe-cargo",
  "target_file": "~/.config/claude-code-permissions-hook.toml",
  "changes": {
    "description": "Updated description with more detail",
    "command_regex": "^cargo (build|test|check|clippy|fmt|run|doc)"
  }
}
```

**Note:** Can update any field except `id`. To change ID, use remove + add.

### Temporarily Disable Rule

```json
{
  "type": "toggle_rule",
  "id": "allow-experimental-feature",
  "enabled": false,
  "target_file": "~/.config/claude-code-permissions-hook.toml"
}
```

**Note:** This comments out the rule rather than deleting it. Use `"enabled": true` to re-enable.

### Batch Operations (All-or-Nothing)

```json
{
  "type": "batch",
  "target_file": "~/.config/claude-code-permissions-hook.toml",
  "operations": [
    {
      "type": "add_rule",
      "rule_type": "allow",
      "rule": {
        "id": "allow-npm-install",
        "tool": "Bash",
        "command_regex": "^npm install"
      }
    },
    {
      "type": "remove_rule",
      "id": "allow-old-npm-pattern"
    },
    {
      "type": "update_llm_config",
      "changes": {"temperature": 0.2}
    }
  ]
}
```

**Important:** If ANY operation fails validation, NONE are applied (atomic).

### Update LLM Configuration

```json
{
  "type": "update_llm_config",
  "target_file": "~/.config/llm-fallback-config.toml",
  "changes": {
    "temperature": 0.2,
    "timeout_secs": 60,
    "max_retries": 3
  }
}
```

### Rollback to Previous State

```json
{
  "type": "rollback",
  "to_commit": "abc123def456"
}
```

**Note:** Get commit hash from Git log or from `rollback_info` in previous responses.

---

## Response Interpretation

### Success Response

```json
{
  "success": true,
  "operation": "add_rule",
  "changes": ["Added rule 'allow-npm-install' at line 42"],
  "rollback_info": {
    "type": "git",
    "commit": "abc123",
    "rollback_command": "decision_reviewer apply --json '{\"type\":\"rollback\",\"to_commit\":\"abc123\"}'"
  },
  "validation": {
    "blocking_passed": true,
    "warnings": []
  }
}
```

**Action:** ✅ Operation successful. Save `rollback_command` if you might need to undo.

### Error Response

```json
{
  "success": false,
  "error": "Validation failed",
  "details": [
    {
      "field": "rule.command_regex",
      "error": "Regex compilation failed: unmatched parenthesis",
      "suggestion": "Check regex syntax. Expected closing ')'"
    }
  ]
}
```

**Action:** ❌ Fix the errors listed in `details` and retry.

### Warning Response (Success with Warnings)

```json
{
  "success": true,
  "operation": "add_rule",
  "changes": ["Added rule ..."],
  "validation": {
    "blocking_passed": true,
    "warnings": [
      {
        "type": "rule_overlap",
        "message": "New rule may overlap with 'allow-npm-patterns'",
        "suggestion": "Consider using exclude patterns"
      }
    ]
  }
}
```

**Action:** ⚠️ Operation succeeded but review warnings. Consider refining rule or use `--force` if intentional.

---

## Safety Checklist

Before applying changes:

- [ ] Environment variable set: `DECISION_REVIEWER_ALLOW_WRITES=1`
- [ ] Dry-run executed and reviewed
- [ ] Validation passed (no blocking errors)
- [ ] Warnings reviewed and understood
- [ ] Target file path is correct
- [ ] Rule ID doesn't conflict with existing rules
- [ ] Regex patterns tested and compile correctly
- [ ] Exclude patterns provide security boundaries
- [ ] In Git repo (for automatic commit) OR backups enabled

---

## Common Patterns

### Pattern 1: Learn from LLM Denial

```bash
# 1. LLM denied something that should be allowed
# 2. Query to see the denial
decision_reviewer query --json '{"type":"log","decision":"deny","last_n":1}'

# 3. Determine it's safe (e.g., "npm install")
# 4. Add hard allow rule
decision_reviewer apply --dry-run --json '{
  "type": "add_rule",
  "rule_type": "allow",
  "target_file": "~/.config/claude-code-permissions-hook.toml",
  "rule": {
    "id": "allow-npm-install",
    "description": "Allow npm install (safe package manager)",
    "tool": "Bash",
    "command_regex": "^npm install",
    "command_exclude_regex": "&|;|\\||`|\\$\\("
  }
}'

# 5. Apply
decision_reviewer apply --json '{ ... }'
```

### Pattern 2: Tighten Security

```bash
# 1. Notice LLM allowing something risky
# 2. Add deny rule to block it explicitly
decision_reviewer apply --json '{
  "type": "add_rule",
  "rule_type": "deny",
  "target_file": "~/.config/claude-code-permissions-hook.toml",
  "position": "start",
  "rule": {
    "id": "deny-system-file-access",
    "description": "Block access to sensitive system files",
    "tool": "Read",
    "file_path_regex": "^/(etc|var|sys|proc)/.*"
  }
}'
```

### Pattern 3: Refine Existing Rule

```bash
# 1. Rule is too broad or too narrow
# 2. Update the regex pattern
decision_reviewer apply --json '{
  "type": "update_rule",
  "id": "allow-cargo-commands",
  "target_file": "~/.config/claude-code-permissions-hook.toml",
  "changes": {
    "command_regex": "^cargo (build|test|check|clippy|fmt|run|doc|bench)"
  }
}'
```

### Pattern 4: Tune LLM Settings

```bash
# 1. LLM too conservative (many safe things denied)
#    → Increase temperature slightly
# 2. LLM too permissive (risky things allowed)
#    → Decrease temperature

decision_reviewer apply --json '{
  "type": "update_llm_config",
  "target_file": "~/.config/llm-fallback-config.toml",
  "changes": {
    "temperature": 0.2
  }
}'
```

---

## Validation Rules

### Blocking Validations (Must Pass)

1. **TOML Syntax:** Must be valid TOML
2. **Regex Compilation:** All regex patterns must compile
3. **XOR Constraint:** Exactly one of `tool` or `tool_regex` required
4. **Required Fields:** All mandatory fields present
5. **Config Load Test:** Final config must load successfully

**If ANY blocking validation fails → operation rejected, no file write.**

### Warning Validations (Show But Allow)

1. **Rule Overlap:** New rule may overlap with existing rules
2. **Broad Pattern:** Regex matches very broadly
3. **Performance Impact:** Complex regex may slow down matching

**Warnings don't block operation but suggest improvements. Use `--force` to suppress.**

---

## Error Recovery

### "Validation failed" Error

**Cause:** Blocking validation didn't pass (syntax, regex, constraints)

**Fix:**
1. Read `details` array in error response
2. Fix each issue listed
3. Retry with corrected JSON

### "Rule with id '...' already exists"

**Cause:** Rule ID collision

**Options:**
1. Use `update_rule` instead of `add_rule`
2. Choose different `id` value
3. Remove existing rule first (if replacing)

### "Config locked" Error

**Cause:** Another process has exclusive lock on config file

**Fix:**
1. Wait a moment and retry
2. Check for other `decision_reviewer` processes

### "Git operation failed"

**Cause:** Git repo issue (not in repo, uncommitted changes, etc.)

**Options:**
1. Fix Git repo state manually
2. Use `--no-commit` flag to skip Git commit
3. Rely on backup fallback (automatic)

---

## Best Practices

### Rule Naming

**Good:**
- `allow-npm-install` - Clear, specific
- `deny-destructive-rm` - Explains purpose
- `allow-safe-cargo-commands` - Descriptive

**Bad:**
- `rule1` - Not descriptive
- `temp` - Unclear intent
- `allow_bash` - Too broad

### Rule Descriptions

**Good:**
```json
{
  "description": "Allow npm install commands for package management (blocks shell injection)"
}
```

**Bad:**
```json
{
  "description": "npm stuff"
}
```

### Regex Patterns

**Good:**
```json
{
  "command_regex": "^npm install( |$)",
  "command_exclude_regex": "&|;|\\||`|\\$\\("
}
```
- Anchored with `^` (start)
- Specific pattern
- Exclude patterns for security

**Bad:**
```json
{
  "command_regex": "npm"
}
```
- Not anchored (matches "run npm" too)
- Too broad
- No exclude patterns (vulnerable)

### Position Strategy

**Deny rules:** `"position": "start"` (checked first)
**Allow rules:** `"position": "end"` (checked after denies)
**High-frequency rules:** Earlier position (performance)

---

## Troubleshooting

### Operation succeeds but behavior unchanged

**Possible causes:**
1. Another rule matching first (rule order matters)
2. Rule syntax correct but logic doesn't match intent
3. Cached config (restart hook if applicable)

**Diagnosis:**
```bash
# Test what rule would match
decision_reviewer query --json '{
  "type": "match_test",
  "tool": "Bash",
  "input": {"command": "your command here"}
}'
```

### Dry-run shows different result than apply

**This should NOT happen.** If it does:
1. File a bug report
2. Check for concurrent modifications
3. Verify target file path is same in both commands

### Rollback doesn't restore expected state

**Possible causes:**
1. Wrong commit hash specified
2. Multiple changes between commits
3. Manual edits after auto-commit

**Fix:**
```bash
# View full Git history
git log --oneline ~/.config/claude-code-permissions-hook.toml

# View specific commit
git show <commit-hash>

# Manual rollback if needed
git checkout <commit-hash> -- ~/.config/claude-code-permissions-hook.toml
```

---

## Performance Tips

1. **Use hard rules for common patterns** (avoid LLM queries)
2. **Order rules by frequency** (frequently-matched rules first)
3. **Batch operations when possible** (single validation, single commit)
4. **Test regex performance** for complex patterns

---

## Security Reminders

1. **Never skip exclude patterns** on security-critical rules
2. **Deny rules take precedence** over allow rules (checked first)
3. **Anchor regex patterns** to prevent unexpected matches (`^` at start)
4. **Review warnings** even if operation succeeds
5. **Keep audit log** for compliance/review
6. **Test in dry-run** before applying to production config

---

## Quick Command Reference

```bash
# Set write permission (required)
export DECISION_REVIEWER_ALLOW_WRITES=1

# Dry-run (preview)
decision_reviewer apply --dry-run --json '{...}'

# Apply (execute)
decision_reviewer apply --json '{...}'

# Force (ignore warnings)
decision_reviewer apply --force --json '{...}'

# Skip Git commit
decision_reviewer apply --no-commit --json '{...}'

# Test rule matching
decision_reviewer query --json '{"type":"match_test",...}'

# View recent logs
decision_reviewer query --json '{"type":"log","last_n":10}'

# Rollback
decision_reviewer apply --json '{"type":"rollback","to_commit":"abc123"}'
```

---

**End of Quick Reference**

For complete details, see: `docs/write-operations-design.md`
