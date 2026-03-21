---
name: rust-checks
description: Checks Rust code for compilation, formatting, linting, anti-patterns, and idiomatic issues
severity-default: medium
tools: [Read, Bash, Grep]
---

# Rust Code Quality Checks

## Critical Compilation Checks

**Must pass before reviewing other issues:**

1. Run `cargo check` - code must compile without errors
2. Run `cargo test` - all tests must pass
3. Run `cargo clippy -- -D warnings` - no clippy warnings allowed

Report any compilation or test failures as **critical** severity.

## Anti-Pattern Detection

Search for these problematic patterns (report as **high** severity):

### Clone to Satisfy Borrow Checker
- Look for `.clone()` calls that appear to work around borrow checker errors
- Check if the same value is cloned multiple times in tight proximity
- Exception: `Rc::clone()` and `Arc::clone()` are intentional and fine
- Suggest: Use `mem::take`, restructure borrows, or redesign data flow

### Deref Polymorphism
- Search for `impl Deref` on non-pointer types (not `Box`, `Rc`, `Arc`, etc.)
- Check if `Deref` is being used to create inheritance-like relationships
- Suggest: Use composition with explicit methods or traits instead

### `#![deny(warnings)]` in Crate Root
- Check for `#![deny(warnings)]` attribute in `lib.rs` or `main.rs`
- Suggest: Use specific `#![deny(lint_name)]` or `RUSTFLAGS="-D warnings"` in CI

## Common Mistakes (report as **medium** severity)

### Error Handling
- Search for `.unwrap()` in production code paths (not in tests or examples)
- Search for `.expect("...")` in production code paths
- Check if functions return `Result` or just `panic!`
- Suggest: Use `?` operator for error propagation

### Ownership Issues
- Look for function parameters taking `&String` instead of `&str`
- Look for function parameters taking `&Vec<T>` instead of `&[T]`
- Look for function parameters taking `&Box<T>` instead of `&T`
- Suggest: Use borrowed types for arguments

### Missing Documentation
- Check if `pub` items lack doc comments (`///`)
- Check if public functions lack examples in their doc comments
- Suggest: Add doc comments to all public APIs

### Missing Traits
- Look for structs that could derive common traits but don't
- Check if `Debug`, `Clone`, `PartialEq`, `Default` are missing where appropriate
- Suggest: Add derives or manual implementations

## Idiomatic Rust Issues (report as **low** severity)

### Constructor Conventions
- Check if types with `new()` method also implement `Default` (should have both)
- Check if `new()` is missing for types that have a clear default state
- Suggest: Implement both `new()` and `Default` trait

### String Handling
- Look for manual string concatenation in loops (using `push_str` repeatedly)
- Suggest: Use `format!()` for readability or pre-allocate for performance

### Iterator Patterns
- Look for manual loops that could be replaced with iterator chains
- Check for `.collect()` followed by iteration (wasteful)
- Suggest: Use `.map()`, `.filter()`, `.fold()` where appropriate

## Test Coverage

Check that:
- Every public function has a corresponding test in `tests/`
- Tests have descriptive names explaining what they test
- Tests cover both happy path and error/edge cases
- Test names follow convention: `test_function_name_condition_expected_behavior`

Report missing test coverage as **medium** severity.

## Project Structure

Verify standard Rust layout:
- `src/main.rs` or `src/lib.rs` exists
- Tests are in `tests/` directory, not scattered in `src/`
- Module structure uses proper `mod.rs` or file-based modules
- No deeply nested module structures (>3 levels suggests poor organization)

Report as **low** severity if structure deviates significantly.

## Reporting Format

For each issue found:
1. **Location**: File path and line number
2. **Issue**: What's wrong (reference specific anti-pattern or idiom)
3. **Impact**: Why it matters (performance, safety, maintainability)
4. **Fix**: Concrete code suggestion or refactoring approach

Example:
```
src/parser.rs:42
Issue: Using .clone() to satisfy borrow checker
Impact: Unnecessary allocation and copies; changes to one variable won't reflect in cloned copies
Fix: Use mem::take(&mut self.buffer) to move the value without cloning, or restructure to borrow differently
```
