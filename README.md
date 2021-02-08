[![crates.io](https://img.shields.io/crates/v/shmem-ipc.svg)](https://crates.io/crates/shmem-ipc)
[![API documentation](https://docs.rs/shmem-ipc/badge.svg)](https://docs.rs/shmem-ipc)
[![license](https://img.shields.io/crates/l/shmem-ipc.svg)](https://crates.io/crates/shmem-ipc)

Communication between processes using shared memory.

This crate uses memfd sealing to ensure safety between untrusted processes,
and therefore, it works only on Linux.

You might want to start in the `sharedring` module, which sets up a lock-free ringbuffer
between untrusted processes. Another useful function is `mem::oneshot` for a scenario where
you write data once and make it available for reading afterwards. The `mem` and `ringbuf`
contain building blocks that might be useful in other use cases.

There is also a client/server example in the `examples` directory that can help you get started.
Enjoy!
