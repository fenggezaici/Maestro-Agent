@echo off
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
cargo test --lib architecture 2>&1 | findstr /R /C:"error" /C:"FAILED" /C:"test result"
