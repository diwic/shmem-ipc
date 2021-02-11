use std::time::Duration;
use criterion::*;
use shmem_ipc::sharedring::{Sender, Receiver};

fn setup_one<T: zerocopy::AsBytes + Copy + zerocopy::FromBytes>(chunks: usize) -> (Sender<T>, Receiver<T>) {
    let s: Sender<T> = Sender::new(chunks).unwrap();
    let memfd = s.memfd().as_file().try_clone().unwrap();
    let e = s.empty_signal().try_clone().unwrap();
    let f = s.full_signal().try_clone().unwrap();
    let r: Receiver<T> = Receiver::open(chunks, memfd, e, f).unwrap();
    (s, r)
}

fn sum(x: &[u64]) -> u64 {
    let mut r = 0u64;
    for y in x { r = r.wrapping_add(*y) };
    r
}

fn bench_one<M: measurement::Measurement>(c: &mut BenchmarkGroup<M>, init: &[u64]) {
    let (mut s, mut r) = setup_one::<u64>(2 * init.len());
    let initsum = sum(init);
    c.bench_with_input(BenchmarkId::new("Sharedring", init.len()*8), &(), |b,_| b.iter(|| {
        let mut x = 0;
        let mut ss: u64 = 0;
        while x < init.len() {
            let mut z = 0;
            unsafe {
                s.send_trusted(|p| {
                    z = std::cmp::min(p.len(), init.len() - x);
                    let part = &init[x..(x+z)];
                    p[0..z].copy_from_slice(part);
                    z
                }).unwrap();
                r.receive_trusted(|p| {
                    assert_eq!(p.len(), z);
                    ss = ss.wrapping_add(sum(p));
                    z
                }).unwrap();
            }
            x += z;
        }
        assert_eq!(x, init.len());
        assert_eq!(initsum, ss);
    }));
}

fn bench_one_unixsocket<M: measurement::Measurement>(c: &mut BenchmarkGroup<M>, init: &[u64]) {
    use std::os::unix::net;
    use std::io::{Read, Write};
    use zerocopy::AsBytes;
    let (mut sender, mut receiver) = net::UnixStream::pair().unwrap();
    sender.set_nonblocking(true).unwrap();
    let mut rbuf: Vec<u64> = init.into();
    let initsum = sum(init);
    c.bench_with_input(BenchmarkId::new("UnixSocket", init.len()*8), &(), |b,_| b.iter(|| {
        let mut x = 0;
        let mut ss: u64 = 0;
        while x < init.len() {
            let z = sender.write(black_box(&init[x..]).as_bytes()).unwrap();
            assert_eq!(z % 8, 0);
            let z2 = z/8;
            receiver.read_exact(rbuf[0..z2].as_bytes_mut()).unwrap();
            ss = ss.wrapping_add(sum(&rbuf[0..z2]));
            x += z2;
        }
        assert_eq!(x, init.len());
        assert_eq!(initsum, ss);
    }));
}

pub fn criterion_benchmark(c: &mut Criterion) {
    let mut v = vec![5, 6, 7, 8];
    let plot_config = PlotConfiguration::default().summary_scale(AxisScale::Logarithmic);
    let mut group = c.benchmark_group("Sharedring vs unix sockets");
    group.plot_config(plot_config);
    // If we're in a hurry
    group.warm_up_time(Duration::from_millis(500));
    group.sample_size(40);
    group.measurement_time(Duration::from_millis(2500));

    loop {
        bench_one(&mut group, &v);
        bench_one_unixsocket(&mut group, &v);
        if v.len() > 1024*1024 { return; }
        v.extend_from_slice(&v.clone());
    }
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
