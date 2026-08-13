/// A app-unique version number for content.
/// This is used for tracking and comparing versions of content across the application.
/// The Rich Text Buffer and the LocalFileModel use this for comparing versions of content.
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, PartialEq, Debug, Copy, Eq, PartialOrd, Ord, Hash)]
pub struct ContentVersion(usize);

impl ContentVersion {
    /// Constructs a new app-unique content version.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let raw = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        ContentVersion(raw)
    }

    pub fn as_i32(&self) -> i32 {
        self.0 as i32
    }

    /// Reconstructs a `ContentVersion` from a raw value received over the wire.
    ///
    /// This bypasses the global atomic counter and should only be used at
    /// protocol deserialization boundaries (e.g. converting a `u64` from a
    /// proto message back into a `ContentVersion`).
    pub fn from_raw(val: usize) -> Self {
        ContentVersion(val)
    }

    /// 协议反序列化辅助:从 wire 上的 `u64` 构造 `ContentVersion`,
    /// 在 32-bit 平台上做饱和(`usize::MAX`)而不是隐式截断。
    /// 当前仓库的 native build 都是 64-bit,这里只是把行为显式化,避免
    /// `as usize` 在小概率 32-bit 编译里悄悄丢高位。
    pub fn from_wire_u64(val: u64) -> Self {
        ContentVersion(usize::try_from(val).unwrap_or(usize::MAX))
    }

    /// Returns the underlying value as a `u64` for wire serialization.
    pub fn as_u64(&self) -> u64 {
        self.0 as u64
    }
}

#[cfg(test)]
#[path = "content_version_tests.rs"]
mod tests;
