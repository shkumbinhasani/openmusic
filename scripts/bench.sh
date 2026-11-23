#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
runs="${RUNS:-5}"
settle="${SETTLE:-20}"
window="${SAMPLE:-30}"
album="${ALBUM:-spotify:album:1b5KESuOMQomF3wntoLAsk}"
openmusic="${OPENMUSIC_APP:-$PWD/target/OpenMusic.app}"
spotify="${SPOTIFY_APP:-/Applications/Spotify.app}"
work="$(mktemp -d)"
debug="$work/debug"
mkdir -p "$debug"
trap 'rm -rf "$work"' EXIT

swiftc -O scripts/bench/launch.swift -o "$work/launch" 2>/dev/null

pids() {
    pgrep -f "^$1/" || true
}

quit() {
    pkill -f "^$1/" 2>/dev/null || true
    tries=0
    while [ -n "$(pids "$1")" ] && [ "$tries" -lt 50 ]; do
        sleep 0.2
        tries=$((tries + 1))
    done
    pkill -9 -f "^$1/" 2>/dev/null || true
    sleep 1
}

footprint_mb() {
    list="$(pids "$1")"
    [ -n "$list" ] || { echo 0; return; }
    footprint -f bytes $list 2>/dev/null |
        awk '/Footprint: [0-9]+ B/ { for (i = 1; i <= NF; i++) if ($i == "Footprint:") sum += $(i + 1) } END { printf "%d", sum / 1048576 }'
}

cpu_seconds() {
    list="$(pids "$1" | paste -sd, -)"
    [ -n "$list" ] || { echo 0; return; }
    ps -o time= -p "$list" | awk '{
        n = split($1, part, ":")
        seconds = part[n] + (n > 1 ? part[n - 1] * 60 : 0) + (n > 2 ? part[n - 2] * 3600 : 0)
        sum += seconds
    } END { printf "%.2f", sum }'
}

cpu_percent() {
    before="$(cpu_seconds "$1")"
    sleep "$window"
    after="$(cpu_seconds "$1")"
    echo "$before $after $window" | awk '{ printf "%.1f", ($2 - $1) / $3 * 100 }'
}

median() {
    tr ' ' '\n' | grep -v '^$' | sort -n | awk '{ value[NR] = $1 } END { print value[int((NR + 1) / 2)] }'
}

processes() {
    pids "$1" | wc -l | tr -d ' '
}

launches() {
    bundle="$1"
    owner="$2"
    shift 2
    times=""
    run=0
    while [ "$run" -lt "$runs" ]; do
        quit "$bundle"
        times="$times $("$work/launch" "$bundle" "$owner" "$@")"
        run=$((run + 1))
    done
    echo "$times" | median
}

openmusic_command() {
    printf '%s\n' "$1" > "$debug/command"
    sleep "${2:-1}"
}

openmusic_state() {
    rm -f "$debug/state.txt"
    openmusic_command state 1
    cat "$debug/state.txt" 2>/dev/null || true
}

measure_openmusic() {
    [ -d "$openmusic" ] || { echo "OpenMusic not found at $openmusic; run scripts/bundle-macos.sh" >&2; exit 1; }
    om_size="$(du -sm "$openmusic" | cut -f1)"
    om_launch="$(launches "$openmusic" OpenMusic "OPENMUSIC_DEBUG_DIR=$debug")"
    sleep "$settle"
    case "$(openmusic_state)" in
        account=true*) ;;
        *) echo "OpenMusic is not signed in. Sign in once, allow Keychain access, then run again." >&2; quit "$openmusic"; exit 1 ;;
    esac
    om_idle="$(footprint_mb "$openmusic")"
    om_idle_cpu="$(cpu_percent "$openmusic")"
    om_procs="$(processes "$openmusic")"
    openmusic_command "play $album 0" "$settle"
    case "$(openmusic_state)" in
        *playing=true*) ;;
        *) echo "OpenMusic did not start playback." >&2 ;;
    esac
    om_play="$(footprint_mb "$openmusic")"
    om_play_cpu="$(cpu_percent "$openmusic")"
    openmusic_command toggle 2
    quit "$openmusic"
}

spotify_script() {
    osascript -e "tell application \"Spotify\" to $1" 2>/dev/null || true
}

measure_spotify() {
    [ -d "$spotify" ] || { echo "Spotify not found at $spotify; skipping" >&2; return 1; }
    was_running="$(pids "$spotify")"
    if [ -n "$was_running" ]; then
        resume_track="$(spotify_script 'id of current track')"
        resume_position="$(spotify_script 'player position')"
    fi
    sp_size="$(du -sm "$spotify" | cut -f1)"
    sp_launch="$(launches "$spotify" Spotify)"
    sleep "$settle"
    sp_idle="$(footprint_mb "$spotify")"
    sp_idle_cpu="$(cpu_percent "$spotify")"
    sp_procs="$(processes "$spotify")"
    spotify_script "play track \"$album\""
    sleep "$settle"
    sp_play="$(footprint_mb "$spotify")"
    sp_play_cpu="$(cpu_percent "$spotify")"
    spotify_script pause
    if [ -n "$was_running" ] && [ -n "${resume_track:-}" ]; then
        spotify_script "play track \"$resume_track\""
        sleep 2
        spotify_script pause
        spotify_script "set player position to ${resume_position:-0}"
    else
        quit "$spotify"
    fi
}

measure_openmusic
if measure_spotify; then
    have_spotify=1
else
    have_spotify=0
fi

macos="$(sw_vers -productVersion)"
chip="$(sysctl -n machdep.cpu.brand_string)"
memory="$(($(sysctl -n hw.memsize) / 1073741824))"
spotify_version="$(defaults read "$spotify/Contents/Info.plist" CFBundleShortVersionString 2>/dev/null || echo unknown)"

printf '\nmacOS %s, %s, %s GB. Spotify %s. Median of %s launches; memory is the summed physical footprint of every process; CPU is averaged over %s s.\n\n' \
    "$macos" "$chip" "$memory" "$spotify_version" "$runs" "$window"
printf '| | OpenMusic | Spotify |\n|---|---:|---:|\n'
row() {
    if [ "$have_spotify" = 1 ]; then
        printf '| %s | %s | %s |\n' "$1" "$2" "$3"
    else
        printf '| %s | %s | n/a |\n' "$1" "$2"
    fi
}
row "App size" "${om_size} MB" "${sp_size:-} MB"
row "Launch to window" "${om_launch} ms" "${sp_launch:-} ms"
row "Processes" "$om_procs" "${sp_procs:-}"
row "Memory, idle" "${om_idle} MB" "${sp_idle:-} MB"
row "Memory, playing" "${om_play} MB" "${sp_play:-} MB"
row "CPU, idle" "${om_idle_cpu}%" "${sp_idle_cpu:-}%"
row "CPU, playing" "${om_play_cpu}%" "${sp_play_cpu:-}%"
