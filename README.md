[![crates.io](https://img.shields.io/crates/v/shmem-ipc.svg)](https://crates.io/crates/shmem-ipc)
[![API documentation](https://docs.rs/shmem-ipc/badge.svg)](https://docs.rs/shmem-ipc)
[![license](https://img.shields.io/crates/l/shmem-ipc.svg)](https://crates.io/crates/shmem-ipc)

Communication between processes using shared memory.

When is this crate useful?
--------------------------

 * When the data transferred is large, and
 * performance or latency is crucial, and
 * you run Linux.

A typical use case could be audio/video streaming.

Don't need maximum performance and minimal latency, and want a serialization protocol built-in?
Try [D-Bus](https://docs.rs/dbus/).

Also, a [unix socket](https://doc.rust-lang.org/std/os/unix/net/struct.UnixStream.html)
is easier to set up and is not that much slower (see benchmark below).

As for Linux, this crate uses memfd sealing to ensure safety between untrusted processes,
and ringbuffer signaling is done using eventfd for best performance.
These two features are Linux only.

Getting started
---------------

You probably want to start in the `sharedring` module, which sets up a lock-free ringbuffer
between untrusted processes. Another useful function is `mem::oneshot` for a scenario where
you write data once and make it available for reading afterwards. The `mem` and `ringbuf`
modules contain building blocks that might be useful in other use cases.

The downside of using memfd based shared memory is that you need to set it up
by transferring file descriptors, using some other way of communication.
Using [D-Bus](https://docs.rs/dbus/) would be the standard way of doing that -
it's also possible using [unix sockets](https://crates.io/crates/uds).

There is also a client/server example in the `examples` directory that can help you get started.
Enjoy!

Benchmark
---------

[![Sharedring vs unix sockets](https://github.com/diwic/shmem-ipc/blob/master/lines.svg)](https://github.com/diwic/shmem-ipc/blob/master/lines.svg)

License
-------

The code is Apache 2.0 / MIT dual licensed. Any code submitted in Pull Requests, discussions or
issues is assumed to have this license, unless explicitly stated otherwise.
