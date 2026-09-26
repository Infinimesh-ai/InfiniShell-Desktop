//! WinGet portable SQLite 1.0 索引：原索引只读，新索引仅在私有候选目录创建。
//! 合同来自 winget-cli 77ff01a9 的 PortableTable / SQLiteMetadataTable。

use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::os::windows::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use libsqlite3_sys as sql;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, SetFileAttributesW};
use windows::core::PCWSTR;

use super::Error;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub(super) path: PathBuf,
    pub(super) kind: u32,
    pub(super) sha: String,
    pub(super) target: PathBuf,
}

struct Database(*mut sql::sqlite3);
impl Drop for Database {
    fn drop(&mut self) {
        unsafe { sql::sqlite3_close(self.0) };
    }
}
struct Statement(*mut sql::sqlite3_stmt);
impl Drop for Statement {
    fn drop(&mut self) {
        unsafe { sql::sqlite3_finalize(self.0) };
    }
}

impl Database {
    fn open(path: &Path, create: bool) -> Result<Self, Error> {
        if !path.is_absolute() || path.to_str().is_none_or(|path| path.contains('\0')) {
            return Err(Error::SourceChanged);
        }
        for suffix in ["-wal", "-shm", "-journal"] {
            if PathBuf::from(format!("{}{suffix}", path.display()))
                .try_exists()
                .map_err(|_| Error::SourceChanged)?
            {
                return Err(Error::RecoveryRequired);
            }
        }
        let path = CString::new(path.to_str().ok_or(Error::SourceChanged)?)
            .map_err(|_| Error::SourceChanged)?;
        let mut pointer = std::ptr::null_mut();
        let flags = if create {
            sql::SQLITE_OPEN_READWRITE
        } else {
            sql::SQLITE_OPEN_READONLY
        } | sql::SQLITE_OPEN_NOMUTEX
            | sql::SQLITE_OPEN_NOFOLLOW;
        let status =
            unsafe { sql::sqlite3_open_v2(path.as_ptr(), &mut pointer, flags, std::ptr::null()) };
        let database = Self(pointer);
        if status != sql::SQLITE_OK || pointer.is_null() {
            return Err(Error::SourceChanged);
        }
        unsafe {
            sql::sqlite3_limit(pointer, sql::SQLITE_LIMIT_LENGTH, 1024 * 1024);
            sql::sqlite3_limit(pointer, sql::SQLITE_LIMIT_SQL_LENGTH, 16384);
            sql::sqlite3_enable_load_extension(pointer, 0);
        }
        database.rows("PRAGMA trusted_schema=OFF", &[])?;
        database.rows("PRAGMA busy_timeout=0", &[])?;
        if !create {
            database.rows("PRAGMA query_only=ON", &[])?;
        }
        Ok(database)
    }

    // SQL 只来自本文件常量；路径、摘要和元数据全部经参数绑定，绝不执行索引中的 SQL。
    fn rows(&self, query: &str, parameters: &[String]) -> Result<Vec<Vec<String>>, Error> {
        let query = CString::new(query).map_err(|_| Error::SourceChanged)?;
        let mut pointer = std::ptr::null_mut();
        if unsafe {
            sql::sqlite3_prepare_v2(
                self.0,
                query.as_ptr(),
                -1,
                &mut pointer,
                std::ptr::null_mut(),
            )
        } != sql::SQLITE_OK
        {
            return Err(Error::SourceChanged);
        }
        let statement = Statement(pointer);
        for (offset, value) in parameters.iter().enumerate() {
            if value.contains('\0')
                || value.len() > 32768
                || unsafe {
                    sql::sqlite3_bind_text(
                        statement.0,
                        (offset + 1) as i32,
                        value.as_ptr().cast(),
                        value.len() as i32,
                        sql::SQLITE_TRANSIENT(),
                    )
                } != sql::SQLITE_OK
            {
                return Err(Error::SourceChanged);
            }
        }
        let mut rows = Vec::new();
        loop {
            match unsafe { sql::sqlite3_step(statement.0) } {
                sql::SQLITE_DONE => return Ok(rows),
                sql::SQLITE_ROW => {
                    if rows.len() >= 256 {
                        return Err(Error::UnsupportedSource);
                    }
                    let mut row = Vec::new();
                    for column in 0..unsafe { sql::sqlite3_column_count(statement.0) } {
                        let size = unsafe { sql::sqlite3_column_bytes(statement.0, column) };
                        if !(0..=32768).contains(&size) {
                            return Err(Error::SourceChanged);
                        }
                        let text = unsafe { sql::sqlite3_column_text(statement.0, column) };
                        let value = if text.is_null() {
                            String::new()
                        } else {
                            unsafe { CStr::from_ptr(text.cast()) }
                                .to_str()
                                .map_err(|_| Error::SourceChanged)?
                                .to_owned()
                        };
                        if value.len() != size as usize {
                            return Err(Error::SourceChanged);
                        }
                        row.push(value);
                    }
                    rows.push(row);
                }
                _ => return Err(Error::SourceChanged),
            }
        }
    }
}

const PORTABLE: &str = "CREATE TABLE portable(filepath TEXT NOT NULL UNIQUE COLLATE NOCASE,filetype INT64 NOT NULL,sha256 BLOB,symlinktarget TEXT)";
const METADATA: &str =
    "CREATE TABLE metadata(name TEXT PRIMARY KEY NOT NULL,value TEXT NOT NULL) WITHOUT ROWID";
fn normalized(sql: &str) -> String {
    sql.chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '[' | ']' | '"' | '`'))
        .flat_map(char::to_lowercase)
        .collect()
}

pub(super) fn read(path: &Path) -> Result<Vec<Entry>, Error> {
    super::plain_ancestors(path)?;
    let before = super::stamp(path)?;
    if std::fs::metadata(path)
        .map_err(|_| Error::SourceChanged)?
        .len()
        > 1024 * 1024
    {
        return Err(Error::UnsupportedSource);
    }
    let database = Database::open(path, false)?;
    let schema = database.rows("SELECT type,name,sql FROM sqlite_schema ORDER BY name", &[])?;
    if schema.len() != 3
        || schema.iter().any(|row| match row.as_slice() {
            [kind, name, source] if kind == "table" && name == "portable" => {
                normalized(source) != normalized(PORTABLE)
            }
            [kind, name, source] if kind == "table" && name == "metadata" => {
                normalized(source) != normalized(METADATA)
            }
            [kind, name, source] if kind == "index" && name == "sqlite_autoindex_portable_1" => {
                !source.is_empty()
            }
            _ => true,
        })
    {
        return Err(Error::UnsupportedSource);
    }
    let metadata: BTreeMap<_, _> = database
        .rows("SELECT name,value FROM metadata", &[])?
        .into_iter()
        .map(|row| {
            let [key, value]: [String; 2] = row.try_into().map_err(|_| Error::SourceChanged)?;
            Ok((key, value))
        })
        .collect::<Result<_, Error>>()?;
    if metadata.len() != 4
        || metadata.get("majorVersion").map(String::as_str) != Some("1")
        || metadata.get("minorVersion").map(String::as_str) != Some("0")
        || metadata
            .get("databaseIdentifier")
            .is_none_or(|value| Uuid::parse_str(value).is_err())
        || metadata
            .get("lastwritetime")
            .is_none_or(|value| value.parse::<u64>().is_err())
    {
        return Err(Error::UnsupportedSource);
    }
    let mut result = Vec::new();
    for row in database.rows(
        "SELECT filepath,filetype,sha256,symlinktarget FROM portable",
        &[],
    )? {
        let [path, kind, sha, target]: [String; 4] =
            row.try_into().map_err(|_| Error::SourceChanged)?;
        let kind = kind.parse().map_err(|_| Error::SourceChanged)?;
        if !matches!(kind, 1..=4) {
            return Err(Error::UnsupportedSource);
        }
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err(Error::SourceChanged);
        }
        result.push(Entry {
            path,
            kind,
            sha: sha.to_ascii_lowercase(),
            target: PathBuf::from(target),
        });
    }
    drop(database);
    if super::stamp(path)? != before {
        return Err(Error::SourceChanged);
    }
    Ok(result)
}

pub(super) fn write(path: &Path, entries: &[Entry]) -> Result<(), Error> {
    // 先 create_new 确保从不覆盖用户文件；SQLite 不获得 CREATE 权限。
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| Error::PersistenceFailed)?;
    file.sync_all().map_err(|_| Error::PersistenceFailed)?;
    drop(file);
    let database = Database::open(path, true)?;
    database.rows("PRAGMA journal_mode=DELETE", &[])?;
    database.rows("PRAGMA synchronous=FULL", &[])?;
    database.rows("BEGIN IMMEDIATE", &[])?;
    database.rows(PORTABLE, &[])?;
    database.rows(METADATA, &[])?;
    for (name, value) in [
        ("majorVersion", "1".to_owned()),
        ("minorVersion", "0".to_owned()),
        ("databaseIdentifier", Uuid::new_v4().braced().to_string()),
        (
            "lastwritetime",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| Error::PersistenceFailed)?
                .as_secs()
                .to_string(),
        ),
    ] {
        database.rows(
            "INSERT INTO metadata(name,value) VALUES(?1,?2)",
            &[name.to_owned(), value],
        )?;
    }
    for entry in entries {
        database.rows(
            "INSERT INTO portable(filepath,filetype,sha256,symlinktarget) VALUES(?1,?2,?3,?4)",
            &[
                entry.path.to_str().ok_or(Error::SourceChanged)?.to_owned(),
                entry.kind.to_string(),
                entry.sha.clone(),
                entry
                    .target
                    .to_str()
                    .ok_or(Error::SourceChanged)?
                    .to_owned(),
            ],
        )?;
    }
    database.rows("COMMIT", &[])?;
    drop(database);
    // 官方 portable 索引是隐藏文件；不把内部登记文件暴露为普通包资源。
    let path_wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe { SetFileAttributesW(PCWSTR(path_wide.as_ptr()), FILE_ATTRIBUTE_HIDDEN) }
        .map_err(|_| Error::PersistenceFailed)?;
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|_| Error::PersistenceFailed)
}
