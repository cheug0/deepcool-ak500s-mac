#!/bin/bash
# 停止并移除 ak500s LaunchAgent
set -euo pipefail
launchctl bootout gui/$(id -u) ~/Library/LaunchAgents/com.ak500s.mac.plist 2>/dev/null || true
rm -f ~/Library/LaunchAgents/com.ak500s.mac.plist
echo "已停止并卸载 LaunchAgent（日志目录 ~/Library/Logs/ak500s-mac 保留）"
