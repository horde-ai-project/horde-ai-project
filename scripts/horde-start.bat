@echo off
REM Thin wrapper for PowerShell start script.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0horde-start.ps1" %*
