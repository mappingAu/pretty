#!/usr/bin/env python3
"""Protocol-level kitty graphics check: error paths, chunking, query, delete."""
import base64
import os
import select
import sys
import termios
import time
import tty
from pathlib import Path

result_path = sys.argv[1] if len(sys.argv) > 1 else "/tmp/pretty-graphics.txt"
root = Path(__file__).resolve().parent
img = base64.b64encode((root / "kitty-demo.png").read_bytes()).decode()
chunks = [img[i:i+4096] for i in range(0, len(img), 4096)]

fd = sys.stdin.fileno()
old = termios.tcgetattr(fd)
tty.setraw(fd)

def w(data):
    os.write(fd, data if isinstance(data, bytes) else data.encode())

def read_for(secs):
    data = b""
    end = time.time() + secs
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], max(0.0, end - time.time()))
        if not r:
            break
        chunk = os.read(fd, 4096)
        if not chunk:
            break
        data += chunk
    return data

w(b"\x1b[2J\x1b[H")
w(b"\x1b[1;38;2;122;162;247mKitty graphics protocol probe\x1b[0m\r\n\r\n")

# 1. Query a non-existent image -> ENOENT/EINVAL response.
w(b"\x1b_Ga=q,i=4242\x1b\\")
not_found = read_for(0.6)
# 2. Chunked PNG transmission (transmit only).
for i, chunk in enumerate(chunks):
    more = 1 if i + 1 < len(chunks) else 0
    if i == 0:
        w(f"\x1b_Ga=t,f=100,i=7,m={more};{chunk}\x1b\\")
    else:
        w(f"\x1b_Gm={more};{chunk}\x1b\\")
transmit = read_for(0.8)
# 3. Query the stored image -> OK.
w(b"\x1b_Ga=q,i=7\x1b\\")
query_ok = read_for(0.6)
# 4. Place it with a placement id and z-index.
w(b"\x1b_Ga=p,i=7,p=3,c=20,r=10,z=-1\x1b\\")
place = read_for(0.6)
# 5. Delete by image id -> OK.
w(b"\x1b_Ga=d,d=i,i=7\x1b\\")
delete = read_for(0.6)
w(b"\x1b_Ga=q,i=7\x1b\\")
query_after = read_for(0.6)

with open(result_path, "wb") as f:
    f.write(b"NOT_FOUND=" + not_found + b"\n")
    f.write(b"TRANSMIT=" + transmit + b"\n")
    f.write(b"QUERY_OK=" + query_ok + b"\n")
    f.write(b"PLACE=" + place + b"\n")
    f.write(b"DELETE=" + delete + b"\n")
    f.write(b"QUERY_AFTER_DELETE=" + query_after + b"\n")

w(b"\r\nProtocol responses:\r\n")
for label, data in [
    ("query missing image", not_found),
    ("transmit chunked PNG", transmit),
    ("query image", query_ok),
    ("place z=-1", place),
    ("delete image", delete),
    ("query deleted image", query_after),
]:
    w(f"  {label}: {data!r}\r\n".encode())
termios.tcsetattr(fd, termios.TCSADRAIN, old)
time.sleep(3600)
