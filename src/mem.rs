/// Reexport the memmap2 crate
pub mod mmap {
    pub use memmap2::*;
}

/// Reexport the memfd crate
pub mod mfd {
    pub use memfd::*;
}

/// Enumeration of errors possible in this library
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Memfd errors")]
    Memfd(#[from] mfd::Error),
    #[error("OS errors")]
    Io(#[from] std::io::Error),
}

fn verify_seal(memfd: &mfd::Memfd, seal: mfd::FileSeal) -> Result<(), Error> {
    let seals = memfd.seals()?;
    if seals.contains(&seal) { return Ok(()) }
    // Try to add the seal.
    memfd.add_seal(seal)?;
    Ok(())
}


/// Creates a memory map of a memfd. The memfd is sealed to be read only.
pub fn read_memfd(memfd: &mfd::Memfd) -> Result<mmap::Mmap, Error> {
    // The file can be truncated; no safe memory mapping.
    verify_seal(&memfd, mfd::FileSeal::SealShrink)?;
    // The file can be written to; no safe references.
    verify_seal(&memfd, mfd::FileSeal::SealWrite)?;

    let r = unsafe { mmap::MmapOptions::new().map_copy_read_only(memfd.as_file()) }?;
    Ok(r)
}

/// Creates a raw memory map of a memfd, suitable for IPC. It must be writable.
pub fn raw_memfd(memfd: &mfd::Memfd) -> Result<mmap::MmapRaw, Error> {
    // The file can be truncated; no safe memory mapping.
    verify_seal(&memfd, mfd::FileSeal::SealShrink)?;

    // If the file has been sealed as read-only, the below will fail.
    // If the file later is trying to be sealed as read-only, that call will fail and
    // our mapping will remain.

    let r = mmap::MmapOptions::new().map_raw(memfd.as_file())?;
    Ok(r)
}

pub fn oneshot<F: FnOnce(&mut[u8])>(size: u64, name: &str, f: F) -> Result<mfd::Memfd, Error> {
    let opts = memfd::MemfdOptions::new().allow_sealing(true).close_on_exec(true);
    let mut h = mfd::SealsHashSet::new();
    h.insert(mfd::FileSeal::SealGrow);
    h.insert(mfd::FileSeal::SealShrink);
    h.insert(mfd::FileSeal::SealSeal);
    h.insert(mfd::FileSeal::SealWrite);

    oneshot_custom(size, name, opts, &h, f)
}

pub fn oneshot_custom<F: FnOnce(&mut[u8])>(size: u64, name: &str, memfd_options: memfd::MemfdOptions, seals: &mfd::SealsHashSet, f: F) -> Result<mfd::Memfd, Error> {
    let memfd = memfd_options.create(name)?;
    // Sets the memory to zeroes.
    memfd.as_file().set_len(size)?;
    // We're the sole owner of the file descriptor, it's safe to create a mutable reference to the data.
    let mut m = unsafe { mmap::MmapMut::map_mut(memfd.as_file())? };
    f(&mut m);
    drop(m);
    if !seals.is_empty() {
        memfd.add_seals(&seals)?;
    }
    Ok(memfd)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn create_mmap() -> Result<(), Error> {
        let opts = mfd::MemfdOptions::default().allow_sealing(true);
        let memfd = opts.create("test-ro")?;
        memfd.as_file().set_len(16384)?;

        let mmap = read_memfd(&memfd)?;
        assert!(memfd.seals()?.contains(&mfd::FileSeal::SealShrink));
        assert!(memfd.seals()?.contains(&mfd::FileSeal::SealWrite));
        assert_eq!(mmap.len(), 16384);
        // The memfd is now read-only, cannot create a writable one.
        assert!(raw_memfd(&memfd).is_err());
        Ok(())
    }

    #[test]
    fn create_mmap_raw() -> Result<(), Error> {
        let opts = mfd::MemfdOptions::default().allow_sealing(true);
        let memfd = opts.create("test-raw")?;
        memfd.as_file().set_len(16384)?;
        let mmap_raw = raw_memfd(&memfd)?;
        assert_eq!(mmap_raw.len(), 16384);
        // The memfd now has a writable mapping, cannot create a read-only one.
        assert!(read_memfd(&memfd).is_err());
        Ok(())
    }

    #[test]
    fn write_then_read() -> Result<(), Error> {
        let m = oneshot(4096, "write_then_read_test", |x| {
            assert_eq!(x.len(), 4096);
            assert_eq!(x[5], 0);
            x[2049] = 100;
        })?;
        let m2 = read_memfd(&m)?;
        assert_eq!(m2[2049], 100);
        assert_eq!(m2[465], 0);
        Ok(())
    }
}
