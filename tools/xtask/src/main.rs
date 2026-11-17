//! Gate and packaging entry point (`cargo xtask <cmd>`).
//!
//! Commands are defined in `docs/design/interfaces.md` §8 (`arch`, `ipc`,
//! `test`, `docs`, `smoke`, `size`).
//!
//! Every command currently exits non-zero with a message saying so. That is
//! deliberate: an unimplemented gate must never look like a passing one. A
//! silent `exit 0` here would let `cargo xtask arch` be cited as evidence that
//! the dependency rules hold when nothing was checked.

use std::process::ExitCode;

const GATES: &[&str] = &["arch", "ipc", "test", "docs", "smoke", "size"];

fn main() -> ExitCode {
    let command = match std::env::args().nth(1) {
        Some(c) => c,
        None => {
            eprintln!("usage: cargo xtask <{}>", GATES.join("|"));
            return ExitCode::FAILURE;
        }
    };

    if !GATES.contains(&command.as_str()) {
        eprintln!(
            "xtask: unknown gate `{command}`; known gates: {}",
            GATES.join(", ")
        );
        return ExitCode::FAILURE;
    }

    eprintln!(
        "xtask: gate `{command}` is not implemented yet (docs/design/interfaces.md §8).\n\
         This is a FAILURE, not a pass: no check ran.\n\
         Use the gate's manual equivalent until it exists."
    );
    ExitCode::FAILURE
}
