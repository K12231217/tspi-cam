//! JSON-RPC 2.0 envelopes. One JSON object per line (`\n`); requests may be
//! concurrent and are matched by `id`. Notifications carry no `id`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::RpcError;
use crate::event::Event;
use crate::method::Call;
use crate::params::RecId;

pub const JSONRPC_VERSION: &str = "2.0";

/// Request id: JSON number or string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Id {
    Number(u64),
    String(String),
}

impl From<u64> for Id {
    fn from(v: u64) -> Self {
        Self::Number(v)
    }
}

impl From<String> for Id {
    fn from(v: String) -> Self {
        Self::String(v)
    }
}

/// Client -> daemon request. Construct with [`Request::new`]; parse with
/// [`Request::decode`] (never `serde_json::from_str`, see the crate docs).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Request {
    pub jsonrpc: &'static str,
    pub id: Id,
    #[serde(flatten)]
    pub call: Call,
}

impl Request {
    pub fn new(id: Id, call: Call) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION,
            id,
            call,
        }
    }

    /// Two-step decode: `method` first (unknown -> `METHOD_NOT_FOUND` with the
    /// name), then that variant's params type (bad shape -> `INVALID_PARAMS`
    /// with the field).
    pub fn decode(line: &str) -> Result<Self, RpcError> {
        let Envelope {
            jsonrpc,
            id,
            method,
            params,
        } = serde_json::from_str(line).map_err(|e| RpcError::parse(e.to_string()))?;
        if jsonrpc != JSONRPC_VERSION {
            return Err(RpcError::parse(format!(
                "unsupported jsonrpc version: {jsonrpc}"
            )));
        }
        let call = match method.as_str() {
            "hello" => Call::Hello(take(params)?),
            "record.status" => {
                no_params(&method, &params)?;
                Call::RecordStatus
            }
            "record.start" => Call::RecordStart(take(params)?),
            "record.stop" => Call::RecordStop(take(params)?),
            "record.list" => Call::RecordList(take(params)?),
            "record.get" => Call::RecordGet(take::<RecId>(params)?),
            "record.delete" => Call::RecordDelete(take(params)?),
            "record.subscribe" => {
                no_params(&method, &params)?;
                Call::RecordSubscribe
            }
            "media.subscribe" => Call::MediaSubscribe(take(params)?),
            "media.unsubscribe" => {
                no_params(&method, &params)?;
                Call::MediaUnsubscribe
            }
            "device.info" => {
                no_params(&method, &params)?;
                Call::DeviceInfo
            }
            "device.config.get" => {
                no_params(&method, &params)?;
                Call::DeviceConfigGet
            }
            "device.config.set" => Call::DeviceConfigSet(take(params)?),
            "device.calibrate.enter" => {
                no_params(&method, &params)?;
                Call::DeviceCalibrateEnter
            }
            "device.calibrate.leave" => {
                no_params(&method, &params)?;
                Call::DeviceCalibrateLeave
            }
            "system.log.tail" => Call::SystemLogTail(take(params)?),
            "system.shutdown" => Call::SystemShutdown(take(params)?),
            other => return Err(RpcError::method_not_found(other)),
        };
        Ok(Self {
            jsonrpc: JSONRPC_VERSION,
            id,
            call,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    jsonrpc: String,
    id: Id,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

fn take<T: serde::de::DeserializeOwned>(params: Option<Value>) -> Result<T, RpcError> {
    let Some(raw) = params else {
        return Err(RpcError::invalid_params("params", "missing params"));
    };
    serde_json::from_value(raw).map_err(|e| {
        let field = field_from(&e).unwrap_or_else(|| "params".to_string());
        RpcError::invalid_params(field, e.to_string())
    })
}

fn no_params(method: &str, params: &Option<Value>) -> Result<(), RpcError> {
    match params {
        None => Ok(()),
        Some(_) => Err(RpcError::invalid_params(
            "params",
            format!("method {method} takes no params"),
        )),
    }
}

/// serde reports `unknown field \`x\`` / `missing field \`x\``; the field name
/// is the part clients branch on.
fn field_from(err: &serde_json::Error) -> Option<String> {
    let message = err.to_string();
    for prefix in ["unknown field `", "missing field `"] {
        if let Some(start) = message.find(prefix) {
            let rest = &message[start + prefix.len()..];
            if let Some(end) = rest.find('`') {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}

/// Server -> client notification (no `id`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    #[serde(flatten)]
    pub event: Event,
}

impl Notification {
    pub fn new(event: Event) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            event,
        }
    }
}

/// Response to a request. `result` and `error` are mutually exclusive
/// (JSON-RPC 2.0 §5); `None` fields are omitted from the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn success(id: Id, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(id: Id, error: RpcError) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: None,
            error: Some(error),
        }
    }
}
