@echo off
REM One-command Windows batch script to push OmniDrop to GitHub and trigger cloud builds.
IF "%~1"=="" (
    echo Usage: push_to_github_cloud_build.bat https://github.com/YOUR_USERNAME/omnidrop.git [v1.0.0]
    exit /b 1
)

SET REMOTE_URL=%~1
SET RELEASE_TAG=%~2
IF "%RELEASE_TAG%"=="" SET RELEASE_TAG=v1.0.0

cd /d "%~dp0.."

IF NOT EXIST ".git" (
    git init
    git branch -M main
)

git add -A
git commit -m "OmniDrop Full-Stack Workspace + Cloud Release Workflow"
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
