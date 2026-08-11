// Zap:`UserUid` 上游已抽到 `crates/cloud_objects`,合并时我方恢复了一份结构相同的
// 本地副本(auth/mod.rs 依赖它),结果同名类型在两处定义,跨 crate 传递时触发 E0308。
// 改为直接复用 crate 版本消除分裂;`TEST_USER_UID` 仍由 auth/mod.rs 自行定义。
pub use cloud_objects::auth::user_uid::UserUid;
