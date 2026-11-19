@echo off
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
cargo build 2>&1 | findstr /R /C:"error" /C:"warning: unused" /C:"Finished"
