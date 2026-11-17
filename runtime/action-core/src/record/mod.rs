//! Recording pipeline: the `Recorder` that owns the two writer threads.
//!
//! Not implemented yet. The interface is `docs/design/interfaces.md` §7 and the
//! on-disk layout is `docs/design/record-format.md`; both are frozen, so this
//! module starts empty rather than shipping a stub that returns `Ok(())`.
//!
//! Deliberately absent until they exist for real:
//! - a second `index.json` type (`action_ipc::IndexSummary` is the only one),
//! - a frame distributor (`interfaces.md` §3.1 fixes the queue depth at 4 and
//!   has no preview slot; a distributor that says otherwise is a second owner).
