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
pub fn mmap_from_memfd(memfd: &mfd::Memfd) -> Result<mmap::Mmap, Error> {
    // The file can be truncated; no safe memory mapping.
    verify_seal(&memfd, mfd::FileSeal::SealShrink)?;
    // The file can be written to; no safe references.
    verify_seal(&memfd, mfd::FileSeal::SealWrite)?;

    let r = unsafe { mmap::MmapOptions::new().map_copy_read_only(memfd.as_file()) }?;
    Ok(r)
}

/// Creates a raw memory map of a memfd, suitable for IPC. It must be writable.
pub fn mmap_raw_from_memfd(memfd: &mfd::Memfd) -> Result<mmap::MmapRaw, Error> {
    // The file can be truncated; no safe memory mapping.
    verify_seal(&memfd, mfd::FileSeal::SealShrink)?;

    // If the file has been sealed as read-only, the below will fail.
    // If the file later is trying to be sealed as read-only, that call will fail and
    // our mapping will remain.

    let r = mmap::MmapOptions::new().map_raw(memfd.as_file())?;
    Ok(r)
}

pub mod ringbuf;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn create_mmap() -> Result<(), Error> {
        let opts = mfd::MemfdOptions::default().allow_sealing(true);
        let memfd = opts.create("test-ro")?;
        memfd.as_file().set_len(16384)?;

        let mmap = mmap_from_memfd(&memfd)?;
        assert!(memfd.seals()?.contains(&mfd::FileSeal::SealShrink));
        assert!(memfd.seals()?.contains(&mfd::FileSeal::SealWrite));
        assert_eq!(mmap.len(), 16384);
        // The memfd is now read-only, cannot create a writable one.
        assert!(mmap_raw_from_memfd(&memfd).is_err());
        Ok(())
    }

    #[test]
    fn create_mmap_raw() -> Result<(), Error> {
        let opts = mfd::MemfdOptions::default().allow_sealing(true);
        let memfd = opts.create("test-raw")?;
        memfd.as_file().set_len(16384)?;
        let mmap_raw = mmap_raw_from_memfd(&memfd)?;
        assert_eq!(mmap_raw.len(), 16384);
        Ok(())
    }

}
