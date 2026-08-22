@echo off
setlocal
if not exist target\release\multibot-26-2.exe cargo build --release
if errorlevel 1 exit /b %errorlevel%
target\release\multibot-26-2.exe

