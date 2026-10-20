#!/bin/bash
# Read-only soak sampler for Linux (#71): every $1 seconds (default 300), one CSV row per running Singularity process
# (Client, Daemon, Soloist) with memory, threads, open files and CPU time so far. Reads /proc only; touches nothing.
# Usage: tools/soak.sh [secs] [out.csv]   (default ~/.cache/singularity/soak.csv)
every=${1:-300}
out=${2:-$HOME/.cache/singularity/soak.csv}
tck=$(getconf CLK_TCK)
[ -s "$out" ] || echo "time,name,pid,rss_kb,threads,fds,cpu_s" > "$out"
while true; do
  now=$(date +%FT%T)
  for pid in $(pgrep -u "$(id -u)" -x 'singularity|singularityd|soloist'); do
    name=$(cat /proc/$pid/comm 2>/dev/null) || continue
    # the installed binaries only, not test runs from the repo
    case $(readlink /proc/$pid/exe) in */target/*) continue ;; esac
    rss=$(awk '/^VmRSS/ {print $2}' /proc/$pid/status)
    thr=$(awk '/^Threads/ {print $2}' /proc/$pid/status)
    fds=$(ls /proc/$pid/fd 2>/dev/null | wc -l)
    # fields 14 and 15 of stat: utime, stime (after the ")" that ends comm, they're 12 and 13)
    cpu=$(sed 's/.*) //' /proc/$pid/stat | awk -v t="$tck" '{printf "%.1f", ($12 + $13) / t}')
    echo "$now,$name,$pid,$rss,$thr,$fds,$cpu" >> "$out"
  done
  sleep "$every"
done
