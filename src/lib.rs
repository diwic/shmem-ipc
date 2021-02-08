//! Communication between processes using shared memory.
//!
//! This crate uses memfd sealing to ensure safety between untrusted processes,
//! and therefore, it works only on Linux.


pub mod mem;

pub mod ringbuf;

pub mod untrusted;


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
