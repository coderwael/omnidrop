//! Task 2C: C-ABI / FFI Bindings (`librust_omnidrop`) for Flutter Mobile & Desktop
//!
//! Exposes thread-safe C functions for initializing the Tokio runtime, starting the
//! mDNS + Multipath receiver daemon, dispatching multi-channel transfers, and polling
//! real-time JSON telemetry snapshots.

use crate::discovery::{CryptoHandshakeEngine, MdnsDiscoveryManager};
use crate::protocol::{ConnectionEndpoint, TransportTier};
use crate::transport::{MultipathReceiverServer, MultipathTransferEngine, TransferProgressEvent};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::runtime::Runtime;

pub struct OmniDropCoreContext {
    runtime: Runtime,
    progress_tx: flume::Sender<TransferProgressEvent>,
    progress_rx: flume::Receiver<TransferProgressEvent>,
    latest_event: Arc<Mutex<Option<TransferProgressEvent>>>,
    mdns: Option<MdnsDiscoveryManager>,
    #[allow(dead_code)]
    crypto: Arc<CryptoHandshakeEngine>,
    shutdown_flag: Arc<AtomicBool>,
}

static GLOBAL_CORE: Lazy<Mutex<Option<OmniDropCoreContext>>> = Lazy::new(|| Mutex::new(None));

/// Initializes the Rust Tokio runtime, cryptographic identity, mDNS discovery, and receiver daemon.
/// Returns 0 on success, negative code on error.
#[no_mangle]
pub extern "C" fn omnidrop_core_init(
    device_id_ptr: *const c_char,
    device_name_ptr: *const c_char,
    platform_ptr: *const c_char,
    listen_port: u16,
    output_dir_ptr: *const c_char,
) -> c_int {
    let device_id = unsafe { cstr_to_string(device_id_ptr) }.unwrap_or_else(|| "omni_node".into());
    let device_name =
        unsafe { cstr_to_string(device_name_ptr) }.unwrap_or_else(|| "OmniDrop Device".into());
    let platform = unsafe { cstr_to_string(platform_ptr) }.unwrap_or_else(|| "unknown".into());
    let output_dir =
        unsafe { cstr_to_string(output_dir_ptr) }.unwrap_or_else(|| "./Downloads".into());

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => return -1,
    };

    let (progress_tx, progress_rx) = flume::unbounded();
    let crypto = Arc::new(CryptoHandshakeEngine::new(
        device_id.clone(),
        device_name.clone(),
        platform.clone(),
    ));
    let shutdown_flag = Arc::new(AtomicBool::new(false));

    let mdns = MdnsDiscoveryManager::new().ok();
    if let Some(ref m) = mdns {
        let _ = m.register_local_node(
            &device_id,
            &device_name,
            &platform,
            "0.0.0.0",
            listen_port,
            "x25519_ready",
        );
        let (peer_tx, _peer_rx) = flume::unbounded();
        let _ = m.start_browsing(peer_tx);
    }

    let bind_addr = format!("0.0.0.0:{listen_port}")
        .parse()
        .unwrap_or_else(|_| "0.0.0.0:48555".parse().unwrap());
    let server = MultipathReceiverServer::new(
        bind_addr,
        PathBuf::from(output_dir),
        progress_tx.clone(),
    );
    let sd_clone = shutdown_flag.clone();
    runtime.spawn(async move {
        let _ = server.run_listener(sd_clone).await;
    });

    let latest_event = Arc::new(Mutex::new(None));
    let rx_clone = progress_rx.clone();
    let latest_clone = latest_event.clone();
    runtime.spawn(async move {
        while let Ok(ev) = rx_clone.recv_async().await {
            *latest_clone.lock() = Some(ev);
        }
    });

    *GLOBAL_CORE.lock() = Some(OmniDropCoreContext {
        runtime,
        progress_tx,
        progress_rx,
        latest_event,
        mdns,
        crypto,
        shutdown_flag,
    });

    0
}

/// Dispatches an asynchronous multi-channel file transfer across all discovered peer endpoints.
/// `files_json_ptr` is a JSON array of file path strings: `["/path/a.mp4", "/path/b.zip"]`.
/// `endpoints_json_ptr` is a JSON array of `ConnectionEndpoint` objects.
#[no_mangle]
pub extern "C" fn omnidrop_send_files(
    files_json_ptr: *const c_char,
    endpoints_json_ptr: *const c_char,
) -> c_int {
    let files_json = match unsafe { cstr_to_string(files_json_ptr) } {
        Some(s) => s,
        None => return -2,
    };
    let endpoints_json = match unsafe { cstr_to_string(endpoints_json_ptr) } {
        Some(s) => s,
        None => return -3,
    };

    let file_strings: Vec<String> = match serde_json::from_str(&files_json) {
        Ok(v) => v,
        Err(_) => return -4,
    };
    let endpoints: Vec<ConnectionEndpoint> = match serde_json::from_str(&endpoints_json) {
        Ok(v) => v,
        Err(_) => {
            vec![ConnectionEndpoint {
                ip: "127.0.0.1".into(),
                port: 48555,
                tier: TransportTier::Tier3LocalLan,
                interface_name: "lo0".into(),
                max_streams: 4,
            }]
        }
    };

    let guard = GLOBAL_CORE.lock();
    let ctx = match guard.as_ref() {
        Some(c) => c,
        None => return -5,
    };

    let session_id = *uuid::Uuid::new_v4().as_bytes();
    let paths: Vec<PathBuf> = file_strings.into_iter().map(PathBuf::from).collect();
    let tx = ctx.progress_tx.clone();

    ctx.runtime.spawn(async move {
        let engine = MultipathTransferEngine::new(tx.clone());
        if let Err(err) = engine
            .send_files(session_id, "omnidrop_sender".into(), paths, endpoints)
            .await
        {
            let _ = tx.send(TransferProgressEvent {
                session_id_hex: "error".into(),
                bytes_transferred: 0,
                total_bytes: 0,
                speed_mbps: 0.0,
                active_channels: 0,
                active_tier: "Failed".into(),
                completed_chunks: 0,
                total_chunks: 0,
                is_complete: false,
                error_message: Some(err.to_string()),
            });
        }
    });

    0
}

/// Polls the latest transfer telemetry snapshot as a newly allocated UTF-8 JSON C-string.
/// Caller must free the returned pointer using `omnidrop_free_string`.
#[no_mangle]
pub extern "C" fn omnidrop_poll_progress_json() -> *mut c_char {
    let guard = GLOBAL_CORE.lock();
    let ctx = match guard.as_ref() {
        Some(c) => c,
        None => return std::ptr::null_mut(),
    };

    // Drain any queued events to ensure the latest state is surfaced
    while let Ok(ev) = ctx.progress_rx.try_recv() {
        *ctx.latest_event.lock() = Some(ev);
    }

    let snapshot = ctx.latest_event.lock().clone();
    match snapshot {
        Some(ev) => match serde_json::to_string(&ev) {
            Ok(json) => CString::new(json)
                .map(|c| c.into_raw())
                .unwrap_or(std::ptr::null_mut()),
            Err(_) => std::ptr::null_mut(),
        },
        None => std::ptr::null_mut(),
    }
}

/// Returns a JSON array of currently discovered mDNS peers.
#[no_mangle]
pub extern "C" fn omnidrop_list_discovered_peers_json() -> *mut c_char {
    let guard = GLOBAL_CORE.lock();
    let ctx = match guard.as_ref() {
        Some(c) => c,
        None => return std::ptr::null_mut(),
    };
    let peers = ctx
        .mdns
        .as_ref()
        .map(|m| m.list_peers())
        .unwrap_or_default();
    match serde_json::to_string(&peers) {
        Ok(json) => CString::new(json)
            .map(|c| c.into_raw())
            .unwrap_or(std::ptr::null_mut()),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Frees a C-string allocated by Rust FFI functions.
#[no_mangle]
pub extern "C" fn omnidrop_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            let _ = CString::from_raw(ptr);
        }
    }
}

/// Shuts down the Rust runtime and active listeners cleanly.
#[no_mangle]
pub extern "C" fn omnidrop_core_shutdown() {
    let mut guard = GLOBAL_CORE.lock();
    if let Some(ctx) = guard.take() {
        ctx.shutdown_flag.store(true, Ordering::Relaxed);
    }
}

unsafe fn cstr_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    CStr::from_ptr(ptr).to_str().ok().map(|s| s.to_owned())
}
