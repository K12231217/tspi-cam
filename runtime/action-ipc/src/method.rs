//! Method names are a permanent wire contract: renaming a variant's
//! `#[serde(rename)]` is a breaking change that requires `API_VERSION + 1`
//! and a `CHANGELOG` `ipc:` entry. Variant names themselves are free.

use serde::{Deserialize, Serialize};

use crate::params::{
    ConfigPatch, DeleteParams, HelloParams, ListParams, LogParams, MediaSubscribeParams, RecId,
    ShutdownParams, StartParams, StopParams,
};

/// How a method is served; drives client timeouts and connection concurrency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// Pure in-memory lookup; 500 ms client timeout.
    Prompt,
    /// Disk I/O; 5 s client timeout.
    Slow,
    /// Changes state, may take tens of seconds; 120 s, serial per connection.
    Operation,
    /// Streams notifications instead of one result.
    Stream,
}

/// Client -> daemon request payload (method + params, adjacently tagged).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum Call {
    #[serde(rename = "hello")]
    Hello(HelloParams),
    #[serde(rename = "record.status")]
    RecordStatus,
    #[serde(rename = "record.start")]
    RecordStart(StartParams),
    #[serde(rename = "record.stop")]
    RecordStop(StopParams),
    #[serde(rename = "record.list")]
    RecordList(ListParams),
    /// Bare scalar params: the identifier needs no wrapper.
    #[serde(rename = "record.get")]
    RecordGet(RecId),
    #[serde(rename = "record.delete")]
    RecordDelete(DeleteParams),
    #[serde(rename = "record.subscribe")]
    RecordSubscribe,
    #[serde(rename = "media.subscribe")]
    MediaSubscribe(MediaSubscribeParams),
    #[serde(rename = "media.unsubscribe")]
    MediaUnsubscribe,
    #[serde(rename = "device.info")]
    DeviceInfo,
    #[serde(rename = "device.config.get")]
    DeviceConfigGet,
    #[serde(rename = "device.config.set")]
    DeviceConfigSet(ConfigPatch),
    #[serde(rename = "device.calibrate.enter")]
    DeviceCalibrateEnter,
    #[serde(rename = "device.calibrate.leave")]
    DeviceCalibrateLeave,
    #[serde(rename = "system.log.tail")]
    SystemLogTail(LogParams),
    #[serde(rename = "system.shutdown")]
    SystemShutdown(ShutdownParams),
}

impl Call {
    /// Wire method name; must stay in lockstep with `#[serde(rename)]`.
    pub const fn method(&self) -> &'static str {
        match self {
            Self::Hello(_) => "hello",
            Self::RecordStatus => "record.status",
            Self::RecordStart(_) => "record.start",
            Self::RecordStop(_) => "record.stop",
            Self::RecordList(_) => "record.list",
            Self::RecordGet(_) => "record.get",
            Self::RecordDelete(_) => "record.delete",
            Self::RecordSubscribe => "record.subscribe",
            Self::MediaSubscribe(_) => "media.subscribe",
            Self::MediaUnsubscribe => "media.unsubscribe",
            Self::DeviceInfo => "device.info",
            Self::DeviceConfigGet => "device.config.get",
            Self::DeviceConfigSet(_) => "device.config.set",
            Self::DeviceCalibrateEnter => "device.calibrate.enter",
            Self::DeviceCalibrateLeave => "device.calibrate.leave",
            Self::SystemLogTail(_) => "system.log.tail",
            Self::SystemShutdown(_) => "system.shutdown",
        }
    }

    /// Lane per `ipc.md` §3.2.
    pub const fn lane(&self) -> Lane {
        match self {
            Self::Hello(_) | Self::RecordStatus | Self::DeviceInfo | Self::DeviceConfigGet => {
                Lane::Prompt
            }
            Self::RecordList(_) | Self::RecordGet(_) | Self::SystemLogTail(_) => Lane::Slow,
            Self::RecordSubscribe | Self::MediaSubscribe(_) => Lane::Stream,
            Self::MediaUnsubscribe => Lane::Prompt,
            Self::RecordStart(_)
            | Self::RecordStop(_)
            | Self::RecordDelete(_)
            | Self::DeviceConfigSet(_)
            | Self::DeviceCalibrateEnter
            | Self::DeviceCalibrateLeave
            | Self::SystemShutdown(_) => Lane::Operation,
        }
    }
}
