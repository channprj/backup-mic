use std::{
    fmt::Write as _,
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

use sha2::{Digest, Sha256};

use crate::error::CoreError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDigest {
    pub size: u64,
    pub sha256: String,
}

pub const HASH_BUFFER_BYTES: usize = 1024 * 1024;

pub fn hash_file(path: &Path) -> Result<FileDigest, CoreError> {
    let file = File::open(path).map_err(CoreError::CopyFailed)?;
    hash_reader(BufReader::with_capacity(HASH_BUFFER_BYTES, file), |_| {})
}

pub fn hash_reader(
    reader: impl Read,
    mut on_bytes: impl FnMut(u64),
) -> Result<FileDigest, CoreError> {
    hash_reader_checked(reader, |bytes| {
        on_bytes(bytes);
        Ok(())
    })
}

pub fn hash_reader_checked(
    mut reader: impl Read,
    mut on_bytes: impl FnMut(u64) -> Result<(), CoreError>,
) -> Result<FileDigest, CoreError> {
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = vec![0_u8; HASH_BUFFER_BYTES];
    loop {
        let read = reader.read(&mut buffer).map_err(CoreError::CopyFailed)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        let read = u64::try_from(read).map_err(|_| CoreError::InvalidRequest)?;
        size = size.checked_add(read).ok_or(CoreError::InvalidRequest)?;
        on_bytes(read)?;
    }
    let mut sha256 = String::with_capacity(64);
    for byte in hasher.finalize() {
        write!(&mut sha256, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(FileDigest { size, sha256 })
}
