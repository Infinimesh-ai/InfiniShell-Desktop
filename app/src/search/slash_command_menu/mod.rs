pub mod fuzzy_match;
pub mod static_commands;

#[cfg(test)]
mod fuzzy_match_tests;

// Zap:上游新增的 `SlashCommandArgumentHint`(把 `Argument::hint_text` 包成带
// `input_prefix` 的结构体)没有随 `static_commands` 一起并入,我方 `Argument` 仍直接
// 暴露 `hint_text`,且没有任何调用点引用该类型,故不再 re-export。
pub use static_commands::{SlashCommandId, StaticCommand};
