#!/bin/bash
# 맥에서 윈도우 설치본 크로스 컴파일 (2026-09-27 성공): brew llvm + nsis, cargo-xwin, target x86_64-pc-windows-msvc, lld-link 은 lld 심볼릭
cd "$(dirname "$0")/nike_app"; export PATH="$HOME/.cargo/bin:/opt/homebrew/opt/llvm/bin:$PATH"
[ -x /opt/homebrew/opt/llvm/bin/lld-link ] || ln -s /opt/homebrew/opt/llvm/bin/lld /opt/homebrew/opt/llvm/bin/lld-link
cargo tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis 2>&1 | grep -E "^error|error\[|Finished|Finished 1 bundle" -A 2
ls -la src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/*.exe src-tauri/target/x86_64-pc-windows-msvc/release/nike_app.exe
