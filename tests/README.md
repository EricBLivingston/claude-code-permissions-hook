# Test Cases

This directory contains sample JSON inputs for testing the command permissions hook.

## Running Tests

First, build the project:
```bash
cargo build
```

Then test each scenario:

### 1. Read allowed (should allow)
```bash
cat tests/read_allowed.json | cargo run -- run --config example.toml
```
Expected: Allow with reason

### 2. Read with path traversal (should deny)
```bash
cat tests/read_path_traversal.json | cargo run -- run --config example.toml
```
Expected: Deny with reason

### 3. Bash with shell injection (should deny)
```bash
cat tests/bash_injection.json | cargo run -- run --config example.toml
```
Expected: Deny with reason

### 4. Bash allowed (should allow)
```bash
cat tests/bash_allowed.json | cargo run -- run --config example.toml
```
Expected: Allow with reason

### 5. Unknown tool (should passthrough)
```bash
cat tests/unknown_tool.json | cargo run -- run --config example.toml
```
Expected: No output (passthrough)

## Validate Config

To validate the example configuration:
```bash
cargo run -- validate --config example.toml
```
