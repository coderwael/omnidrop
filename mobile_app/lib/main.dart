// Task 4: Flutter Mobile Client (Material 3 Dark Theme, BLE + mDNS Radar, Share Sheet & FFI Engine)
import 'dart:async';
import 'dart:io';
import 'package:file_picker/file_picker.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:path_provider/path_provider.dart';
import 'ffi_bridge.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  runApp(const ProviderScope(child: OmniDropMobileApp()));
}

class TransferTelemetryState {
  final double speedMbps;
  final int bytesTransferred;
  final int totalBytes;
  final int completedChunks;
  final int totalChunks;
  final int activeChannels;
  final String activeTier;
  final bool isComplete;
  final List<Map<String, dynamic>> discoveredPeers;
  final List<String> stagedFiles;
  final bool bubbleEnabled;

  const TransferTelemetryState({
    this.speedMbps = 0.0,
    this.bytesTransferred = 0,
    this.totalBytes = 0,
    this.completedChunks = 0,
    this.totalChunks = 0,
    this.activeChannels = 0,
    this.activeTier = 'Idle • BLE + mDNS Beaconing',
    this.isComplete = false,
    this.discoveredPeers = const [],
    this.stagedFiles = const [],
    this.bubbleEnabled = false,
  });

  TransferTelemetryState copyWith({
    double? speedMbps,
    int? bytesTransferred,
    int? totalBytes,
    int? completedChunks,
    int? totalChunks,
    int? activeChannels,
    String? activeTier,
    bool? isComplete,
    List<Map<String, dynamic>>? discoveredPeers,
    List<String>? stagedFiles,
    bool? bubbleEnabled,
  }) {
    return TransferTelemetryState(
      speedMbps: speedMbps ?? this.speedMbps,
      bytesTransferred: bytesTransferred ?? this.bytesTransferred,
      totalBytes: totalBytes ?? this.totalBytes,
      completedChunks: completedChunks ?? this.completedChunks,
      totalChunks: totalChunks ?? this.totalChunks,
      activeChannels: activeChannels ?? this.activeChannels,
      activeTier: activeTier ?? this.activeTier,
      isComplete: isComplete ?? this.isComplete,
      discoveredPeers: discoveredPeers ?? this.discoveredPeers,
      stagedFiles: stagedFiles ?? this.stagedFiles,
      bubbleEnabled: bubbleEnabled ?? this.bubbleEnabled,
    );
  }
}

class OmniDropController extends StateNotifier<TransferTelemetryState> {
  static const _methodChannel = MethodChannel('com.omnidrop.mobile/native_bridge');
  static const _bleEvents = EventChannel('com.omnidrop.mobile/ble_events');

  OmniDropFfiBridge? _ffi;
  Timer? _telemetryTimer;

  OmniDropController() : super(const TransferTelemetryState()) {
    _bootstrap();
  }

  Future<void> _bootstrap() async {
    final dir = await getApplicationDocumentsDirectory();
    try {
      _ffi = OmniDropFfiBridge();
      _ffi!.initializeCore(
        deviceId: 'android_${DateTime.now().millisecondsSinceEpoch % 10000}',
        deviceName: 'Pixel 9 Pro (OmniDrop)',
        platform: 'android',
        listenPort: 48555,
        outputDir: dir.path,
      );
    } catch (_) {}

    await _methodChannel.invokeMethod('startBleDiscovery', {
      'deviceId': 'android_node',
      'deviceName': 'Android OmniDrop',
      'localIp': '192.168.1.42',
      'port': 48555,
    });

    final shared = await _methodChannel.invokeListMethod<String>('getSharedFiles');
    if (shared != null && shared.isNotEmpty) {
      state = state.copyWith(stagedFiles: shared);
    }

    _bleEvents.receiveBroadcastStream().listen((event) {
      if (event is Map) {
        final peer = Map<String, dynamic>.from(event);
        final updated = [...state.discoveredPeers];
        updated.removeWhere((p) => p['device_id'] == peer['device_id']);
        updated.add(peer);
        state = state.copyWith(discoveredPeers: updated);
      }
    });

    _telemetryTimer = Timer.periodic(const Duration(milliseconds: 200), (_) {
      final ffi = _ffi;
      if (ffi == null) return;
      final snap = ffi.pollTransferTelemetry();
      final mdnsPeers = ffi.getDiscoveredPeers();

      if (mdnsPeers.isNotEmpty) {
        final merged = [...state.discoveredPeers];
        for (final p in mdnsPeers) {
          merged.removeWhere((existing) => existing['device_id'] == p['device_id']);
          merged.add(p);
        }
        state = state.copyWith(discoveredPeers: merged);
      }

      if (snap != null) {
        final mbps = (snap['speed_mbps'] as num?)?.toDouble() ?? 0.0;
        final doneChunks = (snap['completed_chunks'] as num?)?.toInt() ?? 0;
        final totChunks = (snap['total_chunks'] as num?)?.toInt() ?? 0;
        final channels = (snap['active_channels'] as num?)?.toInt() ?? 1;
        final tier = (snap['active_tier'] as String?) ?? 'Tier 2 (QUIC)';

        state = state.copyWith(
          speedMbps: mbps,
          bytesTransferred: (snap['bytes_transferred'] as num?)?.toInt() ?? 0,
          totalBytes: (snap['total_bytes'] as num?)?.toInt() ?? 0,
          completedChunks: doneChunks,
          totalChunks: totChunks,
          activeChannels: channels,
          activeTier: tier,
          isComplete: (snap['is_complete'] as bool?) ?? false,
        );

        _methodChannel.invokeMethod('updateForegroundMetrics', {
          'speedMbps': mbps,
          'completedChunks': doneChunks,
          'totalChunks': totChunks,
          'activeChannels': channels,
          'activeTier': tier,
        });
      }
    });
  }

  Future<void> pickFiles() async {
    final res = await FilePicker.platform.pickFiles(allowMultiple: true);
    if (res != null) {
      final paths = res.paths.whereType<String>().toList();
      state = state.copyWith(stagedFiles: [...state.stagedFiles, ...paths]);
    }
  }

  Future<void> toggleBubble(bool enable) async {
    final ok = await _methodChannel.invokeMethod<bool>(
          'toggleFloatingBubble',
          {'enable': enable},
        ) ??
        false;
    state = state.copyWith(bubbleEnabled: ok ? enable : false);
  }

  void sendToPeer(Map<String, dynamic> peer) {
    if (state.stagedFiles.isEmpty) return;
    final rawEndpoints = (peer['endpoints'] as List?)?.cast<Map<String, dynamic>>() ??
        [
          {
            'ip': peer['ip'] ?? '192.168.49.1',
            'port': peer['port'] ?? 48555,
            'tier': 'Tier1WifiDirect',
            'interface_name': 'p2p-wlan0-0',
            'max_streams': 6,
          }
        ];
    _ffi?.startMultipathTransfer(
      filePaths: state.stagedFiles,
      endpoints: rawEndpoints,
    );
  }

  @override
  void dispose() {
    _telemetryTimer?.cancel();
    _ffi?.shutdown();
    super.dispose();
  }
}

final omniDropProvider =
    StateNotifierProvider<OmniDropController, TransferTelemetryState>(
  (ref) => OmniDropController(),
);

class OmniDropMobileApp extends ConsumerWidget {
  const OmniDropMobileApp({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final telemetry = ref.watch(omniDropProvider);
    final controller = ref.read(omniDropProvider.notifier);

    return MaterialApp(
      title: 'OmniDrop',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        useMaterial3: true,
        brightness: Brightness.dark,
        colorSchemeSeed: const Color(0xFF38BDF8),
        scaffoldBackgroundColor: const Color(0xFF0B0F19),
      ),
      home: Scaffold(
        appBar: AppBar(
          backgroundColor: const Color(0xFF111827),
          title: const Text('OmniDrop • Multipath P2P'),
          actions: [
            Row(
              children: [
                const Text('Drop Bubble', style: TextStyle(fontSize: 12)),
                Switch(
                  value: telemetry.bubbleEnabled,
                  onChanged: controller.toggleBubble,
                ),
              ],
            ),
          ],
        ),
        body: Padding(
          padding: const EdgeInsets.all(16),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              // Active Telemetry Banner
              Card(
                color: const Color(0xFF1E293B),
                child: Padding(
                  padding: const EdgeInsets.all(16),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        '${telemetry.speedMbps.toStringAsFixed(1)} Mbps',
                        style: const TextStyle(
                          fontSize: 28,
                          fontWeight: FontWeight.bold,
                          color: Color(0xFF38BDF8),
                        ),
                      ),
                      const SizedBox(height: 4),
                      Text(
                        '${telemetry.activeTier} • ${telemetry.activeChannels} Active Streams',
                        style: const TextStyle(color: Colors.white70),
                      ),
                      const SizedBox(height: 12),
                      LinearProgressIndicator(
                        value: telemetry.totalChunks > 0
                            ? telemetry.completedChunks / telemetry.totalChunks
                            : 0.0,
                      ),
                      const SizedBox(height: 6),
                      Text(
                        'Verified Chunks (SHA-256): ${telemetry.completedChunks} / ${telemetry.totalChunks}',
                        style: const TextStyle(fontSize: 12, color: Colors.white60),
                      ),
                    ],
                  ),
                ),
              ),
              const SizedBox(height: 16),
              // Staged Files Card
              Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  Text(
                    'Staged Payload (${telemetry.stagedFiles.length} files)',
                    style: const TextStyle(fontSize: 16, fontWeight: FontWeight.w600),
                  ),
                  FilledButton.icon(
                    onPressed: controller.pickFiles,
                    icon: const Icon(Icons.add),
                    label: const Text('Select Files'),
                  ),
                ],
              ),
              const SizedBox(height: 12),
              const Text(
                'Nearby Discovered Peers (BLE + mDNS + ECDH Verified)',
                style: TextStyle(fontSize: 16, fontWeight: FontWeight.w600),
              ),
              const SizedBox(height: 8),
              Expanded(
                child: telemetry.discoveredPeers.isEmpty
                    ? const Center(
                        child: Text(
                          'Scanning BLE (0xFD6F) & _omnidrop._tcp.local...',
                          style: TextStyle(color: Colors.white54),
                        ),
                      )
                    : ListView.builder(
                        itemCount: telemetry.discoveredPeers.length,
                        itemBuilder: (context, index) {
                          final peer = telemetry.discoveredPeers[index];
                          return Card(
                            color: const Color(0xFF111827),
                            child: ListTile(
                              leading: const CircleAvatar(
                                backgroundColor: Color(0xFF0284C7),
                                child: Icon(Icons.devices, color: Colors.white),
                              ),
                              title: Text(peer['device_name']?.toString() ?? 'Peer'),
                              subtitle: Text(
                                '${peer['platform']} • ${peer['ip'] ?? 'Multi-NIC'}',
                              ),
                              trailing: ElevatedButton(
                                onPressed: () => controller.sendToPeer(peer),
                                child: const Text('Beam'),
                              ),
                            ),
                          );
                        },
                      ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
