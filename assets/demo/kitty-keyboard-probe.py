#!/usr/bin/env python3
"""Interactive kitty keyboard protocol probe used by scripts/test-hardware.sh.

It queries the negotiated flags, pushes all five progressive-enhancement flags,
reads real key events from the PTY and writes their hex encoding to the path in
argv[1] after a short collection window. The terminal remains usable afterwards.
"""
import os
import select
import sys
import termios
import time
import tty

out_path = sys.argv[1] if len(sys.argv) > 1 else "/tmp/pretty-kitty-keys.txt"
collect_seconds = float(sys.argv[2]) if len(sys.argv) > 2 else 20.0

fd = sys.stdin.fileno()
old = termios.tcgetattr(fd)
tty.setraw(fd)

def write(s: str):
    os.write(fd, s.encode())

def read_for(seconds: float):
    buf = b""
    end = time.time() + seconds
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], max(0.0, end - time.time()))
        if not r:
            break
        chunk = os.read(fd, 4096)
        if not chunk:
            break
        buf += chunk
    return buf

write("\x1b[?u")  # kitty keyboard: query current flags
query = read_for(0.7)
write("\x1b[>31u")  # push all progressive enhancement flags
pushed = read_for(0.3)
write("\x1b[?u")
after_push = read_for(0.5)
write("\x1b[2J\x1b[H")
write("\x1b[1;38;2;122;162;247mKitty keyboard protocol probe\x1b[0m\r\n\r\n")
write("query response: %r\r\n" % query)
write("after push(31): %r\r\n" % after_push)
write("\r\nType keys now (for example: a, A, Ctrl+A, Shift+Enter, arrows).\r\n")
write("The raw encodings are written to: %s\r\n" % out_path)
write("Close with Esc or wait %.0fs.\r\n" % collect_seconds)
sys.stdout.flush()

keys = bytearray()
start = time.time()
while time.time() - start < collect_seconds:
    r, _, _ = select.select([fd], [], [], 0.2)
    if not r:
        continue
    chunk = os.read(fd, 4096)
    if not chunk:
        break
    keys += chunk
    if b"\x1b" in chunk and len(keys) < 8:
        # Do not exit on the kitty CSI whose final byte is 'u'; Esc alone exits.
        pass
    if b"\x1b" == chunk:
        break

with open(out_path, "wb") as f:
    f.write(b"QUERY=" + query + b"\nAFTER_PUSH=" + after_push + b"\nKEYS=" + bytes(keys) + b"\n")

termios.tcsetattr(fd, termios.TCSADRAIN, old)
write("\x1b[?25h")
os.write(fd, b"\r\nProbe finished; results written.\r\n")
