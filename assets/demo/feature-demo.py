#!/usr/bin/env python3
"""Feature showcase used for the real-hardware screenshots."""
import base64
import os
import sys
import termios
import time
from pathlib import Path

try:
    fd = sys.stdin.fileno()
    attrs = termios.tcgetattr(fd)
    attrs[3] &= ~termios.ECHO
    termios.tcsetattr(fd, termios.TCSANOW, attrs)
except Exception:
    pass

out = sys.stdout.buffer
def w(data):
    out.write(data if isinstance(data, bytes) else data.encode())
    out.flush()

backend = os.environ.get("PRETTY_BACKEND", "unknown")
gpu = os.environ.get("PRETTY_GPU", "unknown")
root = Path(__file__).resolve().parent
img = base64.b64encode((root / "kitty-demo.png").read_bytes()).decode()

w(b"\x1b[2J\x1b[H\x1b[?25h")
w(f"\x1b[1;38;2;122;162;247mPretty\x1b[0m — hardware accelerated terminal   "
  f"\x1b[38;5;245mbackend={backend}  gpu={gpu}\x1b[0m\r\n")
w("─" * 86 + "\r\n")
w("\x1b[1mASCII\x1b[0m     The quick brown fox jumps over the lazy dog 0123456789\r\n")
w("\x1b[1mLigatures\x1b[0m a => b != c -> d === e <= >= |> <>  (JetBrains Mono liga/calt)\r\n")
w("\x1b[1mUnicode\x1b[0m   Ω≈ç√∫˜µ≤≥÷ 日本語 中文 한글 Русский հայերեն\r\n")
w("\x1b[1mEmoji\x1b[0m     😀 🚀 🎨 🧪 ✅ ❤️  (bundled Noto Emoji + system fallback)\r\n")
w("\x1b[1mTruecolor\x1b[0m \x1b[38;2;255;90;120mred\x1b[0m "
  "\x1b[38;2;255;190;80morange\x1b[0m \x1b[38;2;100;220;150mgreen\x1b[0m "
  "\x1b[38;2;100;170;255mblue\x1b[0m \x1b[38;2;210;130;255mviolet\x1b[0m "
  "\x1b[48;2;35;55;85m 24-bit background \x1b[0m\r\n")
w("\x1b[1mStyles\x1b[0m    \x1b[1mbold\x1b[0m \x1b[3mitalic\x1b[0m \x1b[4munderline\x1b[0m "
  "\x1b[4:3mundercurl\x1b[0m \x1b[9mstrikeout\x1b[0m \x1b[2mdim\x1b[0m\r\n")
w("\x1b[1mHyperlink\x1b[0m \x1b]8;;https://sw.kovidgoyal.net/kitty/\x1b\\kitty protocol documentation\x1b]8;;\x1b\\"
  "   \x1b[38;5;245m(Ctrl+click opens)\x1b[0m\r\n")
w("\x1b[1mKitty graphics\x1b[0m PNG, base64, two APC chunks, z-index, 32x9 cells:\r\n")
w(b"\x1b[10;3H")
chunks = [img[i:i+4096] for i in range(0, len(img), 4096)]
for i, chunk in enumerate(chunks):
    more = 1 if i + 1 < len(chunks) else 0
    if i == 0:
        w(f"\x1b_Ga=T,f=100,i=1,c=32,r=10,z=-1,m={more};{chunk}\x1b\\")
    else:
        w(f"\x1b_Gm={more};{chunk}\x1b\\")
# Text over a negative z-index image demonstrates layering.
w(b"\x1b[11;4H\x1b[1;38;2;255;255;255;48;2;25;35;55m  NEGATIVE-Z IMAGE + TEXT OVERLAY  \x1b[0m")
w("\r\n\x1b[28;1H")
w("\x1b[38;2;130;190;255mProtocol level:\x1b[0m kitty graphics (partial), keyboard (full), OSC 8, OSC 52, OSC 9/777, true color, scrollback, tiling.\r\n")
w("\x1b[38;5;245mShortcuts: Ctrl+, settings · Ctrl+Shift+E split · Alt+Arrows focus · Shift+PageUp scroll · Ctrl+Shift+Q quit\x1b[0m\r\n")
w("\x1b[38;5;240mThis frame is rendered through wgpu; no CPU rasterized terminal path is used.\x1b[0m\r\n")
time.sleep(3600)
