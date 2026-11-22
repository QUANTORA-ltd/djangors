//! Temporary-file spill helper for large HTTP uploads (`TempFileUpload`).
//!
//! Keeps small uploads in RAM up to a configurable threshold (default 2.5 MiB,
//! matching Django's `FILE_UPLOAD_MAX_MEMORY_SIZE`) and transparently spills
//! larger uploads to a secure temporary file on disk that is automatically
//! deleted on [`Drop`].

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use rand::RngCore;

/// Default in-memory upload threshold: 2.5 MiB (matching Django's `FILE_UPLOAD_MAX_MEMORY_SIZE`).
pub const DEFAULT_MEMORY_THRESHOLD: usize = 2_621_440;

enum UploadStorage {
    Memory(Vec<u8>),
    File { path: PathBuf, file: File },
}

/// An uploaded file buffer that lives in memory up to `threshold` bytes and
/// automatically spills to a secure temporary file on disk when exceeded.
///
/// The temporary file is automatically removed when `TempFileUpload` is dropped.
pub struct TempFileUpload {
    threshold: usize,
    total_len: usize,
    storage: UploadStorage,
}

impl TempFileUpload {
    /// Creates an empty [`TempFileUpload`] that spills to disk once written
    /// bytes exceed `threshold`.
    pub fn new(threshold: usize) -> Self {
        Self {
            threshold,
            total_len: 0,
            storage: UploadStorage::Memory(Vec::new()),
        }
    }

    /// Creates an empty [`TempFileUpload`] with [`DEFAULT_MEMORY_THRESHOLD`] (2.5 MiB).
    pub fn with_default_threshold() -> Self {
        Self::new(DEFAULT_MEMORY_THRESHOLD)
    }

    /// Creates a [`TempFileUpload`] pre-populated with `bytes`, spilling to disk
    /// immediately if `bytes.len() > threshold`.
    pub fn from_bytes(bytes: &[u8], threshold: usize) -> io::Result<Self> {
        let mut upload = Self::new(threshold);
        upload.write_chunk(bytes)?;
        Ok(upload)
    }

    /// Appends a chunk of uploaded bytes, spilling to a temporary file if the
    /// total length would exceed the in-memory threshold.
    pub fn write_chunk(&mut self, chunk: &[u8]) -> io::Result<()> {
        if chunk.is_empty() {
            return Ok(());
        }
        let next_len = self.total_len.saturating_add(chunk.len());
        match &mut self.storage {
            UploadStorage::Memory(buf) => {
                if next_len <= self.threshold {
                    buf.extend_from_slice(chunk);
                    self.total_len = next_len;
                    Ok(())
                } else {
                    let (path, mut file) = create_secure_temp_file()?;
                    if !buf.is_empty() {
                        file.write_all(buf)?;
                    }
                    file.write_all(chunk)?;
                    file.flush()?;
                    self.storage = UploadStorage::File { path, file };
                    self.total_len = next_len;
                    Ok(())
                }
            }
            UploadStorage::File { file, .. } => {
                file.write_all(chunk)?;
                file.flush()?;
                self.total_len = next_len;
                Ok(())
            }
        }
    }

    /// Total number of bytes written to this upload.
    pub fn len(&self) -> usize {
        self.total_len
    }

    /// Returns `true` if no bytes have been written.
    pub fn is_empty(&self) -> bool {
        self.total_len == 0
    }

    /// Returns `true` if this upload has spilled to a temporary file on disk.
    pub fn is_spilled(&self) -> bool {
        matches!(self.storage, UploadStorage::File { .. })
    }

    /// Returns the temporary file path if spilled to disk, or `None` if still in memory.
    pub fn temp_path(&self) -> Option<&Path> {
        match &self.storage {
            UploadStorage::Memory(_) => None,
            UploadStorage::File { path, .. } => Some(path.as_path()),
        }
    }

    /// Reads the entire upload payload into a `Vec<u8>`.
    pub fn read_all(&self) -> io::Result<Vec<u8>> {
        match &self.storage {
            UploadStorage::Memory(buf) => Ok(buf.clone()),
            UploadStorage::File { path, .. } => std::fs::read(path),
        }
    }

    /// Writes the upload contents to `dest` on disk.
    pub fn persist_to(&self, dest: impl AsRef<Path>) -> io::Result<()> {
        match &self.storage {
            UploadStorage::Memory(buf) => std::fs::write(dest, buf),
            UploadStorage::File { path, .. } => {
                std::fs::copy(path, dest)?;
                Ok(())
            }
        }
    }
}

impl Drop for TempFileUpload {
    fn drop(&mut self) {
        if let UploadStorage::File { path, .. } = &self.storage {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn create_secure_temp_file() -> io::Result<(PathBuf, File)> {
    let dir = std::env::temp_dir();
    for _ in 0..16 {
        let mut rand_bytes = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut rand_bytes);
        let suffix: String = rand_bytes.iter().map(|b| format!("{b:02x}")).collect();
        let path = dir.join(format!("djangors_upload_{suffix}.tmp"));

        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }

        match opts.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "failed to create unique temporary upload file",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_upload_stays_in_memory() {
        let upload = TempFileUpload::from_bytes(b"small payload", 1024).unwrap();
        assert!(!upload.is_spilled());
        assert!(upload.temp_path().is_none());
        assert_eq!(upload.len(), 13);
        assert_eq!(upload.read_all().unwrap(), b"small payload");
    }

    #[test]
    fn large_upload_spills_to_disk_and_cleans_up_on_drop() {
        let path: PathBuf;
        {
            let mut upload = TempFileUpload::new(10);
            upload.write_chunk(b"hello ").unwrap();
            assert!(!upload.is_spilled());

            upload.write_chunk(b"world, this spills!").unwrap();
            assert!(upload.is_spilled());
            assert_eq!(upload.len(), 25);
            assert_eq!(upload.read_all().unwrap(), b"hello world, this spills!");

            path = upload.temp_path().unwrap().to_path_buf();
            assert!(path.exists());
        }
        assert!(
            !path.exists(),
            "temporary upload file must be deleted on Drop"
        );
    }
}
