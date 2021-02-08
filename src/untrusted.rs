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

fn page_size() -> usize {
    unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize }
}

fn round_to_page_size<T>(capacity: usize) -> usize {
    let bytes = crate::ringbuf::channel_bufsize::<T>(capacity);
    let ps = page_size();
    let m = bytes % ps;
    if m == 0 { bytes } else { bytes + ps - m }
}

fn eventfd() -> Result<File, std::io::Error> {
    let x = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC) };
    if x == -1 { Err(std::io::Error::last_os_error()) }
    else { Ok(unsafe { File::from_raw_fd(x) }) }
}

impl Inner {
    fn new<T>(capacity: usize) -> Result<Self, Error> {
        let bytes = round_to_page_size::<T>(capacity);
        let opts = crate::mem::mfd::MemfdOptions::default().allow_sealing(true).close_on_exec(true);
        let memfd = opts.create(std::any::type_name::<T>())?;
        memfd.as_file().set_len(bytes as u64)?;

        let empty_signal = eventfd()?;
        let full_signal = eventfd()?;
        let mmap = crate::mem::raw_memfd(&memfd)?;
        Ok(Self { mmap, memfd, empty_signal, full_signal })
    }

    fn open<T>(capacity: usize, file: File, empty_signal: File, full_signal: File) -> Result<Self, Error> {
        let bytes = round_to_page_size::<T>(capacity);
        let memfd = memfd::Memfd::try_from_file(file).map_err(|_| std::io::Error::last_os_error())?;
        let mmap = crate::mem::raw_memfd(&memfd)?;
        if mmap.len() < bytes { Err(crate::ringbuf::Error::BufTooSmall)? };
        Ok(Self { mmap, memfd, empty_signal, full_signal })
    }
}

pub struct Sender<T>(Inner, crate::ringbuf::Sender<T>);

impl<T: Copy + zerocopy::AsBytes> Sender<T> {
    /// Sets up a new ringbuffer and returns the sender half.
    pub fn new(capacity: usize) -> Result<Self, Error> {
        let inner = Inner::new::<T>(capacity)?;
        let ringbuf = unsafe { crate::ringbuf::Sender::attach(inner.mmap.as_ptr(), inner.mmap.len())? };
        Ok(Self(inner, ringbuf))
    }

    /// Attaches to a ringbuffer set up by the receiving side.
    pub fn open(capacity: usize, memfd: File, empty_signal: File, full_signal: File) -> Result<Self, Error> {
        let inner = Inner::open::<T>(capacity, memfd, empty_signal, full_signal)?;
        let ringbuf = unsafe { crate::ringbuf::Sender::attach(inner.mmap.as_ptr(), inner.mmap.len())? };
        Ok(Self(inner, ringbuf))
    }

    /// Low-level access to the ringbuffer.
    ///
    /// Note that writing directly using these methods will not trigger a signal for the receiving side
    /// to wake up.
    pub fn sender_mut(&mut self) -> &mut crate::ringbuf::Sender<T> { &mut self.1 }

    /// The file descriptor for the shared memory area
    pub fn memfd(&self) -> &memfd::Memfd { &self.0.memfd }
    /// The file descriptor written to when the receiving side should wake up
    pub fn empty_signal(&self) -> &File { &self.0.empty_signal }
    /// The file descriptor to register notification for in your favorite non-blocking framework (tokio, async-std etc).
    ///
    /// It is written to by the receiving side when the buffer is no longer full.
    pub fn full_signal(&self) -> &File { &self.0.full_signal }

    /// Sends one or more items through the ringbuffer.
    ///
    /// Because this is a ringbuffer between untrusted processes we can never create references to
    /// the data, so we have to resort to raw pointers.
    /// The closure receives a (ptr, count) pair which can be written to using e g `std::ptr::write`,
    /// and returns the number of items written to that memory area.
    /// If the buffer is full, the closure is not called. If there is more data that could be written
    /// (e g in another part of the ringbuffer), that is indicated in the returned `Status` struct.
    pub fn send_raw<F: FnOnce(*mut T, usize) -> usize>(&mut self, f: F) -> Result<Status, Error> {
        let status = self.sender_mut().send(f)?;
        if status.signal { self.empty_signal().write(&1u64.to_ne_bytes())?; }
        Ok(status)
    }

    /// For blocking scenarios, blocks until the channel is writable.
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
    /// Sets up a new ringbuffer and returns the receiver half.
    pub fn new(capacity: usize) -> Result<Self, Error> {
        let inner = Inner::new::<T>(capacity)?;
        let ringbuf = unsafe { crate::ringbuf::Receiver::attach(inner.mmap.as_ptr(), inner.mmap.len())? };
        Ok(Self(inner, ringbuf))
    }

    /// Attaches to a ringbuffer set up by the sending side.
    pub fn open(capacity: usize, memfd: File, empty_signal: File, full_signal: File) -> Result<Self, Error> {
        let inner = Inner::open::<T>(capacity, memfd, empty_signal, full_signal)?;
        let ringbuf = unsafe { crate::ringbuf::Receiver::attach(inner.mmap.as_ptr(), inner.mmap.len())? };
        Ok(Self(inner, ringbuf))
    }

    /// Low-level access to the ringbuffer.
    ///
    /// Note that reading directly using these methods will not trigger a signal for the sending side
    /// to wake up.
    pub fn receiver_mut(&mut self) -> &mut crate::ringbuf::Receiver<T> { &mut self.1 }
    /// The file descriptor for the shared memory area
    pub fn memfd(&self) -> &memfd::Memfd { &self.0.memfd }
    /// The file descriptor to register notification for in your favorite non-blocking framework (tokio, async-std etc).
    ///
    /// It is written to by the sending side when the buffer is no longer empty.
    pub fn empty_signal(&self) -> &File { &self.0.empty_signal }
    /// The file descriptor written to when the sending side should wake up
    pub fn full_signal(&self) -> &File { &self.0.full_signal }

    /// Receives data from the ringbuffer.
    ///
    /// Because this is a ringbuffer between untrusted processes we can never create references to
    /// the data, so we have to resort to raw pointers.
    /// The closure receives a (ptr, count) pair which can be read from using e g `std::ptr::read`,
    /// and returns the number of items that can be dropped from the ringbuffer.
    /// If the buffer is empty, the closure is not called. If there is more data that could be read
    /// (e g in another part of the ringbuffer), that is indicated in the returned `Status` struct.
    pub fn receive_raw<F: FnOnce(*const T, usize) -> usize>(&mut self, f: F) -> Result<Status, Error> {
        let status = self.receiver_mut().recv(f)?;
        if status.signal { self.full_signal().write(&1u64.to_ne_bytes())?; }
        Ok(status)
    }

    /// For blocking scenarios, blocks until the channel is readable.
    pub fn block_until_readable(&mut self) -> Result<Status, Error> {
        loop {
            let s = self.receiver_mut().read_count()?;
            if s > 0 { return Ok(Status { remaining: s, signal: false })};
            let mut b = [0u8; 8];
            self.empty_signal().read(&mut b)?;
        }
    }
}

#[test]
fn simple() {
    let mut s: Sender<i32> = Sender::new(1000).unwrap();
    assert!(s.sender_mut().write_count().unwrap() >= 1000);
    let memfd = s.memfd().as_file().try_clone().unwrap();
    let e = s.empty_signal().try_clone().unwrap();
    let f = s.full_signal().try_clone().unwrap();
    let mut r: Receiver<i32> = Receiver::open(1000, memfd, e, f).unwrap();
    assert_eq!(r.receiver_mut().read_count().unwrap(), 0);
}
