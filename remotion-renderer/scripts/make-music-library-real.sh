#!/bin/bash
# Encodes the real (CC0) tracks of the music library from their downloaded source files:
#
#   scripts/make-music-library-real.sh DIR
#
# DIR holds the files exactly as downloaded from OpenGameArt (names in the table below; the
# credits and pages are in public/assets/music/library/README.md). The sources are not in the
# repository; this script makes the library's .m4a files from them, the same way
# make-music-library.sh finishes the synthesized placeholders:
#
# - decoded to 44.1 kHz stereo float, so every track has the same format;
# - measured (EBU R128) and brought to -18 LUFS integrated, then a limiter keeps the sample peaks
#   below -3 dBFS (true peak stays below -1 dBTP after AAC); the level is measured again after the
#   limiter and the gain corrected once, so the result lands on -18 even where the limiter works;
# - AAC 128 kb/s, no metadata, bit-exact flags: running it again writes the same files.
#
# The full length of every track is kept (all fit in 4 MB at 128 kb/s). Only digital silence at
# the very end is cut, down to half a second (The Ancient Legend ends with 3.5 s of silence).
# Happy Clappy Loop is a seamless loop: it is processed as three copies in a row and the middle
# one is kept, so the limiter and the encoder see the loop's own end before its start, as they do
# when the render repeats it. Its source has a small step where the end meets the start (a soft
# click); the step is spread over 5 ms on each side of the join (see encode).
set -euo pipefail
if [ $# -ne 1 ] || [ ! -d "$1" ]; then
  echo "usage: $0 DIR (the downloaded source files)" >&2
  exit 2
fi
SRC=$(cd "$1" && pwd)
cd "$(dirname "$0")/../public/assets/music/library"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
SR=44100
TARGET_LUFS=-18
FF="ffmpeg -y -hide_banner -loglevel error"
LIMIT="alimiter=limit=0.708:attack=5:release=80:level=disabled:latency=1"

num() { awk "BEGIN{printf \"%.9f\", $1}"; }

# lufs FILE [FILTER]: integrated loudness.
lufs() {
  ffmpeg -hide_banner -nostats -i "$1" -af "${2:+$2,}ebur128" -f null - 2>&1 |
    awk '$1=="I:"{v=$2} END{print v}'
}

# encode ID SOURCE END_S LOOP: END_S is where the music ends (empty: the whole file); LOOP 1 for a
# seamless loop.
encode() {
  local id=$1 src=$SRC/$2 end=$3 loop=$4
  [ -f "$src" ] || { echo "missing $src" >&2; exit 1; }
  $FF -i "$src" ${end:+-t $end} -af "aresample=$SR" -ac 2 -c:a pcm_f32le "$TMP/$id.wav"
  local pre="" keep="anull" n
  if [ "$loop" = 1 ]; then
    $FF -i "$TMP/$id.wav" -f f32le "$TMP/$id.raw"
    n=$(($(wc -c <"$TMP/$id.raw") / 8))
    pre="aloop=loop=2:size=$n,"
    keep="atrim=start_sample=$n:end_sample=$((2 * n)),asetpts=N/SR/TB"
    # The source's last sample does not lead into its first (a step of a few percent of full
    # scale at a quiet moment: a soft click). Spread the step over SEAM seconds on each side of
    # the join with a raised-cosine ramp, so the end runs continuously into the start; nothing
    # further than SEAM from the join changes.
    local first last
    first=($(head -c 8 "$TMP/$id.raw" | od -An -t f4))
    last=($(tail -c 8 "$TMP/$id.raw" | od -An -t f4))
    # u: seconds from the nearest join, from the sample number (exact, unlike t).
    local seam=0.005 u ramp
    u="((n-$n*floor(n/$n+0.5))/$SR)"
    ramp="(if(lt($u,-$seam),0,if(gt($u,$seam),1,0.5-0.5*cos(PI*($u+$seam)/(2*$seam))))-gte($u,0))"
    $FF -i "$TMP/$id.wav" -af "${pre}aeval='val(0)+($(num "${first[0]}-(${last[0]})"))*$ramp|val(1)+($(num "${first[1]}-(${last[1]})"))*$ramp',$keep" \
      -c:a pcm_f32le "$TMP/$id.seam.wav"
    mv "$TMP/$id.seam.wav" "$TMP/$id.wav"
  fi
  local l1 g1 l2 g
  l1=$(lufs "$TMP/$id.wav")
  g1=$(num "$TARGET_LUFS - ($l1)")
  # The level after the limiter, at that gain.
  $FF -i "$TMP/$id.wav" -af "${pre}volume=${g1}dB,$LIMIT,$keep" -c:a pcm_f32le "$TMP/$id.1.wav"
  l2=$(lufs "$TMP/$id.1.wav")
  g=$(num "$g1 + $TARGET_LUFS - ($l2)")
  $FF -i "$TMP/$id.wav" -af "${pre}volume=${g}dB,$LIMIT,$keep" \
    -ac 2 -ar $SR -c:a aac -b:a 128k -map_metadata -1 -fflags +bitexact -flags:a +bitexact "$id.m4a"
  echo "$id.m4a: source $l1 LUFS, gain ${g} dB, now $(lufs "$id.m4a") LUFS, $(wc -c <"$id.m4a" | tr -d ' ') bytes"
}

#      id                 source file                       end (s)  loop
encode another-august     calm-013_Another_August_0.mp3     ""       0
encode bright-eyes        warm-bright_eyes.ogg              ""       0
encode tension-theme      tense-tension_theme.wav           ""       0
encode happy-clappy-loop  playful-HappyClappyLoop.wav       ""       1
encode overdrive          upbeat-concept1d_2.ogg            ""       0
encode ancient-legend     epic-the_ancient_legend_2.mp3     183.17   0
