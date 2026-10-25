---
name: remove-feature-flag
description: Remove a feature flag after it has been rolled out and stabilized in the Warp codebase.
---

# remove-feature-flag

Remove a feature flag after it has been rolled out and stabilized in the Warp codebase.

## Overview

After a feature flag has been enabled for all users and has stabilized in production, the flag should be removed to reduce technical debt and simplify the codebase. This involves removing the flag definition and all conditional checks.

## When to Remove

Remove a feature flag when:
- 已核对功能在目标通道和相关 GUI/TUI 中实际启用；不能只根据 `app/Cargo.toml` 的 `default` 判断
- The feature has been stable in production for a reasonable period
- There are no plans to disable the feature or provide configuration options
- The team agrees the feature is permanent

## Steps

### 1. Remove from app/Cargo.toml
仅当该运行时开关存在对应 Cargo feature 时，删除 `[features]` 和 `default` 中的相关条目；不要删除仍被其他能力使用的依赖开关：

```toml
[features]
default = [
    # Remove "your_feature_name" from here
]

# Remove this line:
# your_feature_name = []
```

### 2. Remove from FeatureFlag enum
Remove the variant from the `FeatureFlag` enum in `crates/warp_features/src/lib.rs`:

```rust
#[derive(Sequence)]
pub enum FeatureFlag {
    // Remove YourFeatureName,
}
```

### 3. Remove from app/src/features.rs
若存在对应编译期桥接，删除该条目；同时核对 OSS 桌面专用启用列表及 TUI 初始化：

```rust
// Remove these lines:
// #[cfg(feature = "your_feature_name")]
// YourFeatureName,
```

### 4. Remove from DOGFOOD_FLAGS/PREVIEW_FLAGS/RELEASE_FLAGS
If the flag was listed in any of these arrays in `features.rs`, remove it:

```rust
pub const DOGFOOD_FLAGS: &[FeatureFlag] = &[
    // Remove FeatureFlag::YourFeatureName,
];
```

### 5. Remove all runtime checks and dead code
Find and remove all `FeatureFlag::YourFeatureName.is_enabled()` checks throughout the codebase:

**Before:**
```rust
if FeatureFlag::YourFeatureName.is_enabled() {
    // new behavior
} else {
    // old behavior (dead code)
}
```

**After:**
```rust
// new behavior (unconditionally enabled)
```

Use ripgrep to find all occurrences. The shared `FeatureFlag` may be used from the headless TUI, so search `crates/warp_tui/` (and other non-`app/` crates), not just `app/` and `crates/warp_core/`:
```bash
rg "YourFeatureName" app/ crates/
```

### 6. Remove keybinding predicates
If the feature flag was used in keybinding enabled predicates, remove the predicate:

**Before:**
```rust
EditableBinding::new(
    "action:name",
    "Action description",
    YourAction::Variant
)
.with_enabled(|| FeatureFlag::YourFeatureName.is_enabled())
.with_key_binding("cmdorctrl-key")
```

**After:**
```rust
EditableBinding::new(
    "action:name",
    "Action description",
    YourAction::Variant
)
.with_key_binding("cmdorctrl-key")
```

### 7. Clean up dead code branches
Remove any code paths that were only executed when the feature was disabled (the `else` branches in feature checks). These are now dead code.

### 8. Run tests and validation
After removing the flag:

```bash
# Format and lint
./script/format
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings

# Run tests
cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2

# Build the GUI app
cargo run

# Also validate the headless TUI if the flag was used there
./script/run-tui
```

## Best Practices

- Remove feature flags promptly after they're no longer needed to reduce technical debt
- When removing a flag, remove ALL related code (checks, dead branches, keybinding predicates)
- Use grep/ripgrep to ensure you've found all occurrences
- Test thoroughly after removal to ensure no regressions
- Consider doing flag removal in a separate PR for easier review

## Example Search Commands

```bash
# Find all occurrences of the flag name (include the TUI and other non-app crates)
rg "YourFeatureName" app/ crates/

# Find feature flag checks
rg "FeatureFlag::YourFeatureName" app/ crates/warp_tui/

# Find cfg attributes
rg 'cfg\(feature = "your_feature_name"\)' app/
```
