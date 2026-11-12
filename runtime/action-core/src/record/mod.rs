//! Recording pipeline: bounded queues, frame distributor, and the
//! independent video / IMU writers. Capture threads only produce; they
//! never write files. See architecture §5.

pub mod distributor;
pub mod index;
pub mod writer;
