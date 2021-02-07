//! This is a fast ringbuffer that tries to avoid memory copies as much as possible.
//! There can be one producer and one consumer, but they can be in different threads
//! i e, they are Send but not Clone.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::mem::size_of;
use std::{cmp, ptr};


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

const CACHE_LINE_SIZE: usize = 64;

/// Use this utility function to figure out how big buffer you need to allocate.
pub fn channel_bufsize<T>(capacity: usize) -> usize { capacity * size_of::<T>() + CACHE_LINE_SIZE }

/// Create a channel (without signaling)
/// Non-allocating - expects a pre-allocated buffer
pub fn channel<T: zerocopy::AsBytes + zerocopy::FromBytes>(buffer: &mut [u8]) -> (Sender<T>, Receiver<T>) {
    let s = Sender::attach(buffer.as_mut_ptr(), buffer.len());
    let r = Receiver::attach(buffer.as_mut_ptr(), buffer.len());
    s.buf.count().store(0, Ordering::Relaxed);
    (s, r)
}

impl<T> Buf<T> {
    #[inline]
    fn count(&self) -> &AtomicUsize { unsafe { &*self.count_ptr }}

    fn attach(data: *mut u8, length: usize) -> Self {
        assert!(length >= CACHE_LINE_SIZE + size_of::<T>(), "Buffer too small");
        assert!(length < isize::MAX as usize, "Buffer too big");
        Self {
            count_ptr: data as _,
            data: unsafe { data.offset(CACHE_LINE_SIZE as isize) } as _,
            length: (length - CACHE_LINE_SIZE) / size_of::<T>(),
        }
    }
}

impl<T: zerocopy::AsBytes> Sender<T> {

    /// Assume a ringbuf is set up at the location.
    ///
    /// A buffer where the first 64 bytes are zero is okay.
    pub fn attach(data: *mut u8, length: usize) -> Self {
        Self { buf: Buf::attach(data, length), index: 0 }
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
    pub fn send<F: FnOnce(*mut T, usize) -> usize>(&mut self, f: F) -> (usize, bool) {
        let cb = self.buf.count().load(Ordering::Acquire);
        let l = self.buf.length;
        assert!(cb <= l);

        let n = {
             let end = self.index + cmp::min(l - self.index, l - cb);
             let slice_start = unsafe { self.buf.data.offset(self.index as isize) };
             let slice_len = end - self.index;

             let n = if slice_len == 0 { 0 } else { f(slice_start, slice_len) };

             assert!(n <= slice_len);
             n
        };

        let c = self.buf.count().fetch_add(n, Ordering::AcqRel);
        self.index = (self.index + n) % l;
        // dbg!("Send: cb = {}, c = {}, l = {}, n = {}", cb, c, l, n);
        (l - c - n, c == 0 && n > 0)
    }

    /// "Safe" version of send. Will call your closure up to "count" times
    /// and depend on RVO to avoid memory copies.
    ///
    /// Returns (free items, was empty) like send does
    pub fn send_foreach<F: FnMut(usize) -> T>(&mut self, count: usize, mut f: F) -> (usize, bool) {
        let mut i = 0;
        self.send(|p, c| {
            while i < c && i < count {
                unsafe { ptr::write(p.offset(i as isize), f(i)) };
                i += 1;
            };
            i
        })
    }

    /// Returns number of items that can be written
    pub fn write_count(&self) -> usize { self.buf.length - self.buf.count().load(Ordering::Relaxed) }
}

impl<T: zerocopy::FromBytes> Receiver<T> {
    /// Returns (remaining items, was full)
    /// The second item is true if the buffer was full but was read from
    /// (this can be used to signal remote side that more data can be written).
    /// f: This closure returns number of items that can be dropped from buffer.
    /// Since this is a ringbuffer, there might be more items to read even if you
    /// read it all during the closure.
    pub fn recv<F: FnOnce(*const T, usize) -> usize>(&mut self, f: F) -> (usize, bool) {
        let cb = self.buf.count().load(Ordering::Acquire);
        let l = self.buf.length;
        assert!(cb <= l);
        let n = {
            let data_start = unsafe { self.buf.data.offset(self.index as isize) };
            let data_len = cmp::min(self.index + cb, l) - self.index;

            let n = if data_len == 0 { 0 } else { f(data_start, data_len) };
            assert!(n <= data_len);
            n
        };

        let c = self.buf.count().fetch_sub(n, Ordering::AcqRel);
        self.index = (self.index + n) % l;
        // dbg!("Recv: cb = {}, c = {}, l = {}, n = {}", cb, c, l, n);
        return (c - n, c >= l && n > 0)
    }

    /// Returns number of items that can be read
    pub fn read_count(&self) -> usize { self.buf.count().load(Ordering::Relaxed) }

    /// Assume a ringbuf is set up at the location.
    ///
    /// A buffer where the first 64 bytes are zero is okay.
    pub fn attach(data: *mut u8, length: usize) -> Self {
        Self { buf: Buf::attach(data, length), index: 0 }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn simple_test() {
        let mut v = vec![10; 100];
        let (mut s, mut r) = super::channel(&mut v);
        // is it empty?
        r.recv(|_,_| panic!());
        s.send(|d, l| {
            assert!(l > 0);
            unsafe { *d = 5u16 };
            1
        });
        r.recv(|d, l| {
            assert_eq!(l, 1);
            assert_eq!(unsafe { *d }, 5);
            0
        });
        r.recv(|d, l| {
            assert_eq!(l, 1);
            assert_eq!(unsafe { *d }, 5);
            1
        });
        r.recv(|_, _| panic!());

        let mut i = 6;
        s.send_foreach(2, |_| { i += 1; i } );
        r.recv(|d, l| {
            assert_eq!(l, 2);
            let x = unsafe { std::ptr::read(d as *const [u16; 2]) };
            assert_eq!(x, [7, 8]);
            2
        });
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
        });
        let mut called = false;
        s.send_foreach(2, |i| {
            assert_eq!(called, false);
            assert_eq!(i, 0);
            called = true;
            10
        });
        s.send(|_, _| panic!());
        r.recv(|_, l| {
            assert_eq!(l, 3);
            0
        });
        s.send(|_, _| panic!());
        r.recv(|d, l| {
            assert_eq!(l, 3);
            assert_eq!([5, 8, 10], unsafe { std::ptr::read(d as *const [u16; 3]) });
            1
        });
        s.send(|d, l| {
            assert_eq!(l, 1);
            unsafe { *d = 1 };
            1
        });
        s.send(|_, _| panic!());
        r.recv(|d, l| {
            assert_eq!(l, 2);
            assert_eq!([8, 10], unsafe { std::ptr::read(d as *const [u16; 2]) });
            2
        });
        r.recv(|d, l| {
            assert_eq!(l, 1);
            assert_eq!(unsafe { *d }, 1);
            1
        });
    }
}
