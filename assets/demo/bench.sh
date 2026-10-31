#!/bin/bash
# 80x24 full-screen colour/attribute workload for the rendering benchmark.
printf '\033]0;Pretty benchmark\007'
for r in $(seq 1 24); do
  printf '\033[%d;1H' "$r"
  for c in $(seq 1 80); do
    n=$(( (r * 13 + c * 7) % 6 + 31 ))
    printf '\033[%d;%dm' "$n" "$n"
    if (( c % 17 == 0 )); then
      printf '=>'
    elif (( c % 19 == 0 )); then
      printf '!='
    elif (( c % 11 == 0 )); then
      printf '✓'
    else
      printf '%s' "${r: -1}${c: -1}"
    fi
  done
done
printf '\033[0m\033[?25l'
sleep 3600
