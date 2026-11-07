// Task 2C & Task 4: Dart FFI Bridge to `librust_omnidrop`
import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'package:ffi/ffi.dart';

typedef _CoreInitNative = Int32 Function(
  Pointer<Utf8> deviceId,
  Pointer<Utf8> deviceName,
  Pointer<Utf8> platform,
  Uint16 listenPort,
  Pointer<Utf8> outputDir,
);
typedef _CoreInitDart = int Function(
  Pointer<Utf8> deviceId,
  Pointer<Utf8> deviceName,
  Pointer<Utf8> platform,
  int listenPort,
  Pointer<Utf8> outputDir,
);

typedef _SendFilesNative = Int32 Function(
  Pointer<Utf8> filesJson,
  Pointer<Utf8> endpointsJson,
);
typedef _SendFilesDart = int Function(
  Pointer<Utf8> filesJson,
  Pointer<Utf8> endpointsJson,
);

typedef _PollJsonNative = Pointer<Utf8> Function();
typedef _PollJsonDart = Pointer<Utf8> Function();

typedef _FreeStringNative = Void Function(Pointer<Utf8> ptr);
typedef _FreeStringDart = void Function(Pointer<Utf8> ptr);

typedef _ShutdownNative = Void Function();
typedef _ShutdownDart = void Function();

class OmniDropFfiBridge {
  late final DynamicLibrary _lib;
  late final _CoreInitDart _init;
  late final _SendFilesDart _sendFiles;
  late final _PollJsonDart _pollProgress;
  late final _PollJsonDart _listPeers;
  late final _FreeStringDart _freeString;
  late final _ShutdownDart _shutdown;

  OmniDropFfiBridge() {
    if (Platform.isAndroid || Platform.isLinux) {
      _lib = DynamicLibrary.open('librust_omnidrop.so');
    } else if (Platform.isWindows) {
      _lib = DynamicLibrary.open('rust_omnidrop.dll');
    } else {
      throw UnsupportedError('Unsupported platform: ${Platform.operatingSystem}');
    }

    _init = _lib.lookupFunction<_CoreInitNative, _CoreInitDart>('omnidrop_core_init');
    _sendFiles = _lib.lookupFunction<_SendFilesNative, _SendFilesDart>('omnidrop_send_files');
    _pollProgress = _lib.lookupFunction<_PollJsonNative, _PollJsonDart>('omnidrop_poll_progress_json');
    _listPeers = _lib.lookupFunction<_PollJsonNative, _PollJsonDart>('omnidrop_list_discovered_peers_json');
    _freeString = _lib.lookupFunction<_FreeStringNative, _FreeStringDart>('omnidrop_free_string');
    _shutdown = _lib.lookupFunction<_ShutdownNative, _ShutdownDart>('omnidrop_core_shutdown');
  }

  int initializeCore({
    required String deviceId,
    required String deviceName,
    required String platform,
    required int listenPort,
    required String outputDir,
  }) {
    final idPtr = deviceId.toNativeUtf8();
    final namePtr = deviceName.toNativeUtf8();
    final platPtr = platform.toNativeUtf8();
    final dirPtr = outputDir.toNativeUtf8();
    try {
      return _init(idPtr, namePtr, platPtr, listenPort, dirPtr);
    } finally {
      calloc.free(idPtr);
      calloc.free(namePtr);
      calloc.free(platPtr);
      calloc.free(dirPtr);
    }
  }

  int startMultipathTransfer({
    required List<String> filePaths,
    required List<Map<String, dynamic>> endpoints,
  }) {
    final filesPtr = jsonEncode(filePaths).toNativeUtf8();
    final epsPtr = jsonEncode(endpoints).toNativeUtf8();
    try {
      return _sendFiles(filesPtr, epsPtr);
    } finally {
      calloc.free(filesPtr);
      calloc.free(epsPtr);
    }
  }

  Map<String, dynamic>? pollTransferTelemetry() {
    final ptr = _pollProgress();
    if (ptr == nullptr) return null;
    try {
      final jsonStr = ptr.toDartString();
      return jsonDecode(jsonStr) as Map<String, dynamic>;
    } finally {
      _freeString(ptr);
    }
  }

  List<Map<String, dynamic>> getDiscoveredPeers() {
    final ptr = _listPeers();
    if (ptr == nullptr) return const [];
    try {
      final raw = jsonDecode(ptr.toDartString()) as List<dynamic>;
      return raw.cast<Map<String, dynamic>>();
    } finally {
      _freeString(ptr);
    }
  }

  void shutdown() => _shutdown();
}
