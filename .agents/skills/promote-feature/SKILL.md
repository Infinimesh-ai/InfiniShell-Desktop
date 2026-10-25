---
name: promote-feature
description: 将 InfiniShell FeatureFlag 推广到用户指定的 OSS 或兼容通道，核对 GUI/TUI 实际启用条件并保留回滚边界，不自动发布或创建外部跟踪项。
---

# promote-feature

按用户指定的目标通道扩大发布范围，保留可回滚的开关及后续清理说明。默认以本仓库 OSS 行为为准；Dogfood/Preview/Stable 仅在目标确为对应上游兼容通道时使用。

## Overview

Feature flags have two interacting layers:
- **Runtime** (`crates/warp_features/src/lib.rs`): `DOGFOOD_FLAGS`, `PREVIEW_FLAGS`, `RELEASE_FLAGS` — enabled per-channel at startup.
- **Compile-time** (`app/Cargo.toml` + `app/src/features.rs`): Cargo features in `[features]`. The `default = [...]` array enables a feature for all builds. `enabled_features()` in `app/src/features.rs` bridges each Cargo feature to its `FeatureFlag` variant via `#[cfg(feature = "...")]`.

推广后保留合理稳定期和可回滚的启用入口。已存在 Cargo 桥接时可撤回 `default` 条目，纯运行时开关则撤回对应启用列表；稳定后按 `remove-feature-flag` 清理。

## TUI note
The per-channel arrays in `crates/warp_features/src/lib.rs` (`DOGFOOD_FLAGS`/`PREVIEW_FLAGS`/`RELEASE_FLAGS`) are shared and drive both the GUI desktop app (`app/`) and the headless TUI (`crates/warp_tui`) at runtime. The `app/Cargo.toml` `default` + `app/src/features.rs` `enabled_features()` compile-time bridge is GUI-app-only. If a promoted feature should also reach the TUI, make sure it is enabled at runtime for the target channel (verify the actual target binary initialization) and, if the feature relies on a compile-time Cargo feature, that the TUI binary (`crates/warp_tui/Cargo.toml`) enables it too.

## Promote to Dogfood

Add the flag to `DOGFOOD_FLAGS` in `crates/warp_features/src/lib.rs`:

```rust
pub const DOGFOOD_FLAGS: &[FeatureFlag] = &[
    // ...
    FeatureFlag::YourFeature,
];
```

同时核对目标入口实际消费该列表，以及 GUI/TUI 是否共享启用条件。

## Promote to Preview

1. Add to `PREVIEW_FLAGS` in `crates/warp_features/src/lib.rs`.
2. Remove from `DOGFOOD_FLAGS` if present — Preview flags are automatically included in Dogfood builds.

```rust
pub const PREVIEW_FLAGS: &[FeatureFlag] = &[
    // ...
    FeatureFlag::YourFeature,
];
```

## Promote to OSS / Stable

先核对目标 SHA 的通道入口与 `app/src/features.rs`、TUI 初始化。纯运行时功能优先修改适用的运行时启用列表；只有已经使用 Cargo feature 的功能才需要以下编译期桥接，不为推广单独引入编译期开关。

### 1. `app/Cargo.toml` — add to `default`

Add the snake_case feature name to the `default = [...]` array:

```toml
default = [
    # ...
    "your_feature_name",
]
```

Prefer this over adding to `RELEASE_FLAGS` (see the current comment above `RELEASE_FLAGS` in `crates/warp_features/src/lib.rs`). It compiles the feature into all builds and enables a one-line rollback.

### 2. `app/src/features.rs` — add to `enabled_features()` bridge

Add a `#[cfg(...)]` entry inside the `flags.extend([...])` block in `enabled_features()`, following the existing pattern:

```rust
#[cfg(feature = "your_feature_name")]
FeatureFlag::YourFeature,
```

Place it near logically related entries.

### 3. `crates/warp_features/src/lib.rs` — remove from `PREVIEW_FLAGS` / `DOGFOOD_FLAGS`

Remove the variant from whichever arrays it currently lives in:

```rust
pub const PREVIEW_FLAGS: &[FeatureFlag] = &[
    // Remove FeatureFlag::YourFeature,
];
```

### Validate

```bash
./script/format
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
```

### 后续清理

在交付说明中记录稳定期和移除开关的条件。只有用户明确要求创建跟踪项时，才在其指定系统中执行；本 skill 不授权自动创建 Linear issue、发送消息、推送或公开发布。
