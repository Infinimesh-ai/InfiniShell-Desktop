//! Server-issued timestamps for cloud objects.
//!
//! Zap 不再依赖云端 GraphQL 的 `ServerTimestamp` 标量,这里提供与
//! `app/src/server_time.rs` 一致的本地实现,供 cloud object 模型层复用。

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
pub struct ServerTimestamp(DateTime<Utc>);

impl ServerTimestamp {
    pub fn new(time: DateTime<Utc>) -> Self {
        Self(time)
    }

    pub fn from_unix_timestamp_micros(ms_since_epoch: i64) -> Result<Self> {
        let date_time = DateTime::from_timestamp_micros(ms_since_epoch)
            .ok_or_else(|| anyhow!("Unable to convert microseconds into NaiveDateTime"))?;
        Ok(ServerTimestamp::new(date_time))
    }

    pub fn timestamp_micros(&self) -> i64 {
        self.0.timestamp_micros()
    }

    pub fn utc(&self) -> DateTime<Utc> {
        self.0
    }
}

impl From<DateTime<Utc>> for ServerTimestamp {
    fn from(value: DateTime<Utc>) -> Self {
        ServerTimestamp::new(value)
    }
}
