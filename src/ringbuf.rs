//! This is a fast ringbuffer that tries to avoid memory copies as much as possible.
//! There can be one producer and one consumer, but they can be in different threads
//! i e, they are Send but not Clone.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::mem::size_of;
use std::{cmp, ptr};

/// Enumeration of errors possible in this library
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Buffer too small")]
    BufTooSmall,
    #[error("Buffer too big")]
    BufTooBig,
    #[error("Buffer unaligned")]
    BufUnaligned,
    #[error("Buffer corrupt or uninitialized")]
    BufCorrupt,
    #[error("Callback read more items than existed in the buffer")]
    CallbackReadTooMuch,
    #[error("Callback wrote more items than available in the buffer")]
    CallbackWroteTooMuch,
}


#[derive(Copy, Clone)]
struct Buf<T> {
    data: *mut T,
    count_ptr: *const AtomicUsize,
    length: usize,
}

unsafe impl<T> Send for Buf<T> {}

pub struct Sender<T> {
    buf: Buf<T>,
    index: usize,
}

pub struct Receiver<T> {
    buf: Buf<T>,
    index: usize,
}

#[derive(Copy, Clone, Debug)]
pub struct Status {
    /// Number of remaining items that can be immediately read/written
    pub remaining: usize,
    /// True if we should signal the remote side to wake up
    pub signal: bool,
}

const CACHE_LINE_SIZE: usize = 64;

/// Use this utility function to figure out how big buffer you need to allocate.
pub fn channel_bufsize<T>(capacity: usize) -> usize { capacity * size_of::<T>() + CACHE_LINE_SIZE }

/// Initializes a ring buffer.
///
/// # Panics
///
/// In case the buffer is too small or too big.
pub fn channel<T: zerocopy::AsBytes + zerocopy::FromBytes + Copy>(buffer: &mut [u8]) -> (Sender<T>, Receiver<T>) {
    let b = unsafe { Buf::attach(buffer.as_mut_ptr(), buffer.len(), true).unwrap() };
    (Sender { buf: b, index: 0}, Receiver { buf: b, index: 0})
}

impl<T> Buf<T> {
    #[inline]
    fn count(&self) -> &AtomicUsize { unsafe { &*self.count_ptr }}

    #[inline]
    fn load_count(&self) -> Result<usize, Error> {
        let x = self.count().load(Ordering::Acquire);
        if x > self.length { Err(Error::BufCorrupt) } else { Ok(x) }
    }

    unsafe fn attach(data: *mut u8, length: usize, init: bool) -> Result<Self, Error> {
        use Error::*;
        if length < CACHE_LINE_SIZE + size_of::<T>() { Err(BufTooSmall)? }
        if length >= isize::MAX as usize { Err(BufTooBig)? }
        let r = Self {
            count_ptr: data as *mut _ as *const AtomicUsize,
            data: data.offset(CACHE_LINE_SIZE as isize) as _,
            length: (length - CACHE_LINE_SIZE) / size_of::<T>(),
        };
        if (r.count_ptr as usize) % std::mem::align_of::<AtomicUsize>() != 0 { Err(BufUnaligned)? }
        if (r.data as usize) % std::mem::align_of::<T>() != 0 { Err(BufUnaligned)? }
        if init {
            r.count().store(0, Ordering::Release);
        } else {
            r.load_count()?;
        }
        Ok(r)
    }
}

impl<T: zerocopy::AsBytes + Copy> Sender<T> {

    /// Assume a ringbuf is set up at the location.
    ///
    /// A buffer where the first 64 bytes are zero is okay.
    ///
    /// # Safety
    ///
    /// You must ensure that "data" points to a readable and writable memory area of "length" bytes.
    pub unsafe fn attach(data: *mut u8, length: usize) -> Result<Self, Error> {
        Ok(Self { buf: Buf::attach(data, length, false)?, index: 0 })
    }

    /// Lowest level "send" function
    ///
    /// Returns (free items, was empty)
    /// The first item is number of items that can be written to the buffer (until it's full).
    /// The second item is true if the buffer was empty but was written to
    /// (this can be used to signal remote side that more data can be read).
    /// f: This closure returns number of items written to the buffer.
    ///
    /// The pointer sent to the closure is an "out" parameter and contains
    /// garbage data on entering the closure. (This cannot safely be a &mut [T] because
    /// the closure might then read from uninitialized memory, even though it shouldn't)
    ///
    /// Since this is a ringbuffer, there might be more items to write even if you
    /// completely fill up during the closure.
    pub fn send<F: FnOnce(*mut T, usize) -> usize>(&mut self, f: F) -> Result<Status, Error> {
        let cb = self.buf.load_count()?;
        let l = self.buf.length;

        let n = {
             let end = self.index + cmp::min(l - self.index, l - cb);
             let slice_start = unsafe { self.buf.data.offset(self.index as isize) };
             let slice_len = end - self.index;

             let n = if slice_len == 0 { 0 } else { f(slice_start, slice_len) };
             if n > slice_len { Err(Error::CallbackWroteTooMuch)? }
             assert!(n <= slice_len);
             n
        };

        let c = self.buf.count().fetch_add(n, Ordering::AcqRel);
        self.index = (self.index + n) % l;
        // dbg!("Send: cb = {}, c = {}, l = {}, n = {}", cb, c, l, n);
        Ok(Status {
            remaining: l - c - n,
            signal: c == 0 && n > 0
        })
    }

    /// "Safe" version of send. Will call your closure up to "count" times
    /// and depend on RVO to avoid memory copies.
    ///
    /// # Panics
    ///
    /// Panics in case the buffer is corrupt.
    pub fn send_foreach<F: FnMut() -> T>(&mut self, mut count: usize, mut f: F) -> Status {
        loop {
            let status = self.send(|p, c| {
                let mut j = 0;
                while j < c && count > 0 {
                    unsafe { ptr::write(p.offset(j as isize), f()) };
                    j += 1;
                    count -= 1;
                };
                j
            }).unwrap();
            if status.remaining == 0 || count == 0 { return status; }
        }
    }

    /// Returns number of items that can be written
    pub fn write_count(&self) -> Result<usize, Error> { Ok(self.buf.length - self.buf.load_count()?) }
}

impl<T: zerocopy::FromBytes + Copy> Receiver<T> {
    /// Returns (remaining items, was full)
    /// The second item is true if the buffer was full but was read from
    /// (this can be used to signal remote side that more data can be written).
    /// f: This closure returns number of items that can be dropped from buffer.
    /// Since this is a ringbuffer, there might be more items to read even if you
    /// read it all during the closure.
    pub fn recv<F: FnOnce(*const T, usize) -> usize>(&mut self, f: F) -> Result<Status, Error> {
        let cb = self.buf.load_count()?;
        let l = self.buf.length;
        let n = {
            let data_start = unsafe { self.buf.data.offset(self.index as isize) };
            let data_len = cmp::min(self.index + cb, l) - self.index;

            let n = if data_len == 0 { 0 } else { f(data_start, data_len) };
            if n > data_len { Err(Error::CallbackReadTooMuch)? }
            n
        };

        let c = self.buf.count().fetch_sub(n, Ordering::AcqRel);
        self.index = (self.index + n) % l;
        // dbg!("Recv: cb = {}, c = {}, l = {}, n = {}", cb, c, l, n);
        return Ok(Status {
            remaining: c - n,
            signal: c >= l && n > 0,
        })
    }

    /// "Safe" version of recv. Will call your closure up to "count" times
    /// and depend on optimisation to avoid memory copies.
    ///
    /// Returns (free items, was empty) like send does
    ///
    /// # Panics
    ///
    /// Panics in case the buffer is corrupt.
    pub fn recv_foreach<F: FnMut(T)>(&mut self, mut count: usize, mut f: F) -> Status {
        loop {
            let status = self.recv(|p, c| {
                let mut j = 0;
                while j < c && count > 0 {
                    f(unsafe { ptr::read(p.offset(j as isize)) });
                    count -= 1;
                    j += 1;
                };
                j
            }).unwrap();
            if status.remaining == 0 || count == 0 { return status; }
        }
    }


    /// Returns number of items that can be read
    pub fn read_count(&self) -> Result<usize, Error> { self.buf.load_count() }

    /// Assume a ringbuf is set up at the location.
    ///
    /// A buffer where the first 64 bytes are zero is okay.
    ///
    /// # Safety
    ///
    /// You must ensure that "data" points to a readable and writable memory area of "length" bytes.
    pub unsafe fn attach(data: *mut u8, length: usize) -> Result<Self, Error> {
        Ok(Self { buf: Buf::attach(data, length, false)?, index: 0 })
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn simple_test() {
        let mut v = vec![10; 100];
        let (mut s, mut r) = super::channel(&mut v);
        // is it empty?
        r.recv(|_,_| panic!()).unwrap();
        s.send(|d, l| {
            assert!(l > 0);
            unsafe { *d = 5u16 };
            1
        }).unwrap();
        r.recv(|d, l| {
            assert_eq!(l, 1);
            assert_eq!(unsafe { *d }, 5);
            0
        }).unwrap();
        r.recv(|d, l| {
            assert_eq!(l, 1);
            assert_eq!(unsafe { *d }, 5);
            1
        }).unwrap();
        r.recv(|_, _| panic!()).unwrap();

        let mut i = 6;
        s.send_foreach(2, || { i += 1; i } );
        r.recv(|d, l| {
            assert_eq!(l, 2);
            let x = unsafe { std::ptr::read(d as *const [u16; 2]) };
            assert_eq!(x, [7, 8]);
            2
        }).unwrap();
    }

    #[test]
    fn full_buf_test() {
        assert_eq!(super::channel_bufsize::<u16>(3), 64+3*2);
        let mut q: Vec<u8> = vec![66; super::channel_bufsize::<u16>(3)];
        let (mut s, mut r): (super::Sender<u16>, super::Receiver<u16>) = super::channel(&mut q);
        s.send(|dd, l| {
            assert_eq!(l, 3);
            unsafe { std::ptr::write(dd as *mut [u16; 3], [5, 8, 9]); }
            2
        }).unwrap();
        let mut called = false;
        s.send_foreach(2, || {
            assert_eq!(called, false);
            called = true;
            10
        });
        s.send(|_, _| panic!()).unwrap();
        r.recv(|_, l| {
            assert_eq!(l, 3);
            0
        }).unwrap();
        s.send(|_, _| panic!()).unwrap();
        r.recv(|d, l| {
            assert_eq!(l, 3);
            assert_eq!([5, 8, 10], unsafe { std::ptr::read(d as *const [u16; 3]) });
            1
        }).unwrap();
        s.send(|d, l| {
            assert_eq!(l, 1);
            unsafe { *d = 1 };
            1
        }).unwrap();
        s.send(|_, _| panic!()).unwrap();
        r.recv(|d, l| {
            assert_eq!(l, 2);
            assert_eq!([8, 10], unsafe { std::ptr::read(d as *const [u16; 2]) });
            2
        }).unwrap();
        let mut called = false;
        r.recv_foreach(56, |d| {
            assert_eq!(called, false);
            called = true;
            assert_eq!(d, 1);
        });
    }
}
