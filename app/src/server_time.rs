use anyhow::{Result, anyhow};
use chrono::{DateTime, FixedOffset, Utc};
use instant::Instant;
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

// Zap:`Revision` 已统一到 `cloud_objects::cloud_object::Revision`,它内部持有的是
// `cloud_objects::server_time::ServerTimestamp`(与本类型结构一致的另一份实现)。
// 原先 `impl From<Revision> for ServerTimestamp` 挂在被删掉的本地 `Revision` 上,
// 这里在 app 侧补回,保证 `revision.into()` / `.map(Into::into)` 的调用点继续可用。
impl From<cloud_objects::cloud_object::Revision> for ServerTimestamp {
    fn from(revision: cloud_objects::cloud_object::Revision) -> Self {
        ServerTimestamp::new(revision.utc())
    }
}

/// 本地估算的服务端时间。
///
/// Zap 不再请求云端 `/current_time`;启动路径用本地当前时间初始化,调用方仍可
/// 通过该类型获得随单调时钟推进的 wall-clock 时间。
#[derive(Debug, Clone)]
pub struct ServerTime {
    time_at_fetch: DateTime<FixedOffset>,
    fetched_at: Instant,
}

impl ServerTime {
    pub(crate) fn local_now() -> Self {
        Self {
            time_at_fetch: chrono::Utc::now().into(),
            fetched_at: Instant::now(),
        }
    }

    pub(crate) fn current_time(&self) -> DateTime<FixedOffset> {
        let elapsed = chrono::Duration::from_std(self.fetched_at.elapsed())
            .expect("duration should not be bigger than limit");
        self.time_at_fetch + elapsed
    }
}
