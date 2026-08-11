//! 本地用户标识。
//!
//! Zap 剥离了 `warp_server_auth`,`UserUid` 由本 crate 自持,语义与
//! `app/src/auth/user_uid.rs` 保持一致。

pub mod user_uid;

pub use user_uid::{TEST_USER_EMAIL, TEST_USER_UID, UserUid};
