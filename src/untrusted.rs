//! Creates shared memory ring buffers to be used between untrusted processes.
//!
//! The information to be transferred between processes through other means (pipes or D-Bus) is:
//!  * capacity
//!  * memfd file descriptor
//!  * empty signal file descriptor
//!  * full signal file descriptor

use super::Error;
use std::fs::File;
use std::os::unix::io::FromRawFd;
use std::io::{Write, Read};
use crate::ringbuf::Status;

struct Inner {
    mmap: memmap2::MmapRaw,
    memfd: memfd::Memfd,
    empty_signal: File,
    full_signal: File,
}

impl Inner {
    fn new<T>(capacity: usize) -> Result<Self, Error> {
        let bytes = crate::ringbuf::channel_bufsize::<T>(capacity);
        let opts = crate::mem::mfd::MemfdOptions::default();
        let memfd = opts.create(std::any::type_name::<T>())?;
        memfd.as_file().set_len(bytes as u64)?;

        let empty_signal = unsafe { File::from_raw_fd(libc::eventfd(0, 0)) };
        let full_signal = unsafe { File::from_raw_fd(libc::eventfd(0, 0)) };
        let mmap = crate::mem::raw_memfd(&memfd)?;
        Ok(Self { mmap, memfd, empty_signal, full_signal })
    }

    fn open<T>(capacity: usize, file: File, empty_signal: File, full_signal: File) -> Result<Self, Error> {
        let bytes = crate::ringbuf::channel_bufsize::<T>(capacity);
        let memfd = memfd::Memfd::try_from_file(file).map_err(|_| std::io::Error::last_os_error())?;
        let mmap = crate::mem::raw_memfd(&memfd)?;
        if mmap.len() < bytes { Err(crate::ringbuf::Error::BufTooSmall)? };
        Ok(Self { mmap, memfd, empty_signal, full_signal })
    }
}

pub struct Sender<T>(Inner, crate::ringbuf::Sender<T>);

impl<T: Copy + zerocopy::AsBytes> Sender<T> {

    pub fn new(capacity: usize) -> Result<Self, Error> {
        let inner = Inner::new::<T>(capacity)?;
        let ringbuf = unsafe { crate::ringbuf::Sender::attach(inner.mmap.as_ptr(), inner.mmap.len())? };
        Ok(Self(inner, ringbuf))
    }

    pub fn open(capacity: usize, memfd: File, empty_signal: File, full_signal: File) -> Result<Self, Error> {
        let inner = Inner::open::<T>(capacity, memfd, empty_signal, full_signal)?;
        let ringbuf = unsafe { crate::ringbuf::Sender::attach(inner.mmap.as_ptr(), inner.mmap.len())? };
        Ok(Self(inner, ringbuf))
    }

    pub fn sender_mut(&mut self) -> &mut crate::ringbuf::Sender<T> { &mut self.1 }
    pub fn memfd(&self) -> &memfd::Memfd { &self.0.memfd }
    pub fn empty_signal(&self) -> &File { &self.0.empty_signal }
    pub fn full_signal(&self) -> &File { &self.0.full_signal }

    pub fn send_raw<F: FnOnce(*mut T, usize) -> usize>(&mut self, f: F) -> Result<Status, Error> {
        let status = self.sender_mut().send(f)?;
        if status.signal { self.empty_signal().write(&1u64.to_ne_bytes())?; }
        Ok(status)
    }

    pub fn block_until_writable(&mut self) -> Result<Status, Error> {
        loop {
            let s = self.sender_mut().write_count()?;
            if s > 0 { return Ok(Status { remaining: s, signal: false })};
            let mut b = [0u8; 8];
            self.full_signal().read(&mut b)?;
        }
    }
}


pub struct Receiver<T>(Inner, crate::ringbuf::Receiver<T>);

impl<T: Copy + zerocopy::FromBytes> Receiver<T> {

    pub fn new(capacity: usize) -> Result<Self, Error> {
        let inner = Inner::new::<T>(capacity)?;
        let ringbuf = unsafe { crate::ringbuf::Receiver::attach(inner.mmap.as_ptr(), inner.mmap.len())? };
        Ok(Self(inner, ringbuf))
    }

    pub fn open(capacity: usize, memfd: File, empty_signal: File, full_signal: File) -> Result<Self, Error> {
        let inner = Inner::open::<T>(capacity, memfd, empty_signal, full_signal)?;
        let ringbuf = unsafe { crate::ringbuf::Receiver::attach(inner.mmap.as_ptr(), inner.mmap.len())? };
        Ok(Self(inner, ringbuf))
    }

    pub fn receiver_mut(&mut self) -> &mut crate::ringbuf::Receiver<T> { &mut self.1 }
    pub fn memfd(&self) -> &memfd::Memfd { &self.0.memfd }
    pub fn empty_signal(&self) -> &File { &self.0.empty_signal }
    pub fn full_signal(&self) -> &File { &self.0.full_signal }

    pub fn receive_raw<F: FnOnce(*const T, usize) -> usize>(&mut self, f: F) -> Result<Status, Error> {
        let status = self.receiver_mut().recv(f)?;
        if status.signal { self.full_signal().write(&1u64.to_ne_bytes())?; }
        Ok(status)
    }

    pub fn block_until_readable(&mut self) -> Result<Status, Error> {
        loop {
            let s = self.receiver_mut().read_count()?;
            if s > 0 { return Ok(Status { remaining: s, signal: false })};
            let mut b = [0u8; 8];
            self.empty_signal().read(&mut b)?;
        }
    }
}
