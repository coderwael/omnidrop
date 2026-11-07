//! Task 3A: Cross-Platform mDNS (`_omnidrop._tcp.local.`) & ECDH Handshake Signaling
//!
//! Broadcasts local device presence via Multicast DNS and performs authenticated
//! X25519 Ephemeral Diffie-Hellman + Ed25519 identity signature verification.

use crate::protocol::{ConnectionEndpoint, HandshakeMessage, TransportTier};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use hkdf::Hkdf;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use parking_lot::RwLock;
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use x25519_dalek::{PublicKey as X25519Public, StaticSecret as X25519Secret};

pub const OMNIDROP_MDNS_SERVICE_TYPE: &str = "_omnidrop._tcp.local.";

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("mDNS daemon error: {0}")]
    Mdns(String),
    #[error("Cryptographic signature verification failed")]
    SignatureInvalid,
    #[error("AEAD encryption/decryption error")]
    CipherError,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiscoveredPeer {
    pub device_id: String,
    pub device_name: String,
    pub platform: String,
    pub endpoints: Vec<ConnectionEndpoint>,
    pub rssi: Option<i32>,
    pub verified: bool,
}

/// Cryptographic Identity & ECDH Session Manager.
pub struct CryptoHandshakeEngine {
    device_id: String,
    device_name: String,
    platform: String,
    ed25519_signing: SigningKey,
    x25519_secret: X25519Secret,
    x25519_public: X25519Public,
}

impl CryptoHandshakeEngine {
    pub fn new(device_id: String, device_name: String, platform: String) -> Self {
        let mut csprng = OsRng;
        let ed25519_signing = SigningKey::generate(&mut csprng);
        let x25519_secret = X25519Secret::random_from_rng(&mut csprng);
        let x25519_public = X25519Public::from(&x25519_secret);

        Self {
            device_id,
            device_name,
            platform,
            ed25519_signing,
            x25519_secret,
            x25519_public,
        }
    }

    /// Generates a signed `HandshakeMessage` containing local endpoints and public keys.
    pub fn build_handshake(
        &self,
        endpoints: Vec<ConnectionEndpoint>,
        wifi_direct_mac: Option<String>,
        wifi_aware_token: Option<String>,
    ) -> HandshakeMessage {
        let mut nonce = [0u8; 12];
        OsRng.fill_bytes(&mut nonce);

        let mut signed_transcript = Vec::with_capacity(44 + self.device_id.len());
        signed_transcript.extend_from_slice(self.x25519_public.as_bytes());
        signed_transcript.extend_from_slice(&nonce);
        signed_transcript.extend_from_slice(self.device_id.as_bytes());

        let signature: Signature = self.ed25519_signing.sign(&signed_transcript);

        HandshakeMessage {
            device_id: self.device_id.clone(),
            device_name: self.device_name.clone(),
            platform: self.platform.clone(),
            x25519_public_key: *self.x25519_public.as_bytes(),
            ed25519_public_key: self.ed25519_signing.verifying_key().to_bytes(),
            signature: signature.to_bytes().to_vec(),
            nonce,
            endpoints,
            wifi_direct_mac,
            wifi_aware_token,
            protocol_version: 1,
        }
    }

    /// Verifies the peer's Ed25519 signature over their ephemeral X25519 key and derives
    /// a 256-bit symmetric session key via HKDF-SHA256.
    pub fn verify_and_derive_session_key(
        &self,
        peer_msg: &HandshakeMessage,
    ) -> Result<[u8; 32], DiscoveryError> {
        let verifying_key = VerifyingKey::from_bytes(&peer_msg.ed25519_public_key)
            .map_err(|_| DiscoveryError::SignatureInvalid)?;
        let sig_bytes: [u8; 64] = peer_msg
            .signature
            .clone()
            .try_into()
            .map_err(|_| DiscoveryError::SignatureInvalid)?;
        let signature = Signature::from_bytes(&sig_bytes);

        let mut transcript = Vec::with_capacity(44 + peer_msg.device_id.len());
        transcript.extend_from_slice(&peer_msg.x25519_public_key);
        transcript.extend_from_slice(&peer_msg.nonce);
        transcript.extend_from_slice(peer_msg.device_id.as_bytes());

        verifying_key
            .verify(&transcript, &signature)
            .map_err(|_| DiscoveryError::SignatureInvalid)?;

        let peer_x25519 = X25519Public::from(peer_msg.x25519_public_key);
        let shared_secret = self.x25519_secret.diffie_hellman(&peer_x25519);

        let hk = Hkdf::<Sha256>::new(Some(&peer_msg.nonce), shared_secret.as_bytes());
        let mut okm = [0u8; 32];
        hk.expand(b"OMNIDROP-V1-SESSION-KEY", &mut okm)
            .map_err(|_| DiscoveryError::CipherError)?;
        Ok(okm)
    }

    pub fn encrypt_frame(
        session_key: &[u8; 32],
        nonce_bytes: &[u8; 12],
        plaintext: &[u8],
    ) -> Result<Vec<u8>, DiscoveryError> {
        let cipher = ChaCha20Poly1305::new(session_key.into());
        let nonce = Nonce::from_slice(nonce_bytes);
        cipher
            .encrypt(nonce, plaintext)
            .map_err(|_| DiscoveryError::CipherError)
    }
}

/// Cross-Platform mDNS Service Advertiser and Browser.
pub struct MdnsDiscoveryManager {
    daemon: ServiceDaemon,
    peers: Arc<RwLock<HashMap<String, DiscoveredPeer>>>,
}

impl MdnsDiscoveryManager {
    pub fn new() -> Result<Self, DiscoveryError> {
        let daemon = ServiceDaemon::new().map_err(|e| DiscoveryError::Mdns(e.to_string()))?;
        Ok(Self {
            daemon,
            peers: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub fn register_local_node(
        &self,
        device_id: &str,
        device_name: &str,
        platform: &str,
        local_ip: &str,
        listen_port: u16,
        pubkey_hex: &str,
    ) -> Result<(), DiscoveryError> {
        let host_name = format!("{device_id}.local.");
        let properties = [
            ("id", device_id),
            ("name", device_name),
            ("platform", platform),
            ("pk", pubkey_hex),
            ("ver", "1"),
        ];

        let service_info = ServiceInfo::new(
            OMNIDROP_MDNS_SERVICE_TYPE,
            device_id,
            &host_name,
            local_ip,
            listen_port,
            &properties[..],
        )
        .map_err(|e| DiscoveryError::Mdns(e.to_string()))?;

        self.daemon
            .register(service_info)
            .map_err(|e| DiscoveryError::Mdns(e.to_string()))?;
        Ok(())
    }

    pub fn start_browsing(&self, peer_tx: flume::Sender<DiscoveredPeer>) -> Result<(), DiscoveryError> {
        let receiver = self
            .daemon
            .browse(OMNIDROP_MDNS_SERVICE_TYPE)
            .map_err(|e| DiscoveryError::Mdns(e.to_string()))?;
        let peers_map = self.peers.clone();

        std::thread::spawn(move || {
            while let Ok(event) = receiver.recv() {
                if let ServiceEvent::ServiceResolved(info) = event {
                    let device_id = info
                        .get_property_val_str("id")
                        .unwrap_or("unknown_peer")
                        .to_string();
                    let device_name = info
                        .get_property_val_str("name")
                        .unwrap_or("OmniDrop Peer")
                        .to_string();
                    let platform = info
                        .get_property_val_str("platform")
                        .unwrap_or("desktop")
                        .to_string();

                    let port = info.get_port();
                    let endpoints: Vec<ConnectionEndpoint> = info
                        .get_addresses()
                        .iter()
                        .map(|ip| ConnectionEndpoint {
                            ip: ip.to_string(),
                            port,
                            tier: TransportTier::Tier3LocalLan,
                            interface_name: "lan0".into(),
                            max_streams: 4,
                        })
                        .collect();

                    let peer = DiscoveredPeer {
                        device_id: device_id.clone(),
                        device_name,
                        platform,
                        endpoints,
                        rssi: None,
                        verified: true,
                    };
                    peers_map.write().insert(device_id, peer.clone());
                    let _ = peer_tx.send(peer);
                }
            }
        });
        Ok(())
    }

    pub fn list_peers(&self) -> Vec<DiscoveredPeer> {
        self.peers.read().values().cloned().collect()
    }
}
