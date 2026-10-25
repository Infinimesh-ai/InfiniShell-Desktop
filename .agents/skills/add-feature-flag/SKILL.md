---
name: add-feature-flag
description: Add a new feature flag to gate code changes in the Warp codebase.
---

# add-feature-flag

Add a new feature flag to gate code changes in the Warp codebase.

## Overview

InfiniShell 的 `FeatureFlag` 是运行时开关，定义在 `crates/warp_features/src/lib.rs`，由 `warp_core::features` 重导出。默认优先使用运行时开关；仅当平台或可选依赖导致代码无法编译时，再增加 Cargo feature 和对应桥接。

## TUI note
The `FeatureFlag` enum (`crates/warp_features/src/lib.rs`) and the runtime `FeatureFlag::X.is_enabled()` check are SHARED by both front-ends: the GUI desktop app (`app/`) and the headless TUI (`crates/warp_tui`). The Cargo-feature steps in this skill (`app/Cargo.toml`, `app/src/features.rs`) are GUI-app-specific. Prefer runtime `is_enabled()` checks so a flag works in both front-ends with no per-binary wiring. Only if you truly need a compile-time Cargo feature in the TUI, wire it into `crates/warp_tui/Cargo.toml` as well. Test the TUI with `./script/run-tui`.

## Steps

### 1. 按需增加 Cargo feature
仅在需要编译期开关时，在 `app/Cargo.toml` 的 `[features]` 下添加；初始不要加入 `default`。纯运行时开关跳过本步骤：

```toml
[features]
your_feature_name = []
```

### 2. Add to FeatureFlag enum
Add a new variant to the `FeatureFlag` enum in `crates/warp_features/src/lib.rs`:

```rust
#[derive(Sequence)]
pub enum FeatureFlag {
    YourFeatureName,
}
```

### 3. 按需增加编译期到运行时的桥接
仅在第 1 步新增 Cargo feature 时，在 `app/src/features.rs` 的 `enabled_features()` 中增加对应条目：

```rust
#[cfg(feature = "your_feature_name")]
YourFeatureName,
```

### 4. Gate code with runtime checks
In your code, use the runtime check to conditionally execute feature-gated code:

```rust
if FeatureFlag::YourFeatureName.is_enabled() {
    // feature-gated behavior
}
```

### 5. (Optional) Enable for dogfood builds
To enable the feature by default for Dev/dogfood builds, add it to the `DOGFOOD_FLAGS` array in `features.rs`:

```rust
pub const DOGFOOD_FLAGS: &[FeatureFlag] = &[
    FeatureFlag::YourFeatureName,
];
```

### 6. Running with feature flags
仅在该 Cargo feature 已定义且已桥接时，以下命令才会启用运行时开关。执行前遵守 `AGENTS.md` 的本机测试配置：

```bash
cargo run --features your_feature_name

# Multiple features:
cargo run --features your_feature_name,another_feature
```

## Keybindings with Feature Flags

If adding an `EditableBinding` or `FixedBinding` that's part of a gated feature, include an enabled predicate that checks the feature flag. This prevents the keybinding from appearing in keyboard settings when the feature is disabled.

Example:
```rust
EditableBinding::new(
    "action:name",
    "Action description",
    YourAction::Variant
)
.with_enabled(|| FeatureFlag::YourFeatureName.is_enabled())
.with_key_binding("cmdorctrl-key")
```

## 扩大发布范围

使用 `promote-feature` 约定检查实际目标通道。OSS 桌面初始化见 `app/src/features.rs`，TUI 需要单独核对；上游 Dogfood/Preview 列表不自动代表 OSS 可用。

## Best Practices

- **Prefer runtime checks over cfg directives**: Use `FeatureFlag::YourFeatureName.is_enabled()` instead of `#[cfg(...)]` when possible, so flags can be toggled without recompilation and are easier to clean up later
- Use `#[cfg(...)]` only when code cannot compile without the flag (e.g., platform-specific code or missing dependencies)
- Keep flags high-level and product-focused rather than per-call-site
- Remove flags and dead branches after launch has stabilized
