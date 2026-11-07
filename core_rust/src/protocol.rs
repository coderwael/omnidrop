//! Task 1: Binary Wire Protocol & Data Models Specification
//!
//! Defines the 38-byte fixed binary frame header, packet types, CRC32C header integrity,
//! dynamic chunk sizing rules (4MB - 16MB), and serializable manifest models.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

/// Magic bytes "OMNI" (0x4F4D4E49) identifying an OmniDrop wire frame.
pub const MAGIC_BYTES: u32 = 0x4F4D_4E49;

/// Current binary wire protocol version.
pub const PROTOCOL_VERSION: u8 = 0x01;

/// Fixed binary header size in bytes (38 bytes).
///
/// Layout (Big-Endian / Network Byte Order):
/// - [0..4]   `magic`: u32 (0x4F4D4E49)
/// - [4]      `version`: u8 (0x01)
/// - [5]      `packet_type`: u8 (`PacketType` enum)
/// - [6..8]   `flags`: u16 (Bit 0: Encrypted, Bit 1: Compressed, Bit 2: PriorityRetransmit)
/// - [8..24]  `session_id`: [u8; 16] (UUIDv4 raw bytes)
/// - [24..28] `stream_id`: u32 (Channel / Worker ID)
/// - [28..32] `sequence_Or_chunk_idx`: u32 (Chunk index or sequence number)
/// - [32..36] `payload_len`: u32 (Payload length in bytes, max 16 MiB + 4 KiB metadata)
/// - [36..38] `header_crc16`: u16 (CRC-16/IBM-SDLC over bytes 0..36)
pub const HEADER_SIZE: usize = 38;

/// Minimum dynamic chunk size: 4 MiB.
pub const MIN_CHUNK_SIZE: usize = 4 * 1024 * 1024;

/// Maximum dynamic chunk size: 16 MiB.
pub const MAX_CHUNK_SIZE: usize = 16 * 1024 * 1024;

/// Maximum allowed frame payload size (16 MiB + 64 KiB envelope margin).
pub const MAX_FRAME_PAYLOAD: u32 = (16 * 1024 * 1024) + (64 * 1024);

pub mod flags {
    pub const ENCRYPTED: u16 = 0b0000_0001;
    pub const COMPRESSED: u16 = 0b0000_0010;
    pub const PRIORITY_RETRANSMIT: u16 = 0b0000_0100;
    pub const MULTIPATH_DUPLICATE: u16 = 0b0000_1000;
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("Invalid magic bytes: expected 0x4F4D4E49, got 0x{0:08X}")]
    InvalidMagic(u32),
    #[error("Unsupported protocol version: {0}")]
    UnsupportedVersion(u8),
    #[error("Unknown packet type discriminator: 0x{0:02X}")]
    UnknownPacketType(u8),
    #[error("Header CRC16 mismatch: expected 0x{expected:04X}, computed 0x{actual:04X}")]
    HeaderCrcMismatch { expected: u16, actual: u16 },
    #[error("Payload length {0} exceeds maximum allowed frame size")]
    PayloadTooLarge(u32),
    #[error("Insufficient buffer bytes: required {required}, available {available}")]
    IncompleteFrame { required: usize, available: usize },
    #[error("Serialization error: {0}")]
    Serialization(String),
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PacketType {
    Handshake = 0x01,
    FileManifest = 0x02,
    ChunkData = 0x03,
    ChunkAck = 0x04,
    ResumeRequest = 0x05,
    TransferComplete = 0x06,
    HeartbeatPing = 0x07,
    HeartbeatPong = 0x08,
}

impl TryFrom<u8> for PacketType {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(PacketType::Handshake),
            0x02 => Ok(PacketType::FileManifest),
            0x03 => Ok(PacketType::ChunkData),
            0x04 => Ok(PacketType::ChunkAck),
            0x05 => Ok(PacketType::ResumeRequest),
            0x06 => Ok(PacketType::TransferComplete),
            0x07 => Ok(PacketType::HeartbeatPing),
            0x08 => Ok(PacketType::HeartbeatPong),
            other => Err(ProtocolError::UnknownPacketType(other)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameHeader {
    pub version: u8,
    pub packet_type: PacketType,
    pub flags: u16,
    pub session_id: [u8; 16],
    pub stream_id: u32,
    pub chunk_index: u32,
    pub payload_len: u32,
}

impl FrameHeader {
    pub fn new(
        packet_type: PacketType,
        flags: u16,
        session_id: [u8; 16],
        stream_id: u32,
        chunk_index: u32,
        payload_len: u32,
    ) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            packet_type,
            flags,
            session_id,
            stream_id,
            chunk_index,
            payload_len,
        }
    }

    /// Computes CRC-16-CCITT (0x1021) over the first 36 bytes of the encoded header.
    pub fn compute_crc16(bytes: &[u8]) -> u16 {
        let mut crc: u16 = 0xFFFF;
        for &b in bytes {
            crc ^= (b as u16) << 8;
            for _ in 0..8 {
                if (crc & 0x8000) != 0 {
                    crc = (crc << 1) ^ 0x1021;
                } else {
                    crc <<= 1;
                }
            }
        }
        crc
    }

    pub fn encode(&self, dst: &mut BytesMut) {
        dst.reserve(HEADER_SIZE);
        let start_len = dst.len();
        dst.put_u32(MAGIC_BYTES);
        dst.put_u8(self.version);
        dst.put_u8(self.packet_type as u8);
        dst.put_u16(self.flags);
        dst.put_slice(&self.session_id);
        dst.put_u32(self.stream_id);
        dst.put_u32(self.chunk_index);
        dst.put_u32(self.payload_len);
        let crc = Self::compute_crc16(&dst[start_len..start_len + 36]);
        dst.put_u16(crc);
    }

    pub fn decode(src: &[u8]) -> Result<Self, ProtocolError> {
        if src.len() < HEADER_SIZE {
            return Err(ProtocolError::IncompleteFrame {
                required: HEADER_SIZE,
                available: src.len(),
            });
        }
        let expected_crc = u16::from_be_bytes([src[36], src[37]]);
        let actual_crc = Self::compute_crc16(&src[0..36]);
        if expected_crc != actual_crc {
            return Err(ProtocolError::HeaderCrcMismatch {
                expected: expected_crc,
                actual: actual_crc,
            });
        }

        let mut reader = &src[0..36];
        let magic = reader.get_u32();
        if magic != MAGIC_BYTES {
            return Err(ProtocolError::InvalidMagic(magic));
        }
        let version = reader.get_u8();
        if version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion(version));
        }
        let packet_type = PacketType::try_from(reader.get_u8())?;
        let flags = reader.get_u16();
        let mut session_id = [0u8; 16];
        reader.copy_to_slice(&mut session_id);
        let stream_id = reader.get_u32();
        let chunk_index = reader.get_u32();
        let payload_len = reader.get_u32();

        if payload_len > MAX_FRAME_PAYLOAD {
            return Err(ProtocolError::PayloadTooLarge(payload_len));
        }

        Ok(Self {
            version,
            packet_type,
            flags,
            session_id,
            stream_id,
            chunk_index,
            payload_len,
        })
    }
}

/// Complete Wire Frame combining the 38-byte header with its payload.
#[derive(Debug, Clone)]
pub struct WireFrame {
    pub header: FrameHeader,
    pub payload: Bytes,
}

impl WireFrame {
    pub fn to_bytes(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(HEADER_SIZE + self.payload.len());
        self.header.encode(&mut buf);
        buf.put_slice(&self.payload);
        buf.freeze()
    }

    pub fn from_bytes(buf: &[u8]) -> Result<(Self, usize), ProtocolError> {
        let header = FrameHeader::decode(buf)?;
        let total_len = HEADER_SIZE + (header.payload_len as usize);
        if buf.len() < total_len {
            return Err(ProtocolError::IncompleteFrame {
                required: total_len,
                available: buf.len(),
            });
        }
        let payload = Bytes::copy_from_slice(&buf[HEADER_SIZE..total_len]);
        Ok((Self { header, payload }, total_len))
    }
}

/// Transport tiers prioritized by OmniDrop's channel aggregator.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TransportTier {
    Tier1WifiDirect = 1,
    Tier1WifiAware = 2,
    Tier2QuicMultiStream = 3,
    Tier2MultiTcp = 4,
    Tier3LocalLan = 5,
    Tier4WebRtc = 6,
}

impl fmt::Display for TransportTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportTier::Tier1WifiDirect => write!(f, "Tier 1 (Wi-Fi Direct)"),
            TransportTier::Tier1WifiAware => write!(f, "Tier 1 (Wi-Fi Aware)"),
            TransportTier::Tier2QuicMultiStream => write!(f, "Tier 2 (QUIC Multi-Stream)"),
            TransportTier::Tier2MultiTcp => write!(f, "Tier 2 (Multi-Socket TCP)"),
            TransportTier::Tier3LocalLan => write!(f, "Tier 3 (Local LAN)"),
            TransportTier::Tier4WebRtc => write!(f, "Tier 4 (WebRTC Fallback)"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionEndpoint {
    pub ip: String,
    pub port: u16,
    pub tier: TransportTier,
    pub interface_name: String,
    pub max_streams: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeMessage {
    pub device_id: String,
    pub device_name: String,
    pub platform: String,
    pub x25519_public_key: [u8; 32],
    pub ed25519_public_key: [u8; 32],
    pub signature: Vec<u8>,
    pub nonce: [u8; 12],
    pub endpoints: Vec<ConnectionEndpoint>,
    pub wifi_direct_mac: Option<String>,
    pub wifi_aware_token: Option<String>,
    pub protocol_version: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkMeta {
    pub index: u32,
    pub offset: u64,
    pub length: u32,
    pub sha256: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileManifestEntry {
    pub file_id: String,
    pub relative_path: String,
    pub size_bytes: u64,
    pub modified_unix_ms: u64,
    pub unix_permissions: u32,
    pub is_directory: bool,
    pub sha256_root: [u8; 32],
    pub chunk_size_bytes: u32,
    pub chunks: Vec<ChunkMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionManifest {
    pub session_id: [u8; 16],
    pub sender_device_id: String,
    pub total_transfer_bytes: u64,
    pub default_chunk_size_bytes: u32,
    pub created_unix_ms: u64,
    pub files: Vec<FileManifestEntry>,
}

/// Binary layout inside a `CHUNK_DATA` payload before raw file bytes:
/// - `file_id_len`: u16
/// - `file_id`: UTF-8 bytes
/// - `chunk_index`: u32
/// - `offset`: u64
/// - `sha256`: [u8; 32]
/// - `raw_data`: [u8; length]
#[derive(Debug, Clone)]
pub struct ChunkDataPacket {
    pub file_id: String,
    pub chunk_index: u32,
    pub offset: u64,
    pub sha256: [u8; 32],
    pub data: Bytes,
}

impl ChunkDataPacket {
    pub fn encode(&self) -> Bytes {
        let file_id_bytes = self.file_id.as_bytes();
        let cap = 2 + file_id_bytes.len() + 4 + 8 + 32 + self.data.len();
        let mut buf = BytesMut::with_capacity(cap);
        buf.put_u16(file_id_bytes.len() as u16);
        buf.put_slice(file_id_bytes);
        buf.put_u32(self.chunk_index);
        buf.put_u64(self.offset);
        buf.put_slice(&self.sha256);
        buf.put_slice(&self.data);
        buf.freeze()
    }

    pub fn decode(mut payload: Bytes) -> Result<Self, ProtocolError> {
        if payload.len() < 46 {
            return Err(ProtocolError::IncompleteFrame {
                required: 46,
                available: payload.len(),
            });
        }
        let file_id_len = payload.get_u16() as usize;
        if payload.len() < file_id_len + 44 {
            return Err(ProtocolError::IncompleteFrame {
                required: file_id_len + 44,
                available: payload.len(),
            });
        }
        let file_id_bytes = payload.split_to(file_id_len);
        let file_id = String::from_utf8(file_id_bytes.to_vec())
            .map_err(|e| ProtocolError::Serialization(e.to_string()))?;
        let chunk_index = payload.get_u32();
        let offset = payload.get_u64();
        let mut sha256 = [0u8; 32];
        payload.copy_to_slice(&mut sha256);
        let data = payload;
        Ok(Self {
            file_id,
            chunk_index,
            offset,
            sha256,
            data,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkAckMessage {
    pub session_id: [u8; 16],
    pub file_id: String,
    pub chunk_index: u32,
    pub verified: bool,
    pub error_reason: Option<String>,
    pub receiver_rate_bps: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeRequestMessage {
    pub session_id: [u8; 16],
    pub file_id: String,
    pub completed_bitmap: Vec<u8>,
    pub total_chunks: u32,
    pub active_endpoints: Vec<ConnectionEndpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferCompleteMessage {
    pub session_id: [u8; 16],
    pub success: bool,
    pub total_bytes_verified: u64,
    pub duration_ms: u64,
    pub summary: String,
}

/// Computes optimal chunk size (4 MiB to 16 MiB) based on file size and aggregate link speed.
pub fn compute_optimal_chunk_size(file_size_bytes: u64, estimated_bandwidth_bps: u64) -> usize {
    if file_size_bytes <= 64 * 1024 * 1024 || estimated_bandwidth_bps < 100_000_000 {
        MIN_CHUNK_SIZE // 4 MiB for smaller files or <100 Mbps links
    } else if file_size_bytes <= 1024 * 1024 * 1024 || estimated_bandwidth_bps < 500_000_000 {
        8 * 1024 * 1024 // 8 MiB for medium files / Wi-Fi 5
    } else {
        MAX_CHUNK_SIZE // 16 MiB for multi-gigabit Wi-Fi Direct / Wi-Fi 6 / multi-NIC
    }
}
