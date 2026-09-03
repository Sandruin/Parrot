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

# Runs a dispatcher in classic form, or as the given Lua expression when the compositor parses Lua.
dispatch() {
    local reply
    reply=$(hyprctl dispatch "$1" 2>&1 || true)
    [ "$reply" = "ok" ] && return 0
    case $reply in
        *hl.dispatch*) hyprctl eval "return hl.dispatch($2)" >/dev/null ;;
        *) echo "$reply" >&2; return 1 ;;
    esac
}

# Address, pid and logical geometry of the app's main window, empty when it is not running.
find_window() {
    hyprctl -j clients | jq -r '.[] | select(.class == "macro-recorder" and .mapped) | "\(.address) \(.pid) \(.at[0]) \(.at[1]) \(.size[0]) \(.size[1])"' | head -n1
}

started=0
win=$(find_window)
if [ -z "$win" ]; then
    [ -x "$exe" ] || { echo "exe not found: $exe" >&2; exit 1; }
    # Hyprland starts the app from its own environment, so the variables the app reads are forwarded.
    envs=""
    for name in MACRO_DEMO_DOC MACRO_FAKE_ENGINE MACRO_OVERLAY_CAPTURABLE MACRO_OCR_LANG RUST_LOG; do
        [ -n "${!name:-}" ] && envs="$envs $name=${!name}"
    done
    cmd="[float; size 960 680; center] env$envs $(realpath "$exe") $*"
    dispatch "exec $cmd" "hl.dsp.exec_cmd(\"$cmd\")"
    started=1
    for _ in $(seq 1 100); do
        sleep 0.25
        win=$(find_window)
        [ -n "$win" ] && break
    done
    [ -n "$win" ] || { echo "the app never got a window" >&2; exit 1; }
    sleep "$wait_seconds"
fi

read -r addr pid x y w h <<<"$win"
dispatch "focuswindow address:$addr" "hl.dsp.focus({ window = \"address:$addr\" })"
sleep 0.3
mkdir -p "$(dirname "$out")"
grim -g "$x,$y ${w}x${h}" "$out"
echo "saved $out (${w}x${h} logical at $x,$y)"

if [ "$started" = 1 ] && [ "$keep" = 0 ]; then
    dispatch "closewindow address:$addr" "hl.dsp.window.close({ window = \"address:$addr\" })" || kill "$pid"
fi
