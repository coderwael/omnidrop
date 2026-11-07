// Task 5: Desktop Client (Windows/Linux) with System Tray Daemon, Translucent Drag-and-Drop
// Overlay (`WM_DROPFILES` / X11 / Wayland via `desktop_drop`), and One-Click Receive Picker.
import 'dart:async';
import 'dart:io';
import 'package:desktop_drop/desktop_drop.dart';
import 'package:file_picker/file_picker.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:system_tray/system_tray.dart';
import 'package:window_manager/window_manager.dart';
import '../../mobile_app/lib/ffi_bridge.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await windowManager.ensureInitialized();

  const windowOptions = WindowOptions(
    size: Size(520, 640),
    minimumSize: Size(420, 480),
    center: true,
    backgroundColor: Colors.transparent,
    skipTaskbar: false,
    titleBarStyle: TitleBarStyle.hidden,
    alwaysOnTop: true,
  );

  await windowManager.waitUntilReadyToShow(windowOptions, () async {
    await windowManager.show();
    await windowManager.focus();
  });

  runApp(const ProviderScope(child: OmniDropDesktopApp()));
}

class DesktopAppState {
  final String receiveDirectory;
  final bool autoDeduplicate;
  final bool isDraggingOver;
  final List<String> droppedFiles;
  final List<Map<String, dynamic>> discoveredPeers;
  final double speedMbps;
  final int completedChunks;
  final int totalChunks;
  final int activeChannels;
  final String activeTier;

  const DesktopAppState({
    required this.receiveDirectory,
    this.autoDeduplicate = true,
    this.isDraggingOver = false,
    this.droppedFiles = const [],
    this.discoveredPeers = const [],
    this.speedMbps = 0.0,
    this.completedChunks = 0,
    this.totalChunks = 0,
    this.activeChannels = 0,
    this.activeTier = 'Listening on 0.0.0.0:48555 (QUIC + Multi-TCP)',
  });

  DesktopAppState copyWith({
    String? receiveDirectory,
    bool? autoDeduplicate,
    bool? isDraggingOver,
    List<String>? droppedFiles,
    List<Map<String, dynamic>>? discoveredPeers,
    double? speedMbps,
    int? completedChunks,
    int? totalChunks,
    int? activeChannels,
    String? activeTier,
  }) {
    return DesktopAppState(
      receiveDirectory: receiveDirectory ?? this.receiveDirectory,
      autoDeduplicate: autoDeduplicate ?? this.autoDeduplicate,
      isDraggingOver: isDraggingOver ?? this.isDraggingOver,
      droppedFiles: droppedFiles ?? this.droppedFiles,
      discoveredPeers: discoveredPeers ?? this.discoveredPeers,
      speedMbps: speedMbps ?? this.speedMbps,
      completedChunks: completedChunks ?? this.completedChunks,
      totalChunks: totalChunks ?? this.totalChunks,
      activeChannels: activeChannels ?? this.activeChannels,
      activeTier: activeTier ?? this.activeTier,
    );
  }
}

class DesktopDropController extends StateNotifier<DesktopAppState> {
  final SystemTray _systemTray = SystemTray();
  OmniDropFfiBridge? _ffi;
  Timer? _pollTimer;

  DesktopDropController()
      : super(DesktopAppState(
          receiveDirectory:
              '${Platform.environment['USERPROFILE'] ?? Platform.environment['HOME'] ?? '.'}/Downloads/OmniDrop',
        )) {
    _initSystemTray();
    _initRustCore();
  }

  Future<void> _initSystemTray() async {
    try {
      await _systemTray.initSystemTray(
        title: 'OmniDrop',
        iconPath: Platform.isWindows ? 'assets/app_icon.ico' : 'assets/app_icon.png',
        toolTip: 'OmniDrop P2P Daemon (Active)',
      );

      final menu = Menu();
      await menu.buildFrom([
        MenuItemLabel(
          label: 'Show Drop Zone Overlay',
          onClicked: (_) async {
            await windowManager.show();
            await windowManager.focus();
          },
        ),
        MenuItemLabel(
          label: 'Hide to Background Tray',
          onClicked: (_) async => await windowManager.hide(),
        ),
        MenuSeparator(),
        MenuItemLabel(
          label: 'Quit OmniDrop Daemon',
          onClicked: (_) async {
            _ffi?.shutdown();
            exit(0);
          },
        ),
      ]);
      await _systemTray.setContextMenu(menu);
    } catch (_) {}
  }

  void _initRustCore() {
    Directory(state.receiveDirectory).createSync(recursive: true);
    try {
      _ffi = OmniDropFfiBridge();
      _ffi!.initializeCore(
        deviceId: 'desktop_${Platform.localHostname}',
        deviceName: '${Platform.localHostname} (${Platform.operatingSystem})',
        platform: Platform.operatingSystem,
        listenPort: 48555,
        outputDir: state.receiveDirectory,
      );
    } catch (_) {}

    _pollTimer = Timer.periodic(const Duration(milliseconds: 200), (_) {
      final ffi = _ffi;
      if (ffi == null) return;
      final peers = ffi.getDiscoveredPeers();
      final snap = ffi.pollTransferTelemetry();

      state = state.copyWith(
        discoveredPeers: peers,
        speedMbps: (snap?['speed_mbps'] as num?)?.toDouble() ?? state.speedMbps,
        completedChunks:
            (snap?['completed_chunks'] as num?)?.toInt() ?? state.completedChunks,
        totalChunks: (snap?['total_chunks'] as num?)?.toInt() ?? state.totalChunks,
        activeChannels:
            (snap?['active_channels'] as num?)?.toInt() ?? state.activeChannels,
        activeTier: (snap?['active_tier'] as String?) ?? state.activeTier,
      );
    });
  }

  void setDragging(bool dragging) {
    state = state.copyWith(isDraggingOver: dragging);
  }

  void onFilesDropped(List<String> paths) {
    state = state.copyWith(
      isDraggingOver: false,
      droppedFiles: [...state.droppedFiles, ...paths],
    );
  }

  Future<void> pickReceiveDirectory() async {
    final selected = await FilePicker.platform.getDirectoryPath(
      dialogTitle: 'Select OmniDrop Receive Directory (Auto-Deduplicating)',
    );
    if (selected != null) {
      state = state.copyWith(receiveDirectory: selected);
      _ffi?.shutdown();
      _initRustCore();
    }
  }

  void dispatchDroppedFilesToPeer(Map<String, dynamic> peer) {
    if (state.droppedFiles.isEmpty) return;
    final endpoints = (peer['endpoints'] as List?)?.cast<Map<String, dynamic>>() ??
        [
          {
            'ip': peer['ip'] ?? '192.168.1.42',
            'port': peer['port'] ?? 48555,
            'tier': 'Tier2QuicMultiStream',
            'interface_name': 'eth0',
            'max_streams': 8,
          }
        ];
    _ffi?.startMultipathTransfer(
      filePaths: state.droppedFiles,
      endpoints: endpoints,
    );
  }

  @override
  void dispose() {
    _pollTimer?.cancel();
    _ffi?.shutdown();
    super.dispose();
  }
}

final desktopProvider =
    StateNotifierProvider<DesktopDropController, DesktopAppState>(
  (ref) => DesktopDropController(),
);

class OmniDropDesktopApp extends ConsumerWidget {
  const OmniDropDesktopApp({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final appState = ref.watch(desktopProvider);
    final controller = ref.read(desktopProvider.notifier);

    return MaterialApp(
      title: 'OmniDrop Desktop',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        useMaterial3: true,
        brightness: Brightness.dark,
        colorSchemeSeed: const Color(0xFF38BDF8),
      ),
      home: Scaffold(
        backgroundColor: const Color(0xEB0F172A),
        body: Column(
          children: [
            // Custom Frameless Drag Bar
            GestureDetector(
              onPanStart: (_) => windowManager.startDragging(),
              child: Container(
                height: 42,
                padding: const EdgeInsets.symmetric(horizontal: 14),
                color: const Color(0xFF1E293B),
                child: Row(
                  mainAxisAlignment: MainAxisAlignment.spaceBetween,
                  children: [
                    const Row(
                      children: [
                        Icon(Icons.bolt, color: Color(0xFF38BDF8), size: 18),
                        SizedBox(width: 8),
                        Text(
                          'OmniDrop Desktop Overlay',
                          style: TextStyle(fontWeight: FontWeight.w600),
                        ),
                      ],
                    ),
                    IconButton(
                      icon: const Icon(Icons.minimize, size: 18),
                      tooltip: 'Minimize to System Tray',
                      onPressed: () => windowManager.hide(),
                    ),
                  ],
                ),
              ),
            ),
            Expanded(
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    // Native OS Drag-and-Drop Zone (WM_DROPFILES / X11 / Wayland)
                    DropTarget(
                      onDragEntered: (_) => controller.setDragging(true),
                      onDragExited: (_) => controller.setDragging(false),
                      onDragDone: (details) {
                        controller.onFilesDropped(
                          details.files.map((x) => x.path).toList(),
                        );
                      },
                      child: AnimatedContainer(
                        duration: const Duration(milliseconds: 160),
                        height: 170,
                        decoration: BoxDecoration(
                          color: appState.isDraggingOver
                              ? const Color(0x3338BDF8)
                              : const Color(0xFF1E293B),
                          borderRadius: BorderRadius.circular(16),
                          border: Border.all(
                            color: appState.isDraggingOver
                                ? const Color(0xFF38BDF8)
                                : Colors.white24,
                            width: 2,
                          ),
                        ),
                        child: Column(
                          mainAxisAlignment: MainAxisAlignment.center,
                          children: [
                            const Icon(
                              Icons.cloud_upload_outlined,
                              size: 44,
                              color: Color(0xFF38BDF8),
                            ),
                            const SizedBox(height: 8),
                            Text(
                              appState.droppedFiles.isEmpty
                                  ? 'Drop Files or Folders Here to Beam'
                                  : '${appState.droppedFiles.length} File(s) Staged for Multi-Stream Transfer',
                              style: const TextStyle(
                                fontSize: 15,
                                fontWeight: FontWeight.w600,
                              ),
                            ),
                            const SizedBox(height: 4),
                            Text(
                              '${appState.speedMbps.toStringAsFixed(1)} Mbps • ${appState.activeTier}',
                              style: const TextStyle(
                                fontSize: 12,
                                color: Colors.white60,
                              ),
                            ),
                          ],
                        ),
                      ),
                    ),
                    const SizedBox(height: 14),
                    // One-Click Receive Directory Picker with Automatic Deduplication
                    Container(
                      padding: const EdgeInsets.all(12),
                      decoration: BoxDecoration(
                        color: const Color(0xFF111827),
                        borderRadius: BorderRadius.circular(12),
                      ),
                      child: Row(
                        children: [
                          const Icon(Icons.folder_special, color: Color(0xFF38BDF8)),
                          const SizedBox(width: 10),
                          Expanded(
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                const Text(
                                  'Receive Folder (Auto-Deduplication Enabled)',
                                  style: TextStyle(fontSize: 11, color: Colors.white60),
                                ),
                                Text(
                                  appState.receiveDirectory,
                                  overflow: TextOverflow.ellipsis,
                                  style: const TextStyle(fontSize: 13),
                                ),
                              ],
                            ),
                          ),
                          TextButton(
                            onPressed: controller.pickReceiveDirectory,
                            child: const Text('Change'),
                          ),
                        ],
                      ),
                    ),
                    const SizedBox(height: 14),
                    const Text(
                      'Discovered LAN / Wi-Fi Direct Peers',
                      style: TextStyle(fontSize: 14, fontWeight: FontWeight.w600),
                    ),
                    const SizedBox(height: 8),
                    Expanded(
                      child: ListView.builder(
                        itemCount: appState.discoveredPeers.length,
                        itemBuilder: (context, idx) {
                          final peer = appState.discoveredPeers[idx];
                          return Card(
                            color: const Color(0xFF1E293B),
                            child: ListTile(
                              leading: const Icon(Icons.phone_android, color: Color(0xFF38BDF8)),
                              title: Text(peer['device_name']?.toString() ?? 'Peer'),
                              subtitle: Text(peer['platform']?.toString() ?? 'android'),
                              trailing: FilledButton(
                                onPressed: () => controller.dispatchDroppedFilesToPeer(peer),
                                child: const Text('Send Now'),
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
          ],
        ),
      ),
    );
  }
}
