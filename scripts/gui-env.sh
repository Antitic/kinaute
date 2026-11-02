#!/bin/bash
# Start a lightweight X session (Xvfb + i3) for GUI tests. usage: gui-env.sh start|stop
case "$1" in
start)
  pkill Xvfb 2>/dev/null; pkill i3 2>/dev/null; sleep 0.5
  Xvfb :99 -screen 0 1280x800x24 -nolisten tcp >/tmp/xvfb.log 2>&1 &
  sleep 1
  mkdir -p /tmp/i3cfg && printf 'font pango:DejaVu Sans 9\ndefault_border none\nfor_window [class=".*"] border none\n' > /tmp/i3cfg/config
  DISPLAY=:99 i3 -c /tmp/i3cfg/config >/tmp/i3.log 2>&1 &
  sleep 1 ;;
stop) pkill Xvfb; pkill i3 ;;
esac
