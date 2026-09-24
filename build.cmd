@echo off
setlocal
cd /d "%~dp0"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\build-isolation.ps1" %*
set "BUILD_RESULT=%ERRORLEVEL%"
if not "%BUILD_RESULT%"=="0" echo Build failed. See build-logs for details.
if "%BUILD_RESULT%"=="0" echo Build completed. Open artifacts folder.
pause
exit /b %BUILD_RESULT%
