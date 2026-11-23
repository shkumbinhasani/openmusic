#!/bin/sh
set -eu

directory="${1:-/tmp/openmusic-tour}"
artist="${ARTIST:-spotify:artist:7G1GBhoKtEPnP86X2PvEYO}"
album="${ALBUM:-spotify:album:1b5KESuOMQomF3wntoLAsk}"
playlist="${PLAYLIST:-}"

send() {
    printf '%s\n' "$1" > "$directory/command"
    sleep "${2:-3}"
}

snap() {
    rm -f "$directory/$1.done"
    send "snapshot $1" 1
    tries=0
    while [ ! -f "$directory/$1.done" ] && [ "$tries" -lt 20 ]; do
        sleep 0.5
        tries=$((tries + 1))
    done
    printf '%s: %s\n' "$1" "$(cat "$directory/$1.done" 2>/dev/null || echo timeout)"
}

send home 3
snap home
send "search ${SEARCH:-nina simone}" 5
snap search
send "artist $artist" 6
snap artist
send "open $album" 5
snap album
if [ -n "$playlist" ]; then
    send "open $playlist" 5
    snap playlist
fi
send lyrics 4
snap lyrics
send "size 900 640" 2
snap narrow
send "size 1120 760" 2
send state 1
cat "$directory/state.txt"
echo
