@echo off
REM Double-click this to build what is missing and open the ITTOOT Solidcraft OT Connection Bridge.
REM Extra options pass straight through, e.g.  start-bridge.bat -Page settings -Fresh
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\start-bridge.ps1" %*
if errorlevel 1 pause
