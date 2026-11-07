@echo off
REM One-command Windows batch script to push OmniDrop to GitHub and trigger cloud builds.
cd /d "%~dp0.."

SET REMOTE_URL=%~1
IF "%REMOTE_URL%"=="" (
    FOR /F "delims=" %%i IN ('git config --get remote.origin.url 2^>NUL') DO SET REMOTE_URL=%%i
)
IF "%REMOTE_URL%"=="" (
    echo [ERROR] No remote URL provided and no origin configured.
    echo Usage: push_to_github_cloud_build.bat https://github.com/YOUR_USERNAME/omnidrop.git [v1.0.0]
    pause
    exit /b 1
)

SET RELEASE_TAG=%~2
IF "%RELEASE_TAG%"=="" SET RELEASE_TAG=v1.0.0

IF NOT EXIST ".git" (
    git init
    git branch -M main
)

git add -A
git commit -m "fix: resolve Rust syntax/types and add Unicode filename & Range support"
git remote remove origin 2>NUL
git remote add origin %REMOTE_URL%
git tag -f %RELEASE_TAG%
git push -u origin main --force
git push origin %RELEASE_TAG% --force

echo ========================================================================
echo Pushed to %REMOTE_URL% with tag %RELEASE_TAG%!
echo Visit your GitHub repository -> Actions / Releases to download:
echo   1. OmniDrop-Android-arm64.apk
echo   2. OmniDrop-Windows-x64.zip
echo ========================================================================
pause
