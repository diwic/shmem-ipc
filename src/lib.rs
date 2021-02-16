//! Communication between processes using shared memory.
//!
//! This crate uses memfd sealing to ensure safety between untrusted processes,
//! and therefore, it works only on Linux.
//!
//! You might want to start in the `sharedring` module, which sets up a lock-free ringbuffer
//! between untrusted processes. Another useful function is `mem::write_once` for a scenario where
//! you write data once and make it available for reading afterwards. The `mem` and `ringbuf`
//! contain building blocks that might be useful in other use cases.
//!
//! There is also a client/server example in the `examples` directory that can help you get started.
//! Enjoy!

pub mod mem;

pub mod ringbuf;

pub mod sharedring;


/// Enumeration of errors possible in this library
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Memfd errors")]
    Memfd(#[from] mem::mfd::Error),
    #[error("OS errors")]
    Io(#[from] std::io::Error),
    #[error("Ringbuffer errors")]
    Ringbuf(#[from] ringbuf::Error)
}
