@echo off
TITLE OmniDrop Instant P2P Node (Zero-Toolchain Mode)
echo ============================================================================
echo   Starting OmniDrop Zero-Toolchain Standalone Engine...
echo   - Native Win32 WM_DROPFILES Drag-and-Drop Overlay
echo   - 4-Stream SHA-256 Chunk Engine + Android Companion PWA Server
echo ============================================================================

where python >NUL 2>NUL
IF %ERRORLEVEL% EQU 0 (
    python "%~dp0omnidrop_instant_node.py"
    goto :eof
)

where py >NUL 2>NUL
IF %ERRORLEVEL% EQU 0 (
    py -3 "%~dp0omnidrop_instant_node.py"
    goto :eof
)

echo [ERROR] Python 3 was not found on your PATH. Install Python 3 from python.org or the Microsoft Store.
pause
