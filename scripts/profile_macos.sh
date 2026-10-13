#!/bin/sh
set -eu
binary="${1:-target/release/desktop-pet}"
mode="${2:-idle}"
directory="$(mktemp -d)"
cleanup() {
  if [ -n "${pid:-}" ]; then kill "$pid" 2>/dev/null || true; fi
  rm -rf "$directory"
}
trap cleanup EXIT INT TERM
if [ "$mode" = "quiet" ]; then
  printf '{"quiet":true}' > "$directory/config.json"
else
  printf '{}' > "$directory/config.json"
fi
"$binary" --data-dir "$directory" --exit-after 45 > "$directory/output.log" 2>&1 &
pid=$!
sleep 8
printf 'mode=%s pid=%s\n' "$mode" "$pid"
printf 'CPU%% RSS_KB ELAPSED\n'
count=0
while [ "$count" -lt 6 ]; do
  ps -p "$pid" -o %cpu=,rss=,etime=
  sleep 3
  count=$((count + 1))
done
cat "$directory/output.log"

