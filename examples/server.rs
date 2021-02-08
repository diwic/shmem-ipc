//! This is a example service that sums all f64 coming in through shared memory.
//!
//! For the initial setup, the service advertises a setup function over D-Bus.

use dbus::channel::MatchingReceiver;
use dbus::channel::Sender;
use dbus::Path;
use dbus::Message;
use std::sync::Mutex;
use std::sync::Arc;
use std::thread;
use dbus::MethodErr;
use std::os::unix::io::IntoRawFd;
use dbus::blocking::Connection;
use dbus::arg::OwnedFd;
use dbus_crossroads::{Crossroads};
use std::error::Error;
use shmem_ipc::untrusted::Receiver;

const CAPACITY: usize = 500000;

#[derive(Default)]
struct State {
    sum: Arc<Mutex<f64>>,
}

impl State {
    fn add_receiver(&mut self) -> Result<(u64, OwnedFd, OwnedFd, OwnedFd), Box<dyn Error>> {
        // Create a receiver in shared memory.
        let mut r = Receiver::new(CAPACITY as usize)?;
        // These are just to convert the file descriptors to D-Bus format
        let m = unsafe { OwnedFd::new(r.memfd().as_file().try_clone()?.into_raw_fd()) };
        let e = unsafe { OwnedFd::new(r.empty_signal().try_clone()?.into_raw_fd()) };
        let f = unsafe { OwnedFd::new(r.full_signal().try_clone()?.into_raw_fd()) };
        // In this example, we spawn a thread for every ringbuffer.
        // More complex real-world scenarios might multiplex using non-block frameworks,
        // as well as having a mechanism to detect when a client is gone.
        let sum = self.sum.clone();
        thread::spawn(move || {
            loop {
                r.block_until_readable().unwrap();
                let mut s = 0.0f64;
                r.receive_raw(|ptr: *const f64, count| unsafe {
                    // We now have a slice of [f64; count], but due to the Rust aliasing rules
                    // and the untrusted process restrictions, we cannot convert them into a
                    // Rust slice, so we read the data from the raw pointer directly.
                    for i in 0..count {
                        s += *ptr.offset(i as isize);
                    }
                    *sum.lock().unwrap() += s;
                    count
                }).unwrap();
            }
        });
        Ok((CAPACITY as u64, m, e, f))
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let c = Connection::new_session()?;
    c.request_name("com.example.shmemtest", false, true, false)?;
    let mut cr = Crossroads::new();
    let iface_token = cr.register("com.example.shmemtest", |b| {
        b.method("Setup", (), ("capacity", "memfd", "empty_signal", "full_signal"), |_, state: &mut State, _: ()| {
            state.add_receiver().map_err(|e| {
                println!("{}, {:?}", e, e.source());
                MethodErr::failed("failed to setup shared memory")
            })
        });
        b.signal::<(f64,), _>("Sum", ("sum",));
    });
    cr.insert("/shmemtest", &[iface_token], State::default());
    let acr = Arc::new(Mutex::new(cr));
    let acr_clone = acr.clone();
    c.start_receive(dbus::message::MatchRule::new_method_call(), Box::new(move |msg, conn| {
        acr_clone.lock().unwrap().handle_message(msg, conn).unwrap();
        true
    }));

    loop {
        c.process(std::time::Duration::from_millis(1000))?;
        let mut cr = acr.lock().unwrap();
        let state: &mut State = cr.data_mut(&Path::from("/shmemtest")).unwrap();
        let mut sum = state.sum.lock().unwrap();
        if *sum != 0.0 {
            println!("Sum: {}", sum);
            c.send(Message::new_signal("/shmemtest", "com.example.shmemtest", "Sum").unwrap().append1(*sum)).unwrap();
            *sum = 0.0;
        }
    }
}
