# Decision Reviewer Implementation Plan

## Overview

The `decision_reviewer` is a dual-purpose tool that serves both human operators and Claude Code as a machine-queryable API. It provides comprehensive visibility into the permissions hook's decisions, configuration, and rule coverage.

**Key Design Principles** (from Gemini consultation):
- **Custom JSON query format** for programmatic access (simple, evolvable)
- **Stateless architecture** (reliable, no caching bugs)
- **Shared library crate** for code reuse (single source of truth)
- **Auto-discovery** of hook config (convenient, explicit override)
- **Direct matcher reuse** for accurate match testing

## Architecture

### Crate Structure

```
claude-code-permissions-hook/
├── Cargo.toml (workspace)
├── src/
│   ├── bin/
│   │   ├── claude-code-permissions-hook.rs (existing hook binary)
│   │   ├── llm_test_runner.rs (existing test runner)
│   │   └── decision_reviewer.rs (NEW: reviewer binary)
│   ├── lib.rs (NEW: library exports)
│   ├── config.rs (moved to library)
│   ├── matcher.rs (moved to library)
│   ├── llm_safety.rs (moved to library)
│   ├── hook_io.rs (moved to library)
│   ├── logging.rs (moved to library)
│   └── reviewer/ (NEW: reviewer-specific modules)
│       ├── mod.rs
│       ├── query.rs (JSON query interface)
│       ├── analysis.rs (statistical analysis, pattern detection)
│       ├── suggestions.rs (rule gap analysis, suggestions)
│       ├── gemini_review.rs (Gemini-powered analysis)
│       └── display.rs (human-friendly formatting)
└── Cargo.lock
```

### Workspace Configuration

Update `Cargo.toml` to define library and binaries:

```toml
[package]
name = "claude-code-permissions"
version = "0.2.0"

[lib]
name = "claude_code_permissions"
path = "src/lib.rs"

[[bin]]
name = "claude-code-permissions-hook"
path = "src/bin/claude-code-permissions-hook.rs"

[[bin]]
name = "llm_test_runner"
path = "src/bin/llm_test_runner.rs"

[[bin]]
name = "decision_reviewer"
path = "src/bin/decision_reviewer.rs"

[dependencies]
# ... existing dependencies ...
```

## Command Structure

### Primary Interface

```bash
decision_reviewer [OPTIONS] <COMMAND>

OPTIONS:
    --config <PATH>     Path to hook config (overrides auto-discovery)
    --no-color          Disable colored output
    --json              Output in JSON format (for all commands)

COMMANDS:
    query               Execute JSON query (machine API)
    list                List recent decisions (human-friendly)
    stats               Show decision statistics
    analyze             Analyze patterns and gaps
    gemini-review       Gemini-powered analysis of passthroughs
    suggest-rules       Generate rule suggestions
    explain             Explain a specific decision
    validate-config     Validate hook configuration
    help                Print help information
```

### Command Priority (Implementation Order)

**Phase 1: Core API (MVP)**
1. `query` - JSON query interface (highest priority for Claude)
2. `validate-config` - Config validation (reuse existing logic)
3. `list` - Basic log viewing (debugging)

**Phase 2: Analysis**
4. `stats` - Statistical summaries
5. `analyze` - Pattern detection and gap analysis
6. `explain` - Decision explanation with rule tracing

**Phase 3: Advanced Features**
7. `suggest-rules` - Rule gap coverage suggestions
8. `gemini-review` - LLM-powered analysis

## JSON Query Interface

### Query Format (v1.0)

All queries follow this structure:

```json
{
  "type": "query_type",
  "filters": { ... },
  "options": { ... }
}
```

### Supported Query Types

#### 1. Config Queries

**Get entire merged config:**
```json
{"type": "config", "full": true}
```

Response:
```json
{
  "query_time_ms": 15,
  "result": {
    "logging": {"log_file": "/tmp/hook.log", ...},
    "llm_fallback": {"enabled": true, "model": "claude-haiku-4.5", ...},
    "allow": [...],
    "deny": [...]
  }
}
```

**Get specific section:**
```json
{"type": "config", "section": "llm_fallback"}
```

Response:
```json
{
  "query_time_ms": 8,
  "result": {
    "enabled": true,
    "endpoint": "https://openrouter.ai/api/v1",
    "model": "anthropic/claude-haiku-4.5",
    "timeout_secs": 60,
    "temperature": 0.1,
    "max_retries": 2
  }
}
```

**Get config sources (includes tracking):**
```json
{"type": "config", "sources": true}
```

Response:
```json
{
  "query_time_ms": 5,
  "result": {
    "base": "/home/user/.config/claude-code-permissions-hook.toml",
    "includes": [
      "/home/user/.config/llm-fallback-config.toml",
      "/home/user/.config/shared-rules.toml"
    ]
  }
}
```

#### 2. Rule Queries

**All rules of a type:**
```json
{"type": "rule", "filter": "allow"}
```

**Rules for specific tool:**
```json
{"type": "rule", "tool": "Bash"}
```

**Specific rule by ID:**
```json
{"type": "rule", "id": "allow-safe-cargo"}
```

**Rule count statistics:**
```json
{"type": "rule_count"}
```

Response:
```json
{
  "query_time_ms": 3,
  "result": {
    "allow": 15,
    "deny": 8,
    "total": 23,
    "by_tool": {
      "Bash": 10,
      "Read": 5,
      "Write": 3,
      "Edit": 2,
      "Glob": 2,
      "Task": 1
    }
  }
}
```

**Rule sources with line numbers:**
```json
{"type": "rule", "id": "allow-safe-cargo", "with_source": true}
```

Response includes source file and line number where rule is defined.

#### 3. Log Queries

**Recent denials:**
```json
{"type": "log", "decision": "deny", "last_n": 10}
```

**LLM decisions:**
```json
{"type": "log", "source": "llm", "last_n": 20}
```

**Decisions for specific tool:**
```json
{"type": "log", "tool": "Bash", "since": "24h"}
```

**Flagged decisions needing review:**
```json
{"type": "log", "needs_review": true}
```

**Decisions in session:**
```json
{"type": "log", "session_id": "abc123"}
```

**Time-based filtering:**
```json
{
  "type": "log",
  "since": "24h",
  "decision": "passthrough"
}
```

Time formats supported: `"1h"`, `"24h"`, `"7d"`, ISO timestamps

Response format:
```json
{
  "query_time_ms": 45,
  "result_count": 10,
  "results": [
    {
      "timestamp": "2025-11-01T10:30:45Z",
      "tool": "Bash",
      "decision": "deny",
      "source": "rule",
      "rule_id": "deny-destructive-rm",
      "input": {"command": "rm -rf /"},
      "reasoning": "Matched deny pattern for destructive rm"
    }
  ]
}
```

#### 4. Match Testing (Simulation)

**Test if command would be allowed:**
```json
{
  "type": "match_test",
  "tool": "Bash",
  "input": {"command": "cargo test"}
}
```

Response:
```json
{
  "query_time_ms": 12,
  "result": {
    "would_match": true,
    "decision": "allow",
    "matched_rule": {
      "id": "allow-safe-cargo",
      "type": "allow",
      "source_file": "~/.config/claude-code-permissions-hook.toml",
      "source_line": 42
    },
    "llm_consulted": false
  }
}
```

**Test that would hit LLM:**
```json
{
  "type": "match_test",
  "tool": "Bash",
  "input": {"command": "git push origin main"}
}
```

Response:
```json
{
  "query_time_ms": 8,
  "result": {
    "would_match": false,
    "decision": "passthrough",
    "matched_rule": null,
    "llm_consulted": "would_consult",
    "note": "No hard rule matches; would consult LLM in production"
  }
}
```

#### 5. Analysis Queries

**Statistics for timeframe:**
```json
{
  "type": "stats",
  "since": "24h"
}
```

Response:
```json
{
  "query_time_ms": 120,
  "result": {
    "total_decisions": 1543,
    "by_decision": {
      "allow": 1120,
      "deny": 45,
      "passthrough": 378
    },
    "by_source": {
      "hard_rule": 1165,
      "llm": 378
    },
    "by_tool": {
      "Bash": 856,
      "Read": 432,
      "Write": 123
    },
    "top_denied_commands": [
      {"command": "rm -rf /", "count": 15}
    ]
  }
}
```

**Common passthrough patterns:**
```json
{
  "type": "patterns",
  "decision": "passthrough"
}
```

Response:
```json
{
  "query_time_ms": 200,
  "result": {
    "clusters": [
      {
        "pattern": "git push",
        "count": 45,
        "examples": ["git push origin main", "git push -f"]
      },
      {
        "pattern": "docker run",
        "count": 32,
        "examples": []
      }
    ]
  }
}
```

**Rule gaps (commands hitting LLM/passthrough):**
```json
{
  "type": "gaps",
  "tool": "Bash"
}
```

Response:
```json
{
  "query_time_ms": 180,
  "result": {
    "total_gaps": 87,
    "suggestions": [
      {
        "pattern": "git push",
        "count": 45,
        "risk": "medium",
        "suggested_rule": {
          "tool": "Bash",
          "command_regex": "^git push ",
          "description": "Allow git push operations"
        }
      }
    ]
  }
}
```

### Error Responses

All errors follow this format:

```json
{
  "error": {
    "code": "config_not_found",
    "message": "Could not locate hook config file",
    "details": "Searched: ./claude-code-permissions-hook.toml, ~/.config/..."
  }
}
```

Error codes:
- `config_not_found` - Hook config file not found
- `config_parse_error` - Config file is invalid TOML
- `log_not_found` - Log file not found
- `invalid_query` - Malformed query JSON
- `unsupported_query_type` - Unknown query type
- `internal_error` - Unexpected error

### Output Format

**Auto-detection:**
- TTY (terminal): Pretty-printed JSON with colors
- Pipe/redirect: Compact JSON (one-line)

**Force compact:**
```bash
decision_reviewer query --compact '{"type":"config","section":"llm_fallback"}'
```

**Force pretty:**
```bash
decision_reviewer query --pretty '{"type":"log","last_n":10}'
```

## Implementation Phases

### Phase 1: Core Infrastructure (Week 1)

**Objective:** Set up shared library architecture and basic query interface.

**Tasks:**
1. Create `src/lib.rs` and move core modules to library
2. Update `Cargo.toml` for workspace with library + binaries
3. Create `src/bin/decision_reviewer.rs` skeleton
4. Implement config auto-discovery logic
5. Create `src/reviewer/query.rs` with query parsing
6. Implement basic query types: `config`, `rule_count`, `rule`
7. Add JSON output with metadata (query_time_ms, result_count)
8. Add error handling with structured error objects
9. Unit tests for query parsing and execution

**Deliverables:**
- Shared library crate working for hook and reviewer
- `decision_reviewer query` command functional
- Config queries working (full, section, sources)
- Rule queries working (filter, tool, id, count)

**Validation:**
```bash
# Test config queries
decision_reviewer query '{"type":"config","section":"llm_fallback"}'
decision_reviewer query '{"type":"config","full":true}'
decision_reviewer query '{"type":"config","sources":true}'

# Test rule queries
decision_reviewer query '{"type":"rule_count"}'
decision_reviewer query '{"type":"rule","filter":"allow"}'
decision_reviewer query '{"type":"rule","tool":"Bash"}'
```

### Phase 2: Log Queries and Match Testing (Week 2)

**Objective:** Enable log analysis and match simulation.

**Tasks:**
1. Create log parsing module in `src/reviewer/query.rs`
2. Implement time-based filtering (since, until)
3. Implement log filtering (decision, source, tool)
4. Add pagination support (last_n, offset)
5. Implement match_test query type using shared matcher.rs
6. Handle LLM fallback in match testing ("would_consult" response)
7. Add rule source tracing (file + line number)
8. Performance optimization for log scanning (reverse reading)
9. Unit tests for log queries and match testing

**Deliverables:**
- Log queries functional for all filter combinations
- Match testing working with accurate simulation
- Rule source tracing showing where rules are defined

**Validation:**
```bash
# Test log queries
decision_reviewer query '{"type":"log","last_n":10}'
decision_reviewer query '{"type":"log","decision":"deny","since":"24h"}'
decision_reviewer query '{"type":"log","source":"llm"}'

# Test match testing
decision_reviewer query '{"type":"match_test","tool":"Bash","input":{"command":"cargo test"}}'
decision_reviewer query '{"type":"match_test","tool":"Bash","input":{"command":"rm -rf /"}}'
decision_reviewer query '{"type":"match_test","tool":"Read","input":{"file_path":"/etc/passwd"}}'
```

### Phase 3: Human-Friendly Commands (Week 3)

**Objective:** Build rich CLI commands for human operators.

**Tasks:**
1. Create `src/reviewer/display.rs` for formatting
2. Implement `list` command with table output
3. Implement `stats` command with summary output
4. Implement `explain` command with decision flow visualization
5. Implement `validate-config` command (reuse existing validation)
6. Add colored output support (with --no-color flag)
7. Add TTY detection for auto-formatting
8. Add verbose modes for detailed output
9. Integration tests for all human commands

**Deliverables:**
- All human-friendly commands working
- Rich formatting with colors and tables
- Comprehensive help text for all commands

### Phase 4: Analysis Features (Week 4)

**Objective:** Add pattern detection and rule gap analysis.

**Tasks:**
1. Create `src/reviewer/analysis.rs` module
2. Implement pattern clustering for passthroughs
3. Implement rule gap detection algorithm
4. Implement `analyze` command with gap reporting
5. Implement `suggest-rules` command with TOML output
6. Add `stats` analysis queries (patterns, gaps)
7. Add risk scoring heuristics for gaps
8. Performance optimization for large log analysis
9. Integration tests for analysis features

### Phase 5: Gemini Integration (Week 5)

**Objective:** Add LLM-powered analysis capabilities.

**Tasks:**
1. Create `src/reviewer/gemini_review.rs` module
2. Implement Gemini MCP integration for analysis
3. Add prompt templates for different analysis types
4. Implement `gemini-review` command
5. Add markdown report generation
6. Add caching for expensive Gemini calls
7. Error handling for Gemini timeouts/failures
8. Integration tests for Gemini features

### Phase 6: Polish and Documentation (Week 6)

**Objective:** Production-ready release.

**Tasks:**
1. Comprehensive documentation in README
2. Examples for all query types
3. Integration guide for Claude Code
4. Performance benchmarking and optimization
5. Security audit of query interface
6. Release notes and changelog
7. Installation instructions
8. Usage examples and tutorials

## Performance Targets

### Query Response Times

| Query Type | Target | Max Acceptable |
|------------|--------|----------------|
| Config queries | <10ms | 50ms |
| Rule queries | <20ms | 100ms |
| Log queries (last_n ≤ 100) | <100ms | 500ms |
| Match test | <50ms | 200ms |
| Stats (24h) | <200ms | 1s |
| Patterns analysis | <500ms | 3s |
| Gaps analysis | <1s | 5s |

## Success Metrics

### For Claude (Machine API)

1. **Reliability:** <1% error rate on valid queries
2. **Speed:** 95th percentile query time <200ms
3. **Accuracy:** Match test results 100% match actual hook behavior
4. **Coverage:** All critical query types supported

### For Humans (CLI)

1. **Usability:** New users can run basic commands without reading docs
2. **Richness:** Output provides actionable insights
3. **Performance:** Commands complete in <5s for typical use cases
4. **Help:** Comprehensive help text and examples

## Conclusion

This implementation plan provides a comprehensive roadmap for building `decision_reviewer` as a dual-purpose tool that serves both human operators and Claude Code as a reliable machine-queryable API.

**Key Success Factors:**
- Shared library architecture ensures accuracy and maintainability
- Stateless design provides reliability without caching complexity
- JSON query interface enables programmatic use by Claude
- Rich CLI commands provide human-friendly analysis
- Phased implementation allows incremental delivery and validation

**Estimated Timeline:** 6 weeks for full implementation
**Risk Level:** Low (building on proven hook codebase)
**Expected Impact:** High (transforms opaque barrier into queryable resource)
