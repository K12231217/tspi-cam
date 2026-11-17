//! Request parameter structs, one per `Call` variant that takes `params`.
//! Every struct is `deny_unknown_fields`: an extra key is `INVALID_PARAMS`
//! naming the field. Optional fields are omitted from the wire, never `null`.

use serde::{Deserialize, Serialize};

/// `record.get` takes the identifier bare (`"params":"20261116-141230-3f2a"`),
/// while `record.delete` insists on an object because deletion is permanent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecId(pub String);

impl RecId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Frame geometry for the encoder (not the sensor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Codec {
    #[serde(rename = "h264")]
    H264,
    #[serde(rename = "h265")]
    H265,
}

/// `hello`: the only call that reports both API versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelloParams {
    pub api_version: u32,
    pub client: String,
}

/// `record.start`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartParams {
    pub encode: Resolution,
    pub codec: Codec,
    pub bitrate_kbps: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// `record.stop`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopParams {
    pub reason: String,
}

/// `record.list`: `before` is a cursor, omitted on the first page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListParams {
    pub limit: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<RecId>,
}

/// `record.delete`: object, not a bare id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteParams {
    pub rec_id: RecId,
    pub confirm: bool,
}

/// `media.subscribe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaSubscribeParams {
    pub max_fps: u8,
}

/// `system.log.tail`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogParams {
    pub lines: u16,
}

/// `system.shutdown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShutdownParams {
    pub confirm: bool,
}

/// Scalar config values only. TOML datetimes leak `$__toml_private_datetime`
/// into JSON and JSON `null` cannot be decoded, so tables/arrays/datetimes are
/// unsupported by design; adding them requires a dedicated method.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConfigValue {
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
}

/// `device.config.set`: writes the user layer only, key by key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigPatch {
    pub path: String,
    pub value: ConfigValue,
}
