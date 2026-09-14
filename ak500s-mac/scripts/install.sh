#!/bin/bash
# 安装 LaunchAgent：登录后自动启动 ak500s run（CPU 温度模式）
# 用法：./scripts/install.sh [/path/to/ak500s 二进制]
set -euo pipefail

BIN="${1:-}"
if [[ -z "$BIN" ]]; then
    # 默认取本仓库编译产物
    for CAND in \
        "$(dirname "$0")/../target/release/ak500s-mac" \
        "$(dirname "$0")/../target/universal2-apple-darwin/release/ak500s-mac" \
        /usr/local/bin/ak500s; do
        if [[ -x "$CAND" ]]; then BIN="$CAND"; break; fi
    done
fi
if [[ -z "$BIN" || ! -x "$BIN" ]]; then
    echo "找不到可执行文件，请先 cargo build --release，或用法: $0 /path/to/ak500s" >&2
    exit 1
fi
BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"
echo "使用二进制: $BIN"

mkdir -p ~/Library/Logs/ak500s-mac

cat > ~/Library/LaunchAgents/com.ak500s.mac.plist <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.ak500s.mac</string>
    <key>ProgramArguments</key>
    <array>
        <string>${BIN}</string>
        <string>run</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardErrorPath</key>
    <string>$HOME/Library/Logs/ak500s-mac/err.log</string>
    <key>StandardOutPath</key>
    <string>$HOME/Library/Logs/ak500s-mac/out.log</string>
</dict>
</plist>
EOF

launchctl bootout gui/$(id -u) ~/Library/LaunchAgents/com.ak500s.mac.plist 2>/dev/null || true
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.ak500s.mac.plist
echo "已安装并启动。日志: ~/Library/Logs/ak500s-mac/"
echo "停止并卸载: ./scripts/uninstall.sh"
