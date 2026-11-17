//! `actionctl`: thin client for `actiond`. Builds one typed `Call` per
//! subcommand; the socket client lands together with the daemon's IPC server,
//! so today the NDJSON line is printed instead of sent.

use action_ipc::params::{Codec, Resolution, StartParams, StopParams};
use action_ipc::{Call, Id, Request};

fn usage() -> ! {
    eprintln!("usage: actionctl <status|start|stop|show-config>");
    std::process::exit(2);
}

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_else(|| usage());
    let call = match arg.as_str() {
        "status" => Call::RecordStatus,
        "start" => Call::RecordStart(StartParams {
            encode: Resolution {
                width: 1920,
                height: 1080,
            },
            codec: Codec::H264,
            bitrate_kbps: 10_000,
            note: None,
        }),
        "stop" => Call::RecordStop(StopParams {
            reason: "operator".to_string(),
        }),
        "show-config" => Call::DeviceConfigGet,
        _ => usage(),
    };
    let request = Request::new(Id::Number(1), call);
    println!(
        "{}",
        serde_json::to_string(&request).expect("request is serializable")
    );
}
