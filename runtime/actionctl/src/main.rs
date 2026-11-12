//! `actionctl`: CLI for `actiond` (status / start / stop / show-config).
//! Talks to the daemon over the control socket only; never touches
//! hardware. Skeleton: argument parsing only, IPC client lands with the
//! IPC server (M1 A段 step 8).

use action_ipc::control::ControlRequest;

fn usage() -> ! {
    eprintln!("usage: actionctl <status|start|stop|show-config>");
    std::process::exit(2);
}

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_else(|| usage());
    let request = match arg.as_str() {
        "status" | "start" | "stop" => match arg.as_str() {
            "status" => ControlRequest::GetStatus,
            "start" => ControlRequest::StartRecording,
            _ => ControlRequest::StopRecording,
        },
        "show-config" => {
            println!("control socket: /run/tspi-cam/control.sock");
            return;
        }
        _ => usage(),
    };
    // TODO (M1 A段 step 8): send `request` over the control socket and
    // print the response.
    println!("(skeleton) would send: {:?}", request);
}
