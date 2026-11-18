@echo off
REM Thin wrapper for PowerShell stop script.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0horde-stop.ps1" %*
