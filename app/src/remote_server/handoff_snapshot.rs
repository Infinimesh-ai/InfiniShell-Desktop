//! Zap:本模块原是 `UploadHandoffSnapshot` RPC 的守护进程侧实现 —— 在远程 SSH
//! 会话里把 git patch 与游离文件打包上传到 GCS,供 local→cloud 交接使用。
//!
//! Zap 是本地优先 fork,整条 local→cloud 交接链路已下线:
//! - `crate::ai::blocklist::handoff`(含 `touched_repos::derive_touched_workspace`)
//!   文件虽保留在磁盘上,但已不在 `ai::blocklist::mod` 中挂载;
//! - `crate::server::server_api`(`AIClient` / `InitialSnapshotToken`)与
//!   `warp_graphql` 云端网关已物理删除;
//! - `ai::agent_sdk::driver::upload_snapshot_for_handoff` 随之移除。
//!
//! 因此这里不再提供 `gather_and_upload_handoff_snapshot`。守护进程侧的
//! `UploadHandoffSnapshot` 处理器也应一并移除(见 `remote_server::server_model`)。
//! 保留本文件是为了给日后接回"本地/自托管快照上传"留一个落点。

#![allow(dead_code)]
