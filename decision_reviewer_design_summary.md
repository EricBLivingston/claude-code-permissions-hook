# Decision Reviewer Design Summary

## Gemini Consultation Results

This document summarizes the key design decisions made based on Gemini's brainstorming and technical recommendations for the `decision_reviewer` tool.

## Top Ideas from Brainstorming

### 1. Daemon Mode with In-Memory Caching ⭐
**Impact: 5/5 | Feasibility: 3/5**

The most direct way to meet <100ms performance requirements for API consumers. Recommended for **future enhancement** (Phase 2+), starting with stateless architecture first.

### 2. What-If Scenario Simulator ⭐
**Impact: 5/5 | Feasibility: 3/5**

Test proposed rule changes against historical data before applying them. Essential for safely evolving the ruleset. Recommended for **Phase 4+**.

### 3. Rule Coverage Analyzer ⭐
**Impact: 5/5 | Feasibility: 3/5**

Proactively identify gaps in security policy by testing common commands. **Implemented in Phase 4** as the `gaps` query type and `analyze` command.

### 4. Permission Entitlement Reporting ⭐
**Impact: 5/5 | Innovation: 5/5**

Query what *is possible* rather than what *happened*. "What commands is Claude allowed to run?" **Core to v1.0** via the `match_test` query type.

### 5. Rule Source Tracing ⭐
**Impact: 5/5 | Feasibility: 2/5**

Trace rules back to source file and line number through multiple includes. **Implemented in Phase 2** for debugging modular configurations.

### Other Notable Ideas

- **Explain a Decision** - Step-by-step decision flow (Implemented in Phase 3)
- **Interactive TUI Dashboard** - Live-updating terminal UI (Future enhancement)
- **LLM-Powered Natural Language Query** - Ask questions in plain English (Future enhancement)
- **Config Linter and Optimizer** - Analyze config for issues (Future enhancement)
- **Temporal Query Rewind** - Time-travel for policy analysis (Future enhancement)

## Technical Design Decisions

### 1. Query Interface Format

**Decision: Custom JSON query format**

**Rationale:**
- Simple for Claude (LLMs excel at generating structured JSON)
- Easy to implement in Rust (serde + serde_json)
- Evolvable (start simple, add complexity later)
- GraphQL/DSL are overkill for local tool

**Example:**
```json
{"type": "log", "decision": "deny", "last_n": 10}
```

### 2. Configuration Format

**Decision: No separate config file, auto-discovery with --config override**

**Rationale:**
- Reviewer reads hook's native TOML format (consistency)
- Separate config to point to another config is redundant
- Auto-discovery is convenient for common case
- Required flag provides explicit override path

**Search order:**
1. `--config` flag (if provided)
2. `CLAUDE_PERMISSIONS_CONFIG` env var
3. `./claude-code-permissions-hook.toml`
4. `../.config/claude-code-permissions-hook.toml`
5. `~/.config/claude-code-permissions-hook.toml`
6. `/etc/claude-code-permissions-hook.toml`

### 3. Match Testing Implementation

**Decision: Direct call to matcher.rs functions via shared library**

**Rationale:**
- 100% accuracy (uses actual production code)
- No logic drift (single source of truth)
- Maintainable (no duplicated logic)

**LLM Handling:**
- Report "would_consult" without making network call
- Fast, cheap, sufficient for testing logic

**Example response:**
```json
{
  "would_match": false,
  "decision": "passthrough",
  "llm_consulted": "would_consult",
  "note": "No hard rule matches; would consult LLM in production"
}
```

### 4. Performance Architecture

**Decision: Start with stateless architecture**

**Rationale:**
- Simplest to implement (no process management)
- No caching bugs (everything is fresh)
- Performance likely acceptable for local file reads
- Can evolve to daemon mode if needed

**Optimization strategies:**
- Read logs in reverse for `last_n` queries
- Lazy evaluation of includes
- Sample large datasets for pattern analysis

**Future:** Daemon mode if stateless proves too slow

### 5. Query Language Features

**Decision: Simple approach for v1.0, complex features later**

**v1.0 (Simple):**
```json
{"type": "log", "decision": "deny", "last_n": 10}
```

**Future (Complex):**
```json
{
  "type": "log",
  "filter": {"and": [{"decision": "deny"}, {"tool": "Bash"}]},
  "sort": [{"field": "timestamp", "order": "desc"}],
  "limit": 10,
  "offset": 0,
  "aggregate": {"group_by": "tool", "count": true}
}
```

**Rationale:**
- Simple approach covers 80% of use cases
- YAGNI: Don't build complex unused features
- Iterative development based on real needs

### 6. Output Format

**Decision: Auto-detect TTY, always include metadata, structured errors**

**JSON Output:**
- TTY: Pretty-printed with colors
- Pipe: Compact one-line
- Override with `--compact` or `--pretty`

**Metadata:**
```json
{
  "query_time_ms": 45,
  "result_count": 10,
  "results": [...]
}
```

**Structured Errors:**
```json
{
  "error": {
    "code": "config_not_found",
    "message": "Could not locate hook config file",
    "details": "Searched: ./..., ~/.config/..."
  }
}
```

**Rationale:**
- TTY detection is standard CLI practice
- Metadata enables programmatic clients to understand context
- Structured errors allow clients to react to specific failures

### 7. Security Considerations

**Decision: Trust file system permissions as primary boundary**

**Rationale:**
- Standard for local developer tools
- Operates with user's privileges
- No redundant application-level permissions

**Secondary measures:**
- `--read-only` mode (usability, not security)
- Audit logging and rate limiting deferred (multi-user feature)

### 8. Integration Points

**Decision: Shared library crate in Cargo workspace**

**Architecture:**
```
claude-code-permissions-hook/
├── Cargo.toml (workspace)
├── src/
│   ├── lib.rs (NEW: shared library)
│   ├── bin/
│   │   ├── claude-code-permissions-hook.rs
│   │   ├── llm_test_runner.rs
│   │   └── decision_reviewer.rs (NEW)
│   ├── config.rs (moved to library)
│   ├── matcher.rs (moved to library)
│   ├── llm_safety.rs (moved to library)
│   └── reviewer/ (NEW: reviewer modules)
```

**Rationale:**
- Idiomatic Rust way to share code
- Single source of truth (no drift)
- Clear separation (core logic vs. application entry points)
- Eliminates code duplication

## Key Insights for Claude Integration

### 1. Queryable Security System

The tool transforms the permissions hook from an **opaque barrier** into a **queryable resource** that Claude can learn from and adapt to.

**Example workflow:**
1. Claude attempts operation → denied by hook
2. Claude queries: `{"type":"log","decision":"deny","last_n":1}` to see why
3. Claude reads rule: `{"id":"deny-destructive-rm","description":"Block rm -rf"}`
4. Claude understands boundary and adjusts approach
5. Claude checks alternative: `{"type":"match_test",...}` before attempting
6. Claude proceeds with confidence

### 2. Proactive Learning

Claude can **proactively** understand the security posture before attempting operations:

- **Check config**: `{"type":"config","section":"llm_fallback"}` → Is LLM enabled?
- **Test command**: `{"type":"match_test","tool":"Bash","input":{"command":"npm install"}}` → Would this be allowed?
- **Review denials**: `{"type":"log","decision":"deny","last_n":5}` → What boundaries exist?
- **Find gaps**: `{"type":"gaps","tool":"Bash"}` → What operations hit LLM vs. hard rules?

### 3. Reliable API Contract

**Performance targets:**
- Config queries: <10ms
- Rule queries: <20ms
- Log queries (100 entries): <100ms
- Match test: <50ms

**Reliability:**
- Structured errors with codes
- <1% error rate on valid queries
- 100% accuracy on match testing (uses actual matcher code)

### 4. Human and Machine Synergy

**Machine (Claude) uses:**
- `query` command with JSON
- Fast, programmatic, reliable
- Enables adaptive behavior

**Human (operator) uses:**
- `list`, `stats`, `analyze`, `explain` commands
- Rich formatting, colors, tables
- Comprehensive insights

Both share the same underlying data and logic via the shared library.

## Implementation Strategy

### Phase 1: Core API (Week 1)
- Shared library architecture
- `query` command with config/rule queries
- Foundation for all future work

### Phase 2: Log & Match Testing (Week 2)
- Log queries with filtering
- Match testing with rule tracing
- Critical for Claude's adaptive workflow

### Phase 3: Human Commands (Week 3)
- Rich CLI for operators
- `list`, `stats`, `explain`, `validate-config`
- Human-friendly analysis

### Phase 4: Analysis (Week 4)
- Pattern detection
- Gap analysis
- Rule suggestions
- Actionable insights

### Phase 5: Gemini Integration (Week 5)
- LLM-powered analysis
- Advanced pattern recognition
- High-level insights

### Phase 6: Polish (Week 6)
- Documentation
- Performance optimization
- Security audit
- Production-ready release

## Success Criteria

### For Claude (Machine API)
- ✅ <1% error rate on valid queries
- ✅ 95th percentile query time <200ms
- ✅ Match test results 100% match actual hook
- ✅ All critical query types supported

### For Humans (CLI)
- ✅ Usable without reading docs
- ✅ Actionable insights in output
- ✅ Commands complete in <5s
- ✅ Comprehensive help text

### Overall
- ✅ Zero regressions in hook binary
- ✅ >80% code reuse via shared library
- ✅ >85% test coverage
- ✅ Complete API reference

## Conclusion

The design prioritizes:
1. **Reliability** - Stateless, shared library, structured errors
2. **Simplicity** - JSON queries, auto-discovery, standard patterns
3. **Performance** - <200ms for 95% of queries, optimized log scanning
4. **Maintainability** - Single source of truth, clear separation of concerns
5. **Evolvability** - Start simple, add complexity based on real needs

This foundation enables Claude to interact intelligently with the security system while providing powerful analysis tools for human operators.

**Timeline:** 6 weeks
**Risk:** Low (building on proven codebase)
**Impact:** High (transforms barrier into resource)
