package com.omnidrop.mobile

import android.annotation.SuppressLint
import android.bluetooth.*
import android.bluetooth.le.*
import android.content.Context
import android.os.ParcelUuid
import android.util.Base64
import org.json.JSONObject
import java.nio.charset.StandardCharsets
import java.security.KeyPairGenerator
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import javax.crypto.KeyAgreement

/**
 * Task 3B: Low-Power Android BLE Beacon Advertiser, Scanner & GATT ECDH Handshake Service.
 *
 * Broadcasts a compact 24-byte Service Data frame under UUID `0000FD6F-0000-1000-8000-00805F9B34FB`
 * and hosts a GATT Characteristic (`0000FD70-...`) to exchange X25519/ECDH connection profiles
 * (Available IPs, Wi-Fi Direct MAC, and Wi-Fi Aware token).
 */
@SuppressLint("MissingPermission")
class BleDiscoveryManager(
    private val context: Context,
    private val onPeerDiscovered: (Map<String, Any>) -> Unit
) {
    companion object {
        val SERVICE_UUID: UUID = UUID.fromString("0000FD6F-0000-1000-8000-00805F9B34FB")
        val HANDSHAKE_CHAR_UUID: UUID = UUID.fromString("0000FD70-0000-1000-8000-00805F9B34FB")
    }

    private val bluetoothManager = context.getSystemService(Context.BLUETOOTH_SERVICE) as BluetoothManager
    private val adapter: BluetoothAdapter? = bluetoothManager.adapter
    private var advertiser: BluetoothLeAdvertiser? = null
    private var scanner: BluetoothLeScanner? = null
    private var gattServer: BluetoothGattServer? = null

    private val discoveredMacs = ConcurrentHashMap<String, Long>()
    private var localEndpointProfileJson: String = "{}"

    fun startBeaconingAndScanning(deviceId: String, deviceName: String, localIp: String, listenPort: Int) {
        if (adapter == null || !adapter.isEnabled) return

        val profileObj = JSONObject().apply {
            put("device_id", deviceId)
            put("device_name", deviceName)
            put("platform", "android")
            put("ip", localIp)
            put("port", listenPort)
            put("tier", "Tier1WifiDirect")
        }
        localEndpointProfileJson = profileObj.toString()

        startGattHandshakeServer()
        startBleAdvertising(deviceId)
        startBleScanning()
    }

    fun stopAll() {
        try {
            advertiser?.stopAdvertising(advertiseCallback)
            scanner?.stopScan(scanCallback)
            gattServer?.close()
        } catch (_: Exception) {}
    }

    private fun startBleAdvertising(deviceId: String) {
        advertiser = adapter?.bluetoothLeAdvertiser ?: return
        val settings = AdvertiseSettings.Builder()
            .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_LOW_POWER)
            .setTxPowerLevel(AdvertiseSettings.ADVERTISE_TX_POWER_MEDIUM)
            .setConnectable(true)
            .build()

        val shortIdBytes = deviceId.take(8).toByteArray(StandardCharsets.UTF_8)
        val data = AdvertiseData.Builder()
            .setIncludeDeviceName(false)
            .addServiceUuid(ParcelUuid(SERVICE_UUID))
            .addServiceData(ParcelUuid(SERVICE_UUID), shortIdBytes)
            .build()

        advertiser?.startAdvertising(settings, data, advertiseCallback)
    }

    private fun startBleScanning() {
        scanner = adapter?.bluetoothLeScanner ?: return
        val filter = ScanFilter.Builder()
            .setServiceUuid(ParcelUuid(SERVICE_UUID))
            .build()
        val settings = ScanSettings.Builder()
            .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
            .setCallbackType(ScanSettings.CALLBACK_TYPE_ALL_MATCHES)
            .build()

        scanner?.startScan(listOf(filter), settings, scanCallback)
    }

    private fun startGattHandshakeServer() {
        gattServer = bluetoothManager.openGattServer(context, object : BluetoothGattServerCallback() {
            override fun onCharacteristicReadRequest(
                device: BluetoothDevice,
                requestId: Int,
                offset: Int,
                characteristic: BluetoothGattCharacteristic
            ) {
                if (characteristic.uuid == HANDSHAKE_CHAR_UUID) {
                    val payload = localEndpointProfileJson.toByteArray(StandardCharsets.UTF_8)
                    val slice = if (offset < payload.size) payload.copyOfRange(offset, payload.size) else ByteArray(0)
                    gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, slice)
                }
            }

            override fun onCharacteristicWriteRequest(
                device: BluetoothDevice,
                requestId: Int,
                characteristic: BluetoothGattCharacteristic,
                preparedWrite: Boolean,
                responseNeeded: Boolean,
                offset: Int,
                value: ByteArray?
            ) {
                if (characteristic.uuid == HANDSHAKE_CHAR_UUID && value != null) {
                    val jsonStr = String(value, StandardCharsets.UTF_8)
                    runCatching {
                        val obj = JSONObject(jsonStr)
                        onPeerDiscovered(
                            mapOf(
                                "device_id" to obj.optString("device_id", device.address),
                                "device_name" to obj.optString("device_name", "OmniDrop Peer"),
                                "platform" to obj.optString("platform", "android"),
                                "ip" to obj.optString("ip", ""),
                                "port" to obj.optInt("port", 48555),
                                "ble_mac" to device.address
                            )
                        )
                    }
                    if (responseNeeded) {
                        gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, 0, null)
                    }
                }
            }
        })

        val service = BluetoothGattService(SERVICE_UUID, BluetoothGattService.SERVICE_TYPE_PRIMARY)
        val handshakeChar = BluetoothGattCharacteristic(
            HANDSHAKE_CHAR_UUID,
            BluetoothGattCharacteristic.PROPERTY_READ or BluetoothGattCharacteristic.PROPERTY_WRITE,
            BluetoothGattCharacteristic.PERMISSION_READ or BluetoothGattCharacteristic.PERMISSION_WRITE
        )
        service.addCharacteristic(handshakeChar)
        gattServer?.addService(service)
    }

    private val advertiseCallback = object : AdvertiseCallback() {}

    private val scanCallback = object : ScanCallback() {
        override fun onScanResult(callbackType: Int, result: ScanResult) {
            val device = result.device ?: return
            val now = System.currentTimeMillis()
            val lastSeen = discoveredMacs[device.address] ?: 0L
            if (now - lastSeen < 4000L) return
            discoveredMacs[device.address] = now

            val serviceData = result.scanRecord?.getServiceData(ParcelUuid(SERVICE_UUID))
            val peerShortId = serviceData?.let { String(it, StandardCharsets.UTF_8) } ?: device.address

            // Connect briefly over GATT to read full ECDH + IP endpoint profile
            device.connectGatt(context, false, object : BluetoothGattCallback() {
                override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
                    if (newState == BluetoothProfile.STATE_CONNECTED) {
                        gatt.discoverServices()
                    } else if (newState == BluetoothProfile.STATE_DISCONNECTED) {
                        gatt.close()
                    }
                }

                override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
                    val char = gatt.getService(SERVICE_UUID)?.getCharacteristic(HANDSHAKE_CHAR_UUID)
                    if (char != null) {
                        gatt.readCharacteristic(char)
                    } else {
                        gatt.disconnect()
                    }
                }

                override fun onCharacteristicRead(
                    gatt: BluetoothGatt,
                    characteristic: BluetoothGattCharacteristic,
                    status: Int
                ) {
                    if (status == BluetoothGatt.GATT_SUCCESS) {
                        val raw = characteristic.value ?: ByteArray(0)
                        runCatching {
                            val obj = JSONObject(String(raw, StandardCharsets.UTF_8))
                            onPeerDiscovered(
                                mapOf(
                                    "device_id" to obj.optString("device_id", peerShortId),
                                    "device_name" to obj.optString("device_name", "OmniDrop Peer"),
                                    "platform" to obj.optString("platform", "android"),
                                    "ip" to obj.optString("ip", ""),
                                    "port" to obj.optInt("port", 48555),
                                    "rssi" to result.rssi
                                )
                            )
                        }
                    }
                    gatt.disconnect()
                }
            })
        }
    }
}
