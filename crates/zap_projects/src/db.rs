//! 全进程共享一个 `Mutex<SqliteConnection>` 给项目管理器用。
//!
//! 与 `warp_ssh_manager::db` 同一决策:主写入连接在专门写线程里走 `ModelEvent`
//! channel,接入代价高;SQLite WAL 模式天然支持多写连接(写互斥 + busy_timeout
//! 重试),项目 CRUD 是低频用户操作,冲突可忽略。
//!
//! 路径由 app 启动时经 `set_database_path` 传入;未初始化时 `with_conn` 报错。

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use anyhow::{Result, anyhow};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

static DB_PATH: OnceLock<PathBuf> = OnceLock::new();
static CONN: OnceLock<Mutex<Option<SqliteConnection>>> = OnceLock::new();

/// 由 app 启动时调用一次,传入 sqlite db 文件路径。重复调用会被忽略
/// (OnceLock 语义)。
pub fn set_database_path(path: PathBuf) {
    let _ = DB_PATH.set(path);
}

fn open() -> Result<SqliteConnection> {
    let path = DB_PATH
        .get()
        .ok_or_else(|| anyhow!("zap_projects::db: database path not initialized"))?;
    let url = path.to_string_lossy();
    let mut conn = SqliteConnection::establish(&url)?;
    conn.batch_execute(
        "PRAGMA foreign_keys = ON; \
         PRAGMA busy_timeout = 2000; \
         PRAGMA journal_mode = WAL;",
    )?;
    Ok(conn)
}

/// 锁内执行闭包。首次调用时 lazy 打开连接;后续调用复用。
pub fn with_conn<R>(f: impl FnOnce(&mut SqliteConnection) -> Result<R>) -> Result<R> {
    let mtx = CONN.get_or_init(|| Mutex::new(None));
    let mut guard = mtx
        .lock()
        .map_err(|_| anyhow!("zap_projects db mutex poisoned"))?;
    if guard.is_none() {
        *guard = Some(open()?);
    }
    let conn = guard
        .as_mut()
        .ok_or_else(|| anyhow!("zap_projects db connection unavailable"))?;
    f(conn)
}
