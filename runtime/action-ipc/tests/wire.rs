//! Wire fixtures copied verbatim from `docs/design/ipc.md` §2 and
//! `record-format.md` §2. Comparison is semantic (parsed `Value`), never
//! byte-wise: `serde_json::Map` ordering must not punish a correct encoder.

use action_ipc::error::{ErrorClass, ErrorData, RpcError, StateReason};
use action_ipc::event::{Event, Progress, RecordEvent};
use action_ipc::method::{Call, Lane};
use action_ipc::params::{
    Codec, ConfigValue, DeleteParams, HelloParams, ListParams, MediaSubscribeParams, RecId,
    Resolution, StartParams, StopParams,
};
use action_ipc::results::{
    Deleted, Dropped, EncodeInfo, FrameFormat, FrameHeader, IndexDropped, IndexSummary, Started,
    State, Status, Subscribed, TimestampSemantics,
};
use action_ipc::{Id, Notification, Request, Response, API_VERSION};

fn assert_json_eq(actual: impl serde::Serialize, expected: &str) {
    let actual: serde_json::Value = serde_json::to_value(actual).expect("serializable");
    let expected: serde_json::Value = serde_json::from_str(expected).expect("fixture is JSON");
    assert_eq!(actual, expected);
}

fn rec_id() -> RecId {
    RecId::new("20261116-141230-3f2a")
}

#[test]
fn hello_request() {
    let request = Request::new(
        Id::Number(1),
        Call::Hello(HelloParams {
            api_version: API_VERSION,
            client: "actionctl/0.1.0".into(),
        }),
    );
    assert_json_eq(
        request,
        r#"{"jsonrpc":"2.0","id":1,"method":"hello","params":{"api_version":1,"client":"actionctl/0.1.0"}}"#,
    );
}

#[test]
fn record_start_request() {
    let request = Request::new(
        Id::Number(2),
        Call::RecordStart(StartParams {
            encode: Resolution {
                width: 1920,
                height: 1080,
            },
            codec: Codec::H264,
            bitrate_kbps: 10_000,
            note: None,
        }),
    );
    assert_json_eq(
        request,
        r#"{"jsonrpc":"2.0","id":2,"method":"record.start","params":{"encode":{"width":1920,"height":1080},"codec":"h264","bitrate_kbps":10000}}"#,
    );
}

#[test]
fn record_stop_request() {
    let request = Request::new(
        Id::Number(3),
        Call::RecordStop(StopParams {
            reason: "operator".into(),
        }),
    );
    assert_json_eq(
        request,
        r#"{"jsonrpc":"2.0","id":3,"method":"record.stop","params":{"reason":"operator"}}"#,
    );
}

#[test]
fn record_delete_request() {
    let request = Request::new(
        Id::Number(4),
        Call::RecordDelete(DeleteParams {
            rec_id: rec_id(),
            confirm: true,
        }),
    );
    assert_json_eq(
        request,
        r#"{"jsonrpc":"2.0","id":4,"method":"record.delete","params":{"rec_id":"20261116-141230-3f2a","confirm":true}}"#,
    );
}

#[test]
fn record_subscribe_has_no_params() {
    let request = Request::new(Id::Number(5), Call::RecordSubscribe);
    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(request.call.method(), "record.subscribe");
    assert!(
        value.get("params").is_none(),
        "unit variant must omit params"
    );
    assert_json_eq(
        request,
        r#"{"jsonrpc":"2.0","id":5,"method":"record.subscribe"}"#,
    );
}

#[test]
fn media_subscribe_request_and_ack() {
    let request = Request::new(
        Id::Number(6),
        Call::MediaSubscribe(MediaSubscribeParams { max_fps: 10 }),
    );
    assert_json_eq(
        request,
        r#"{"jsonrpc":"2.0","id":6,"method":"media.subscribe","params":{"max_fps":10}}"#,
    );
    let ack = Response::success(
        Id::Number(6),
        serde_json::json!({"subscribed":true,"width":480,"height":800,"format":"nv12"}),
    );
    assert_json_eq(
        ack,
        r#"{"jsonrpc":"2.0","id":6,"result":{"subscribed":true,"width":480,"height":800,"format":"nv12"}}"#,
    );
}

#[test]
fn record_progress_notification() {
    let notification = Notification::new(Event::RecordProgress(Progress {
        rec_id: rec_id(),
        elapsed_ms: 15_320,
        frames: 459,
        imu_samples: 3_080,
        dropped: Dropped::default(),
    }));
    assert_json_eq(
        notification,
        r#"{"jsonrpc":"2.0","method":"record.progress","params":{"rec_id":"20261116-141230-3f2a","elapsed_ms":15320,"frames":459,"imu_samples":3080,"dropped":{"gap_frames":0,"queue_dropped":0}}}"#,
    );
}

#[test]
fn record_event_notification() {
    let notification = Notification::new(Event::RecordEvent(RecordEvent {
        rec_id: rec_id(),
        class: ErrorClass::Storage,
        code: 1003,
        message: "headroom below 500 MB, stopping".into(),
    }));
    assert_json_eq(
        notification,
        r#"{"jsonrpc":"2.0","method":"record.event","params":{"rec_id":"20261116-141230-3f2a","class":"E_STORAGE","code":1003,"message":"headroom below 500 MB, stopping"}}"#,
    );
}

#[test]
fn record_start_result() {
    let response = Response::success(
        Id::Number(2),
        serde_json::to_value(Started {
            rec_id: rec_id(),
            state: State::Recording,
        })
        .unwrap(),
    );
    assert_json_eq(
        response,
        r#"{"jsonrpc":"2.0","id":2,"result":{"rec_id":"20261116-141230-3f2a","state":"recording"}}"#,
    );
}

#[test]
fn error_response_omits_result() {
    let response = Response::failure(
        Id::Number(2),
        RpcError {
            code: 1001,
            message: "already recording".into(),
            data: Some(ErrorData::Class {
                class: ErrorClass::State,
                reason: Some(StateReason::AlreadyRecording),
            }),
        },
    );
    let value = serde_json::to_value(&response).unwrap();
    assert!(value.get("result").is_none(), "result must be omitted");
    assert_json_eq(
        response,
        r#"{"jsonrpc":"2.0","id":2,"error":{"code":1001,"message":"already recording","data":{"class":"E_STATE","reason":"already_recording"}}}"#,
    );
}

#[test]
fn media_frame_header() {
    let notification = Notification::new(Event::MediaFrame(FrameHeader {
        frame_id: 12_034,
        pts_ns: 88_123_456_789,
        width: 480,
        height: 800,
        format: FrameFormat::Nv12,
        stride: 480,
        payload_len: 576_000,
    }));
    assert_json_eq(
        notification,
        r#"{"jsonrpc":"2.0","method":"media.frame","params":{"frame_id":12034,"pts_ns":88123456789,"width":480,"height":800,"format":"nv12","stride":480,"payload_len":576000}}"#,
    );
}

#[test]
fn decode_unknown_method() {
    let err = Request::decode(r#"{"jsonrpc":"2.0","id":1,"method":"record.nope"}"#).unwrap_err();
    assert_eq!(err.code, action_ipc::error::METHOD_NOT_FOUND);
    assert_eq!(
        err.data,
        Some(ErrorData::Method {
            method: "record.nope".into()
        })
    );
}

#[test]
fn decode_unknown_field() {
    let err = Request::decode(
        r#"{"jsonrpc":"2.0","id":2,"method":"record.stop","params":{"reason":"x","bogus":1}}"#,
    )
    .unwrap_err();
    assert_eq!(err.code, action_ipc::error::INVALID_PARAMS);
    assert_eq!(
        err.data,
        Some(ErrorData::Field {
            field: "bogus".into()
        })
    );
}

#[test]
fn decode_record_get_bare_scalar() {
    let request = Request::decode(
        r#"{"jsonrpc":"2.0","id":4,"method":"record.get","params":"20261116-141230-3f2a"}"#,
    )
    .unwrap();
    assert_eq!(request.call, Call::RecordGet(rec_id()));
    let value = serde_json::to_value(&request).unwrap();
    assert!(value["params"].is_string(), "params must be a scalar");
}

#[test]
fn decode_unit_variant_rejects_params() {
    let err = Request::decode(r#"{"jsonrpc":"2.0","id":4,"method":"record.status","params":{}}"#)
        .unwrap_err();
    assert_eq!(err.code, action_ipc::error::INVALID_PARAMS);
}

#[test]
fn method_and_lane_table() {
    let calls = [
        (Call::RecordStatus, "record.status", Lane::Prompt),
        (Call::RecordSubscribe, "record.subscribe", Lane::Stream),
        (Call::MediaUnsubscribe, "media.unsubscribe", Lane::Prompt),
        (
            Call::DeviceCalibrateEnter,
            "device.calibrate.enter",
            Lane::Operation,
        ),
        (
            Call::RecordList(ListParams {
                limit: 10,
                before: None,
            }),
            "record.list",
            Lane::Slow,
        ),
    ];
    for (call, method, lane) in calls {
        assert_eq!(call.method(), method);
        assert_eq!(call.lane(), lane);
    }

    // Every variant appears exactly once in the method table (no orphan, no
    // duplicate wire name): serialise the enum through a request and count.
    let all = [
        Call::Hello(HelloParams {
            api_version: API_VERSION,
            client: "test".into(),
        }),
        Call::RecordStatus,
        Call::RecordStart(StartParams {
            encode: Resolution {
                width: 1920,
                height: 1080,
            },
            codec: Codec::H265,
            bitrate_kbps: 1,
            note: None,
        }),
        Call::RecordStop(StopParams { reason: "x".into() }),
        Call::RecordList(ListParams {
            limit: 1,
            before: Some(rec_id()),
        }),
        Call::RecordGet(rec_id()),
        Call::RecordDelete(DeleteParams {
            rec_id: rec_id(),
            confirm: false,
        }),
        Call::RecordSubscribe,
        Call::MediaSubscribe(MediaSubscribeParams { max_fps: 10 }),
        Call::MediaUnsubscribe,
        Call::DeviceInfo,
        Call::DeviceConfigGet,
        Call::DeviceConfigSet(action_ipc::params::ConfigPatch {
            path: "camera.fps".into(),
            value: ConfigValue::Integer(30),
        }),
        Call::DeviceCalibrateEnter,
        Call::DeviceCalibrateLeave,
        Call::SystemLogTail(action_ipc::params::LogParams { lines: 50 }),
        Call::SystemShutdown(action_ipc::params::ShutdownParams { confirm: true }),
    ];
    for call in &all {
        let value = serde_json::to_value(call).expect("call serialises");
        assert_eq!(
            value["method"],
            call.method(),
            "#[serde(rename)] drifted from Call::method for {call:?}"
        );
    }

    let mut names: Vec<&str> = all.iter().map(|c| c.method()).collect();
    names.sort_unstable();
    let before = names.len();
    names.dedup();
    assert_eq!(before, names.len(), "duplicate wire method name");
    assert_eq!(names.len(), 17, "every Call variant listed once");
}

#[test]
fn config_value_is_scalar_only() {
    let value: ConfigValue = serde_json::from_str("3").unwrap();
    assert_eq!(value, ConfigValue::Integer(3));
    let value: ConfigValue = serde_json::from_str("\"h264\"").unwrap();
    assert_eq!(value, ConfigValue::String("h264".into()));
    assert!(serde_json::from_str::<ConfigValue>("null").is_err());
    assert!(serde_json::from_str::<ConfigValue>("[1,2]").is_err());
}

#[test]
fn index_summary_matches_record_format() {
    // record-format.md §2, with the optional clock offset omitted (the contract
    // forbids `"x":null` on the wire).
    let fixture = r#"{
      "rec_id": "20261116-141230-3f2a",
      "complete": true,
      "started_mono_ns": 88123456789,
      "ended_mono_ns": 208123456789,
      "duration_ms": 120000,
      "frames": 3600,
      "imu_samples": 24960,
      "dropped": {"gap_frames": 0, "queue_dropped": 0, "imu_dropped": 0},
      "bytes": 481846567,
      "codec": "h264",
      "width": 2592,
      "height": 1944,
      "timestamp_semantics": "sof",
      "fps_actual": 30.045,
      "imu_rate_actual_hz": 208.0,
      "errors": []
    }"#;
    let summary: IndexSummary = serde_json::from_str(fixture).expect("fixture parses");
    assert_eq!(summary.dropped, IndexDropped::default());
    assert_eq!(summary.codec, Codec::H264);
    assert_eq!(summary.timestamp_semantics, TimestampSemantics::Sof);
    assert_json_eq(summary, fixture);
}

#[test]
fn status_omits_optional_fields_and_keeps_dropped_split() {
    let status = Status {
        state: State::Idle,
        rec_id: None,
        elapsed_ms: 0,
        frames: 0,
        imu_samples: 0,
        dropped: Dropped {
            gap_frames: 0,
            queue_dropped: 0,
        },
        disk_free_mb: 1024,
        camera: action_ipc::results::CameraInfo {
            width: 2592,
            height: 1944,
            fps: 30,
        },
        encode: EncodeInfo {
            codec: Codec::H264,
            bitrate_kbps: 10_000,
        },
        last_error: None,
    };
    let value = serde_json::to_value(&status).unwrap();
    assert!(value.get("rec_id").is_none());
    assert!(value.get("last_error").is_none());
    assert_eq!(value["dropped"]["gap_frames"], 0);
    assert_eq!(value["dropped"]["queue_dropped"], 0);
}

#[test]
fn deleted_and_subscribed_shapes() {
    assert_json_eq(Deleted { deleted_bytes: 42 }, r#"{"deleted_bytes":42}"#);
    assert_json_eq(Subscribed { subscribed: true }, r#"{"subscribed":true}"#);
}
