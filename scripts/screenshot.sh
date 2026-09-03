#!/usr/bin/env bash
# Launches the app floating on Hyprland (or attaches to a running instance), screenshots its window with grim and optionally closes it.
# Usage: scripts/screenshot.sh [-o out.png] [-e target/debug/macro-recorder] [-w seconds] [-k] [-- app args]
set -euo pipefail

out=screenshots/app.png
exe=target/debug/macro-recorder
wait_seconds=4
keep=0
while getopts "o:e:w:k" opt; do
    case $opt in
        o) out=$OPTARG ;;
        e) exe=$OPTARG ;;
        w) wait_seconds=$OPTARG ;;
        k) keep=1 ;;
        *) exit 2 ;;
    esac
done
shift $((OPTIND - 1))

# Address and logical geometry of the app's main window, empty when it is not running.
find_window() {
    hyprctl -j clients | jq -r '.[] | select(.class == "macro-recorder" and .mapped) | "\(.address) \(.at[0]) \(.at[1]) \(.size[0]) \(.size[1])"' | head -n1
}

started=0
win=$(find_window)
if [ -z "$win" ]; then
    [ -x "$exe" ] || { echo "exe not found: $exe" >&2; exit 1; }
    hyprctl dispatch exec "[float; size 960 680; center] $(realpath "$exe") $*" >/dev/null
    started=1
    for _ in $(seq 1 100); do
        sleep 0.25
        win=$(find_window)
        [ -n "$win" ] && break
    done
    [ -n "$win" ] || { echo "the app never got a window" >&2; exit 1; }
    sleep "$wait_seconds"
fi

read -r addr x y w h <<<"$win"
hyprctl dispatch focuswindow "address:$addr" >/dev/null
sleep 0.3
mkdir -p "$(dirname "$out")"
grim -g "$x,$y ${w}x${h}" "$out"
echo "saved $out (${w}x${h} logical at $x,$y)"

if [ "$started" = 1 ] && [ "$keep" = 0 ]; then
    hyprctl dispatch closewindow "address:$addr" >/dev/null
fi
