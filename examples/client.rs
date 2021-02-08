//! Sends a lot of f64 values over shared memory to the server every second.

use dbus::blocking::{Connection, Proxy};
use dbus::arg::OwnedFd;
use std::error::Error;
use std::thread::sleep;
use shmem_ipc::sharedring::Sender;
use std::time::Duration;
use std::fs::File;
use std::os::unix::io::FromRawFd;

fn main() -> Result<(), Box<dyn Error>> {
    // Setup a D-Bus connection and call the Setup method of the server.
    let c = Connection::new_session()?;
    let proxy = Proxy::new("com.example.shmemtest", "/shmemtest", Duration::from_millis(3000), &c);
    let (capacity, memfd, empty_signal, full_signal): (u64, OwnedFd, OwnedFd, OwnedFd) =
        proxy.method_call("com.example.shmemtest", "Setup", ())?;

    // Convert the file descriptors and setup the ringbuffer.
    let memfd = unsafe { File::from_raw_fd(memfd.into_fd()) };
    let empty_signal = unsafe { File::from_raw_fd(empty_signal.into_fd()) };
    let full_signal = unsafe { File::from_raw_fd(full_signal.into_fd()) };
    let mut r = Sender::open(capacity as usize, memfd, empty_signal, full_signal)?;
    let mut items = 100000;
    loop {
        let item = 1.0f64 / (items as f64);
        r.send_raw(|p: *mut f64, mut count| unsafe {
            // We now have a slice of [f64; count], but due to the Rust aliasing rules
            // and the untrusted process restrictions, we cannot convert them into a
            // Rust slice, so we write the data through the raw pointer directly.
            if items < count { count = items };
            for i in 0..count {
                *p.offset(i as isize) = item;
            }
            println!("Sending {} items of {}, in total {}", count, item, (count as f64) * item);
            count
        }).unwrap();
        items += 100000;
        sleep(Duration::from_millis(1000));
    }
}
