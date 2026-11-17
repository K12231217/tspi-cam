//! Error codes, classes and `reason` values. Codes are part of the wire
//! contract: they are written into `record.event` and `index.json.errors`,
//! never only logged.

use serde::{Deserialize, Serialize};

/// Wire code for a frame that is not valid JSON (`PARSE`).
pub const PARSE: i32 = -32700;
/// Wire code for an unknown method; `data.method` names it.
pub const METHOD_NOT_FOUND: i32 = -32601;
/// Wire code for a bad parameter shape; `data.field` names it.
pub const INVALID_PARAMS: i32 = -32602;

/// Failure class shared by `record.event` and `index.json.errors`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorClass {
    #[serde(rename = "E_STATE")]
    State,
    #[serde(rename = "E_HW")]
    Hardware,
    #[serde(rename = "E_STORAGE")]
    Storage,
    #[serde(rename = "E_CFG")]
    Config,
    #[serde(rename = "E_IPC")]
    Ipc,
    #[serde(rename = "E_CALIB")]
    Calibration,
    #[serde(rename = "E_ENC_LIFECYCLE")]
    EncLifecycle,
}

impl ErrorClass {
    /// Numeric code that accompanies this class on the wire.
    pub const fn code(self) -> i32 {
        match self {
            Self::State => 1001,
            Self::Hardware => 1002,
            Self::Storage => 1003,
            Self::Config => 1004,
            Self::Ipc => 1005,
            Self::Calibration => 1006,
            Self::EncLifecycle => 1007,
        }
    }
}

/// Fixed `E_STATE.data.reason` set; clients branch on this, never on
/// `message` text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StateReason {
    #[serde(rename = "already_recording")]
    AlreadyRecording,
    #[serde(rename = "not_recording")]
    NotRecording,
    #[serde(rename = "busy_stopping")]
    BusyStopping,
    #[serde(rename = "recording")]
    Recording,
    #[serde(rename = "fault")]
    Fault,
}

/// JSON-RPC error object. `data` presence depends on the code:
/// method/field problems name the method/field, `E_STATE` carries the reason.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ErrorData>,
}

/// Structured `error.data`. Untagged: exactly one shape matches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ErrorData {
    /// `METHOD_NOT_FOUND`: the offending method name.
    Method { method: String },
    /// `INVALID_PARAMS`: the offending field name.
    Field { field: String },
    /// `E_STATE` and friends: class plus, for state errors, the reason.
    Class {
        class: ErrorClass,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<StateReason>,
    },
}

impl RpcError {
    pub fn parse(message: impl Into<String>) -> Self {
        Self {
            code: PARSE,
            message: message.into(),
            data: None,
        }
    }

    pub fn method_not_found(method: impl Into<String>) -> Self {
        let method = method.into();
        Self {
            code: METHOD_NOT_FOUND,
            message: format!("method not found: {method}"),
            data: Some(ErrorData::Method { method }),
        }
    }

    pub fn invalid_params(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: INVALID_PARAMS,
            message: message.into(),
            data: Some(ErrorData::Field {
                field: field.into(),
            }),
        }
    }

    pub fn class(class: ErrorClass, message: impl Into<String>) -> Self {
        Self {
            code: class.code(),
            message: message.into(),
            data: Some(ErrorData::Class {
                class,
                reason: None,
            }),
        }
    }

    pub fn state(reason: StateReason, message: impl Into<String>) -> Self {
        Self {
            code: ErrorClass::State.code(),
            message: message.into(),
            data: Some(ErrorData::Class {
                class: ErrorClass::State,
                reason: Some(reason),
            }),
        }
    }
}
