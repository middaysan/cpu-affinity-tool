@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Start-render-test.ps1"
if errorlevel 1 (
  echo See TEST-ERROR.txt in this folder.
  pause
)
