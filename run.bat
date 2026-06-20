@echo off
REM One-click launcher for the Chess Brilliant analyzer.
REM Double-click this file, or run it from a terminal.
cd /d "%~dp0"

REM 1. Build the Rust engine if it isn't built yet.
if not exist "engine\target\release\chess_engine.exe" (
    echo Building the Rust engine ^(first run only^)...
    pushd engine
    cargo build --release
    popd
    if not exist "engine\target\release\chess_engine.exe" (
        echo.
        echo ERROR: engine build failed. Is Rust installed? ^(https://rustup.rs^)
        pause
        exit /b 1
    )
)

REM 2. Make sure Python dependencies are present.
python -c "import chess, flask" 2>nul
if errorlevel 1 (
    echo Installing Python dependencies ^(first run only^)...
    python -m pip install chess flask
)

REM 3. Open the browser a few seconds after the server starts.
start "" cmd /c "timeout /t 3 >nul & start http://127.0.0.1:5000"

REM 4. Run the server (Ctrl+C to stop).
echo.
echo ============================================================
echo  Chess Brilliant running at http://127.0.0.1:5000
echo  Close this window or press Ctrl+C to stop.
echo ============================================================
cd analysis
python server.py
