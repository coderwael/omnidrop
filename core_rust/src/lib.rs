//! OmniDrop Core Native Library (`librust_omnidrop`)
//!
//! Exports protocol models, zero-copy mmap chunking, multipath QUIC/TCP transport,
//! mDNS + ECDH discovery, and C-ABI FFI bindings for Flutter Android & Desktop.

pub mod chunker;
pub mod discovery;
pub mod ffi;
pub mod protocol;
pub mod transport;

pub use chunker::{ChunkerError, MmapFileChunker, SparseFileAssembler};
pub use discovery::{CryptoHandshakeEngine, DiscoveredPeer, MdnsDiscoveryManager};
pub use protocol::{
    ChunkAckMessage, ChunkDataPacket, ConnectionEndpoint, FrameHeader, HandshakeMessage,
    PacketType, ResumeRequestMessage, SessionManifest, TransferCompleteMessage, TransportTier,
    WireFrame,
};
pub use transport::{MultipathReceiverServer, MultipathTransferEngine, TransferProgressEvent};
