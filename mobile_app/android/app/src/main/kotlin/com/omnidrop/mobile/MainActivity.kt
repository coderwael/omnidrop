package com.omnidrop.mobile

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.EventChannel
import io.flutter.plugin.common.MethodChannel
import java.io.File
import java.io.FileOutputStream

/**
 * Task 3B & 4C: Android MainActivity bridging Flutter MethodChannels/EventChannels
 * to BLE Advertising/Scanning, Share Sheet Intents (`ACTION_SEND` / `ACTION_SEND_MULTIPLE`),
 * Floating Drop Bubble Overlay, and Foreground Service updates.
 */
class MainActivity : FlutterActivity() {
    companion object {
        private const val METHOD_CHANNEL = "com.omnidrop.mobile/native_bridge"
        private const val BLE_EVENT_CHANNEL = "com.omnidrop.mobile/ble_events"
    }

    private var bleManager: BleDiscoveryManager? = null
    private var bleEventSink: EventChannel.EventSink? = null
    private val sharedFilePaths = mutableListOf<String>()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        extractIncomingShareIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        extractIncomingShareIntent(intent)
    }

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)

        EventChannel(flutterEngine.dartExecutor.binaryMessenger, BLE_EVENT_CHANNEL)
            .setStreamHandler(object : EventChannel.StreamHandler {
                override fun onListen(arguments: Any?, events: EventChannel.EventSink?) {
                    bleEventSink = events
                }

                override fun onCancel(arguments: Any?) {
                    bleEventSink = null
                }
            })

        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, METHOD_CHANNEL)
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "startBleDiscovery" -> {
                        val deviceId = call.argument<String>("deviceId") ?: "android_node"
                        val deviceName = call.argument<String>("deviceName") ?: Build.MODEL
                        val localIp = call.argument<String>("localIp") ?: "0.0.0.0"
                        val port = call.argument<Int>("port") ?: 48555

                        if (bleManager == null) {
                            bleManager = BleDiscoveryManager(this) { peerMap ->
                                runOnUiThread { bleEventSink?.success(peerMap) }
                            }
                        }
                        bleManager?.startBeaconingAndScanning(deviceId, deviceName, localIp, port)
                        result.success(true)
                    }
                    "stopBleDiscovery" -> {
                        bleManager?.stopAll()
                        result.success(true)
                    }
                    "getSharedFiles" -> {
                        val copy = ArrayList(sharedFilePaths)
                        sharedFilePaths.clear()
                        result.success(copy)
                    }
                    "toggleFloatingBubble" -> {
                        val enable = call.argument<Boolean>("enable") ?: false
                        if (enable) {
                            if (Settings.canDrawOverlays(this)) {
                                startService(Intent(this, FloatingDropBubbleService::class.java))
                                result.success(true)
                            } else {
                                val permIntent = Intent(
                                    Settings.ACTION_MANAGE_OVERLAY_PERMISSION,
                                    Uri.parse("package:$packageName")
                                )
                                startActivity(permIntent)
                                result.success(false)
                            }
                        } else {
                            stopService(Intent(this, FloatingDropBubbleService::class.java))
                            result.success(true)
                        }
                    }
                    "updateForegroundMetrics" -> {
                        val srvIntent = Intent(this, OmniDropForegroundService::class.java).apply {
                            action = OmniDropForegroundService.ACTION_UPDATE
                            putExtra("speed_mbps", call.argument<Double>("speedMbps") ?: 0.0)
                            putExtra("completed_chunks", call.argument<Int>("completedChunks") ?: 0)
                            putExtra("total_chunks", call.argument<Int>("totalChunks") ?: 1)
                            putExtra("active_channels", call.argument<Int>("activeChannels") ?: 1)
                            putExtra("active_tier", call.argument<String>("activeTier") ?: "Tier 2")
                        }
                        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                            startForegroundService(srvIntent)
                        } else {
                            startService(srvIntent)
                        }
                        result.success(true)
                    }
                    else -> result.notImplemented()
                }
            }
    }

    private fun extractIncomingShareIntent(intent: Intent?) {
        if (intent == null) return
        when (intent.action) {
            Intent.ACTION_SEND -> {
                val uri = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
                } else {
                    @Suppress("DEPRECATION")
                    intent.getParcelableExtra(Intent.EXTRA_STREAM)
                }
                uri?.let { resolveUriToFilePath(it)?.let(sharedFilePaths::add) }
            }
            Intent.ACTION_SEND_MULTIPLE -> {
                val uris = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java)
                } else {
                    @Suppress("DEPRECATION")
                    intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM)
                }
                uris?.forEach { u -> resolveUriToFilePath(u)?.let(sharedFilePaths::add) }
            }
        }
    }

    private fun resolveUriToFilePath(uri: Uri): String? {
        return runCatching {
            val fileName = uri.lastPathSegment?.substringAfterLast('/') ?: "shared_payload.bin"
            val cacheFile = File(cacheDir, fileName)
            contentResolver.openInputStream(uri)?.use { input ->
                FileOutputStream(cacheFile).use { output ->
                    input.copyTo(output)
                }
            }
            cacheFile.absolutePath
        }.getOrNull()
    }
}
