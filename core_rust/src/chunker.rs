//! Task 2A: Zero-Copy Memory-Mapped File Chunker & Sparse File Receiver Assembler
//!
//! Uses `memmap2::Mmap` for zero-copy reading, `rayon` for parallel SHA-256 manifest
//! computation, and random-access sparse file pre-allocation on the receiver.

use crate::protocol::{
    compute_optimal_chunk_size, ChunkDataPacket, ChunkMeta, FileManifestEntry,
};
use bytes::Bytes;
use memmap2::{Mmap, MmapMut, MmapOptions};
use parking_lot::Mutex;
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChunkerError {
    #[error("I/O failure on path {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("Chunk index {0} out of bounds (total chunks: {1})")]
    ChunkOutOfBounds(u32, u32),
    #[error("SHA-256 verification failed for chunk {index}: expected {expected}, got {actual}")]
    HashMismatch {
        index: u32,
        expected: String,
        actual: String,
    },
}

/// Zero-copy reader backed by OS virtual memory mapping (`mmap`).
pub struct MmapFileChunker {
    file_id: String,
    path: PathBuf,
    file_size: u64,
    chunk_size: u32,
    mmap: Option<Mmap>,
    chunks: Vec<ChunkMeta>,
    root_sha256: [u8; 32],
}

impl MmapFileChunker {
    /// Opens a file, memory-maps it without copying into user-space heap buffers,
    /// and computes per-chunk SHA-256 digests in parallel across CPU cores.
    pub fn open(
        path: impl AsRef<Path>,
        relative_path: String,
        file_id: String,
        estimated_bps: u64,
    ) -> Result<(Self, FileManifestEntry), ChunkerError> {
        let path_ref = path.as_ref();
        let file = File::open(path_ref).map_err(|e| ChunkerError::Io {
            path: path_ref.display().to_string(),
            source: e,
        })?;
        let metadata = file.metadata().map_err(|e| ChunkerError::Io {
            path: path_ref.display().to_string(),
            source: e,
        })?;

        let file_size = metadata.len();
        let chunk_size = compute_optimal_chunk_size(file_size, estimated_bps) as u32;
        let modified_unix_ms = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        if file_size == 0 {
            let empty_hash: [u8; 32] = Sha256::digest([]).into();
            let entry = FileManifestEntry {
                file_id: file_id.clone(),
                relative_path,
                size_bytes: 0,
                modified_unix_ms,
                unix_permissions: 0o644,
                is_directory: false,
                sha256_root: empty_hash,
                chunk_size_bytes: chunk_size,
                chunks: Vec::new(),
            };
            return Ok((
                Self {
                    file_id,
                    path: path_ref.to_path_buf(),
                    file_size: 0,
                    chunk_size,
                    mmap: None,
                    chunks: Vec::new(),
                    root_sha256: empty_hash,
                },
                entry,
            ));
        }

        // SAFETY: File is opened read-only and guarded for the lifetime of the transfer session.
        let mmap = unsafe {
            MmapOptions::new().map(&file).map_err(|e| ChunkerError::Io {
                path: path_ref.display().to_string(),
                source: e,
            })?
        };

        let total_chunks = ((file_size + (chunk_size as u64) - 1) / (chunk_size as u64)) as u32;

        // Compute SHA-256 per chunk in parallel using Rayon worker pool
        let chunks: Vec<ChunkMeta> = (0..total_chunks)
            .into_par_iter()
            .map(|idx| {
                let offset = (idx as u64) * (chunk_size as u64);
                let remaining = file_size - offset;
                let length = remaining.min(chunk_size as u64) as u32;
                let slice = &mmap[offset as usize..(offset as usize + length as usize)];
                let digest: [u8; 32] = Sha256::digest(slice).into();
                ChunkMeta {
                    index: idx,
                    offset,
                    length,
                    sha256: digest,
                }
            })
            .collect();

        // Compute Merkle-like root hash over concatenated chunk hashes
        let mut root_hasher = Sha256::new();
        for c in &chunks {
            root_hasher.update(c.sha256);
        }
        let root_sha256: [u8; 32] = root_hasher.finalize().into();

        let entry = FileManifestEntry {
            file_id: file_id.clone(),
            relative_path,
            size_bytes: file_size,
            modified_unix_ms,
            unix_permissions: 0o644,
            is_directory: false,
            sha256_root: root_sha256,
            chunk_size_bytes: chunk_size,
            chunks: chunks.clone(),
        };

        Ok((
            Self {
                file_id,
                path: path_ref.to_path_buf(),
                file_size,
                chunk_size,
                mmap: Some(mmap),
                chunks,
                root_sha256,
            },
            entry,
        ))
    }

    /// Reads a chunk slice directly from the memory-mapped region and wraps it in a `ChunkDataPacket`.
    pub fn read_chunk_packet(&self, chunk_index: u32) -> Result<ChunkDataPacket, ChunkerError> {
        let meta = self
            .chunks
            .get(chunk_index as usize)
            .ok_or(ChunkerError::ChunkOutOfBounds(
                chunk_index,
                self.chunks.len() as u32,
            ))?;

        let slice = match &self.mmap {
            Some(m) => {
                let start = meta.offset as usize;
                let end = start + (meta.length as usize);
                Bytes::copy_from_slice(&m[start..end])
            }
            None => Bytes::new(),
        };

        Ok(ChunkDataPacket {
            file_id: self.file_id.clone(),
            chunk_index,
            offset: meta.offset,
            sha256: meta.sha256,
            data: slice,
        })
    }

    pub fn total_chunks(&self) -> u32 {
        self.chunks.len() as u32
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn root_sha256(&self) -> [u8; 32] {
        self.root_sha256
    }

    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    pub fn chunk_size(&self) -> u32 {
        self.chunk_size
    }
}

/// Sparse file receiver that pre-allocates the exact target file size and writes
/// verified chunks out-of-order across concurrent transport threads.
pub struct SparseFileAssembler {
    file_id: String,
    target_path: PathBuf,
    file_size: u64,
    total_chunks: u32,
    mmap_mut: Option<Arc<Mutex<MmapMut>>>,
    received_bitmap: Mutex<Vec<u8>>,
    verified_chunks_count: Mutex<u32>,
}

impl SparseFileAssembler {
    pub fn create(
        output_dir: impl AsRef<Path>,
        manifest_entry: &FileManifestEntry,
        deduplicate: bool,
    ) -> Result<Self, ChunkerError> {
        let base_dir = output_dir.as_ref();
        let mut target_path = base_dir.join(&manifest_entry.relative_path);

        if deduplicate && target_path.exists() {
            let stem = target_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("received_file")
                .to_string();
            let ext = target_path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{e}"))
                .unwrap_or_default();
            let parent = target_path.parent().unwrap_or(base_dir).to_path_buf();
            let mut counter = 1u32;
            while target_path.exists() {
                target_path = parent.join(format!("{stem} ({counter}){ext}"));
                counter += 1;
            }
        }

        if let Some(parent) = target_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ChunkerError::Io {
                path: parent.display().to_string(),
                source: e,
            })?;
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&target_path)
            .map_err(|e| ChunkerError::Io {
                path: target_path.display().to_string(),
                source: e,
            })?;

        file.set_len(manifest_entry.size_bytes)
            .map_err(|e| ChunkerError::Io {
                path: target_path.display().to_string(),
                source: e,
            })?;

        let mmap_mut = if manifest_entry.size_bytes > 0 {
            let m = unsafe {
                MmapOptions::new()
                    .map_mut(&file)
                    .map_err(|e| ChunkerError::Io {
                        path: target_path.display().to_string(),
                        source: e,
                    })?
            };
            Some(Arc::new(Mutex::new(m)))
        } else {
            None
        };

        let total_chunks = manifest_entry.chunks.len() as u32;
        let bitmap_bytes = ((total_chunks + 7) / 8) as usize;

        Ok(Self {
            file_id: manifest_entry.file_id.clone(),
            target_path,
            file_size: manifest_entry.size_bytes,
            total_chunks,
            mmap_mut,
            received_bitmap: Mutex::new(vec![0u8; bitmap_bytes]),
            verified_chunks_count: Mutex::new(0),
        })
    }

    /// Verifies the chunk's SHA-256 digest and writes it at its exact byte offset.
    pub fn verify_and_write_chunk(&self, packet: &ChunkDataPacket) -> Result<bool, ChunkerError> {
        if packet.chunk_index >= self.total_chunks {
            return Err(ChunkerError::ChunkOutOfBounds(
                packet.chunk_index,
                self.total_chunks,
            ));
        }

        // Fast check if already verified (idempotent for multipath duplicate tail chunks)
        {
            let bmp = self.received_bitmap.lock();
            let byte_idx = (packet.chunk_index / 8) as usize;
            let bit_mask = 1u8 << (packet.chunk_index % 8);
            if (bmp[byte_idx] & bit_mask) != 0 {
                return Ok(self.is_complete());
            }
        }

        // Streaming SHA-256 integrity check BEFORE committing to mapped disk
        let computed: [u8; 32] = Sha256::digest(&packet.data).into();
        if computed != packet.sha256 {
            return Err(ChunkerError::HashMismatch {
                index: packet.chunk_index,
                expected: hex_encode(&packet.sha256),
                actual: hex_encode(&computed),
            });
        }

        if let Some(mmap_arc) = &self.mmap_mut {
            let mut mmap = mmap_arc.lock();
            let start = packet.offset as usize;
            let end = start + packet.data.len();
            mmap[start..end].copy_from_slice(&packet.data);
        }

        let mut bmp = self.received_bitmap.lock();
        let byte_idx = (packet.chunk_index / 8) as usize;
        let bit_mask = 1u8 << (packet.chunk_index % 8);
        if (bmp[byte_idx] & bit_mask) == 0 {
            bmp[byte_idx] |= bit_mask;
            let mut count = self.verified_chunks_count.lock();
            *count += 1;
        }

        Ok(self.is_complete())
    }

    pub fn flush(&self) -> Result<(), ChunkerError> {
        if let Some(mmap_arc) = &self.mmap_mut {
            let mmap = mmap_arc.lock();
            mmap.flush().map_err(|e| ChunkerError::Io {
                path: self.target_path.display().to_string(),
                source: e,
            })?;
        }
        Ok(())
    }

    pub fn is_complete(&self) -> bool {
        *self.verified_chunks_count.lock() >= self.total_chunks
    }

    pub fn bitmap_snapshot(&self) -> Vec<u8> {
        self.received_bitmap.lock().clone()
    }

    pub fn missing_chunk_indices(&self) -> Vec<u32> {
        let bmp = self.received_bitmap.lock();
        let mut missing = Vec::new();
        for idx in 0..self.total_chunks {
            let byte_idx = (idx / 8) as usize;
            let bit_mask = 1u8 << (idx % 8);
            if (bmp[byte_idx] & bit_mask) == 0 {
                missing.push(idx);
            }
        }
        missing
    }

    pub fn target_path(&self) -> &Path {
        &self.target_path
    }

    pub fn file_id(&self) -> &str {
        &self.file_id
    }

    pub fn file_size(&self) -> u64 {
        self.file_size
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}
