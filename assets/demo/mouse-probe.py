#!/usr/bin/env python3
"""Mouse protocol probe: enables 1002 (button motion) + 1006 (SGR)."""
import os
import select
import sys
import termios
import time
import tty

out_path = sys.argv[1] if len(sys.argv) > 1 else "/tmp/pretty-mouse.txt"
seconds = float(sys.argv[2]) if len(sys.argv) > 2 else 8.0
fd = sys.stdin.fileno()
old = termios.tcgetattr(fd)
tty.setraw(fd)

def w(data):
    os.write(fd, data if isinstance(data, bytes) else data.encode())

def read_for(t):
    data = b""
    end = time.time() + t
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], max(0.0, end - time.time()))
        if not r:
            break
        chunk = os.read(fd, 4096)
        if not chunk:
            break
        data += chunk
    return data

w(b"\x1b[?1002h\x1b[?1006h\x1b[2J\x1b[H")
w(b"\x1b[1;38;2;122;162;247mMouse protocol probe\x1b[0m\r\n\r\n")
w(b"Tracking: 1002 (button-event) + 1006 (SGR).\r\n")
w(b"Click, drag and release inside this pane; raw encodings are written to:\r\n")
w(out_path.encode() + b"\r\n")
data = read_for(seconds)
with open(out_path, "wb") as f:
    f.write(data)
w(b"\x1b[?1002l\x1b[?1006l\r\nProbe finished; mouse tracking disabled.\r\n")
termios.tcsetattr(fd, termios.TCSADRAIN, old)
time.sleep(3600)
