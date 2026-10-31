#!/bin/bash
# Cycle DECSCUSR cursor shapes with labels so screenshots/logs demonstrate
# cursor shape control through the terminal protocol.
printf '\033]0;Pretty cursor styles\007\033[2J\033[H'
printf '\033[1;38;2;122;162;247mCursor shape control (DECSCUSR, CSI Ps SP q)\033[0m\n\n'
printf 'Steady block      \033[2 q  <-- should be a filled cell\n'; sleep 2
printf 'Steady underline  \033[4 q  <-- should be a line\n'; sleep 2
printf 'Steady bar        \033[6 q  <-- should be a vertical bar\n'; sleep 2
printf 'Blinking block    \033[1 q  <-- should blink\n'; sleep 2
printf 'Default block     \033[0 q\n'
sleep 3600
