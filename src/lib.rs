//! Communication between processes using shared memory.
//!
//! This crate uses memfd sealing to ensure safety between untrusted processes,
//! and therefore, it works only on Linux.

pub mod mem;

pub mod ringbuf;
