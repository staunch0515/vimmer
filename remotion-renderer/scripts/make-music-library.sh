#!/bin/bash
# Synthesizes the placeholder music library (public/assets/music/library/*.m4a) with ffmpeg
# alone: no samples, nothing downloaded. See public/assets/music/library/README.md, which also
# explains how to replace these with real tracks (no code change needed).
#
# How the tracks loop seamlessly: every sound is a function of time that repeats exactly every
# loop. Struck notes and drums are written as "time since the strike" (mod(t - at, period)), so a
# note rings into the next bar and its tail wraps around the loop like the real thing; held
# chords are free-running sines whose frequencies are rounded so that a whole number of periods
# fits the loop. Each track is rendered for two loops and the second one is kept: it starts with
# the echoes and note tails of the first, exactly as it does when it follows itself.
#
# Loudness: each track is measured (EBU R128) and brought to -18 LUFS, then a limiter keeps the
# peaks below -3 dBFS (true peak stays below -1 dBTP after AAC). Deterministic: running it again
# writes the same files.
set -euo pipefail
cd "$(dirname "$0")/../public/assets/music"
mkdir -p library
cd library
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
SR=44100
TARGET_LUFS=-18
FF="ffmpeg -y -hide_banner -loglevel error"

num() { awk "BEGIN{printf \"%.9f\", $1}"; }

# tempo BPM BARS: beat (B), bar (BAR) and loop (L) length in seconds, loop length in samples (LS).
tempo() {
  B=$(num "60/$1")
  BAR=$(num "240/$1")
  L=$(num "240*$2/$1")
  LS=$(awk "BEGIN{printf \"%d\", 240*$2/$1*$SR+0.5}")
}

# fq HZ: the nearest frequency with a whole number of periods in the loop.
fq() { awk -v f="$1" -v l="$L" 'BEGIN{printf "%.6f", int(f*l+0.5)/l}'; }

# sel INDEX V0 V1 ...: the value at INDEX (nested ifs; the last value is the fallback).
sel() {
  local idx=$1 out="" close="" i=0
  shift
  local n=$#
  for v in "$@"; do
    if [ $i -lt $((n - 1)) ]; then
      out="${out}if(eq($idx,$i),$v,"
      close="$close)"
    else
      out="$out$v"
    fi
    i=$((i + 1))
  done
  printf '%s%s' "$out" "$close"
}

# tone CHORD K MULT [q]: tone K of chord CHORD (an expression) from the chord table CH (one
# chord per bar, four bars per cycle), times MULT; with q, rounded to fit the loop.
tone() {
  local idx=$1 k=$2 mult=$3 quant=${4:-} vals=() c f
  for c in "${CH[@]}"; do
    f=$(echo "$c" | awk -v k=$((k + 1)) -v m="$mult" '{printf "%.4f", $k*m}')
    [ -n "$quant" ] && f=$(fq "$f")
    vals+=("$f")
  done
  sel "$idx" "${vals[@]}"
}

# pad VOICE AMP MULT [VOICES]: held chords from CH. Each bar's chord fades in over XF seconds
# while the previous one fades out. VOICE is a timbre with @ for the phase 2*PI*f*t.
pad() {
  local voice=$1 amp=$2 mult=$3 nv=${4:-4} sum="0" k fc fp
  for k in $(seq 0 $((nv - 1))); do
    fc=$(tone 'ld(6)' "$k" "$mult" q)
    fp=$(tone 'ld(7)' "$k" "$mult" q)
    sum="$sum+ld(5)*(${voice//@/2*PI*$fc*t})+(1-ld(5))*(${voice//@/2*PI*$fp*t})"
  done
  printf '(st(5,pow(sin(PI/2*min(1,mod(t,%s)/%s)),2));st(6,mod(floor(t/%s+1e-7),4));st(7,mod(floor(t/%s+1e-7)+3,4));%s*(%s))' \
    "$BAR" "$XF" "$BAR" "$BAR" "$amp" "$sum"
}

# note AT PERIOD AMP K MULT ENV VOICE: tone K of the chord playing at the strike, struck AT
# seconds into every PERIOD. In ENV, ld(3) is the time since the strike; VOICE uses @ for the
# phase 2*PI*f*(time since the strike), so every note starts at phase 0.
note() {
  local at=$1 period=$2 amp=$3 k=$4 mult=$5 env=$6 voice=$7
  printf '(st(3,mod(t-%s,%s));st(8,mod(floor((t-ld(3))/%s+1e-6),4));st(4,%s);%s*(%s)*(%s))' \
    "$at" "$period" "$BAR" "$(tone 'ld(8)' "$k" "$mult")" "$amp" "$env" "${voice//@/2*PI*ld(4)*ld(3)}"
}

# perc AT PERIOD AMP BODY: a drum hit; U in BODY is the time since the hit.
perc() { printf '(st(3,mod(t-%s,%s));%s*(%s))' "$1" "$2" "$3" "${4//U/ld(3)}"; }

# bass AT PERIOD AMP ROOTS ENV VOICE: like note, from the root table ROOTS instead of CH.
bass() {
  local saved=("${CH[@]}")
  CH=()
  for r in $4; do CH+=("$r"); done
  note "$1" "$2" "$3" 0 1 "$5" "$6"
  CH=("${saved[@]}")
}

KICK='sin(2*PI*(50*U+70*0.03*(1-exp(-U/0.03))))*exp(-U/0.14)'
NOISE='(random(9)*2-1)'
# A soft attack, so no note starts with a jump.
att() { echo "min(1,ld(3)/$1)"; }

# finish NAME GRAPH INPUT...: renders two loops through GRAPH (which ends in [m]), measures the
# second loop, sets its loudness, limits the peaks and writes NAME.m4a.
finish() {
  local name=$1 graph=$2
  shift 2
  local inputs=() e
  for e in "$@"; do inputs+=(-f lavfi -i "aevalsrc='$e':s=$SR:d=$(num "2*$L+0.05")"); done
  $FF "${inputs[@]}" -filter_complex "$graph" -map "[m]" -c:a pcm_f32le "$TMP/$name.wav"
  local trim="atrim=start_sample=$LS:end_sample=$((2 * LS))" lufs gain
  lufs=$(ffmpeg -hide_banner -nostats -i "$TMP/$name.wav" -af "$trim,ebur128" -f null - 2>&1 |
    awk '$1=="I:"{v=$2} END{print v}')
  gain=$(num "$TARGET_LUFS - ($lufs)")
  $FF -i "$TMP/$name.wav" \
    -af "volume=${gain}dB,alimiter=limit=0.708:attack=5:release=80:level=disabled:latency=1,$trim,asetpts=N/SR/TB" \
    -ac 2 -ar $SR -c:a aac -b:a 128k -map_metadata -1 -fflags +bitexact -flags:a +bitexact "$name.m4a"
  echo "$name.m4a: loop $(num "$L*1000") ms, raw $lufs LUFS, gain ${gain} dB"
}

# Stereo from mono inputs.
ST='pan=stereo|c0=c0|c1=c0'
# A small room: a few early reflections.
ROOM='aecho=0.9:0.85:37|61|89|127:0.3|0.24|0.18|0.12'

# ---- calm: 75 BPM, Cmaj7 Am7 Fmaj7 G6. A slow, breathing pad and a soft arpeggio of eighth notes
# (beats a little louder), echoed a dotted eighth later.
tempo 75 16
XF=1.4
CH=("261.63 329.63 392.00 493.88" "220.00 261.63 329.63 392.00" "174.61 220.00 261.63 329.63" "196.00 246.94 293.66 329.63")
BREATH="(0.8+0.2*sin(2*PI*t/(2*$BAR)))"
PADV='sin(@)+0.18*sin(2*@)+0.05*sin(3*@)'
PAD_L="$BREATH*$(pad "$PADV" 0.10 1)"
PAD_R="$BREATH*$(pad "$PADV" 0.10 1.003)"
CH_SAVED=("${CH[@]}")
CH=("130.81" "110.00" "87.31" "98.00")
LOWV='sin(@)+0.1*sin(2*@)'
PAD_L="$PAD_L+$(pad "$LOWV" 0.16 1 1)"
PAD_R="$PAD_R+$(pad "$LOWV" 0.16 1 1)"
CH=("${CH_SAVED[@]}")
ARP=0
PATTERN=(0 1 2 3 2 1 2 3)
for s in 0 1 2 3 4 5 6 7; do
  amp=$([ $((s % 2)) -eq 0 ] && echo 0.11 || echo 0.06)
  ARP="$ARP+$(note "$(num "$s*$B/2")" "$BAR" "$amp" "${PATTERN[$s]}" 2 "$(att 0.006)*exp(-ld(3)/0.45)" 'sin(@)+0.3*exp(-ld(3)/0.15)*sin(2*@)')"
done
finish calm-drift \
  "[0:a]lowpass=f=2400[p];[1:a]lowpass=f=3200,aecho=0.8:0.9:$(num "$B*750"):0.3,$ST[a];[p][a]amix=inputs=2:normalize=0,$ROOM[m]" \
  "$PAD_L|$PAD_R" "$ARP"

# ---- upbeat: 120 BPM, C G Am F. Pumping chords (ducked on every beat, like a sidechain), off-beat
# bass, a plucked hook, four-on-the-floor kick, claps on 2 and 4, off-beat hats.
tempo 120 24
XF=0.03
CH=("261.63 329.63 392.00 523.25" "246.94 293.66 392.00 493.88" "220.00 261.63 329.63 440.00" "220.00 261.63 349.23 440.00")
PUMP="(1-0.7*exp(-mod(t,$B)/0.12)*min(1,mod(t,$B)/0.006))"
SAW='sin(@)+0.5*sin(2*@)+0.33*sin(3*@)+0.25*sin(4*@)+0.2*sin(5*@)'
CHORDS_L="$PUMP*$(pad "$SAW" 0.05 1)"
CHORDS_R="$PUMP*$(pad "$SAW" 0.05 1.004)"
MEL=0
for s in 0 1 2 3 4 5 6 7; do
  amp=$([ $((s % 2)) -eq 1 ] && echo 0.30 || echo 0.14)
  MEL="$MEL+$(bass "$(num "$s*$B/2")" "$BAR" "$amp" "130.81 98.00 110.00 87.31" "$(att 0.004)*exp(-ld(3)/0.12)" 'sin(@)+0.45*sin(2*@)+0.2*sin(3*@)')"
done
HOOK=0
PATTERN=(3 2 1 2 3 2 0 1)
for s in 0 1 2 3 4 5 6 7; do
  [ "$s" -eq 5 ] && continue
  HOOK="$HOOK+$(note "$(num "$s*$B/2")" "$BAR" 0.07 "${PATTERN[$s]}" 2 "$(att 0.003)*exp(-ld(3)/0.13)" 'sin(@)+0.4*sin(2*@)*exp(-ld(3)/0.05)')"
done
DRUMS="$(perc 0 "$B" 0.9 "$KICK")+$(perc "$B" "$BAR" 0.25 'sin(2*PI*190*U)*exp(-U/0.05)')+$(perc "$(num "3*$B")" "$BAR" 0.25 'sin(2*PI*190*U)*exp(-U/0.05)')"
HATS="$(perc "$(num "$B/2")" "$B" 0.22 "$NOISE*exp(-U/0.03)")+$(perc "$(num "$B/4")" "$(num "$B/2")" 0.05 "$NOISE*exp(-U/0.012)")"
CLAP="$(perc "$B" "$(num "2*$B")" 0.5 "$NOISE*(exp(-U/0.09)+0.6*exp(-mod(U,0.011)/0.004)*lt(U,0.033))")"
finish upbeat-sunny \
  "[0:a]lowpass=f=2600[c];[1:a]lowpass=f=1800,$ST[b];[2:a]aecho=0.8:0.85:$(num "$B*750"):0.28,lowpass=f=5000,$ST[h];[3:a]$ST[k];[4:a]highpass=f=7000,$ST[t];[5:a]bandpass=f=1500:width_type=o:w=2,$ST[cl];[c][b][h][k][t][cl]amix=inputs=6:normalize=0,$ROOM[m]" \
  "$CHORDS_L|$CHORDS_R" "$MEL" "$HOOK" "$DRUMS" "$HATS" "$CLAP"

# ---- tense: 90 BPM, A minor. A low A/E drone with a slow filter-like swell, tremolo strings on
# Am F Dm E, a muted sixteenth pulse on A, a heartbeat kick on every beat and a ticking clock.
tempo 90 16
XF=1.5
CH=("220.00 261.63 329.63 440.00" "174.61 220.00 261.63 349.23" "146.83 220.00 293.66 349.23" "164.81 207.65 246.94 329.63")
A1=$(fq 55)
E2=$(fq 82.41)
DRONEV() { echo "sin(2*PI*$1*t)+0.5*sin(4*PI*$1*t)+0.3*sin(6*PI*$1*t)+0.2*sin(8*PI*$1*t)"; }
SWELL="(0.75+0.25*sin(2*PI*t/(4*$BAR)))"
TREM="(0.7+0.3*sin(2*PI*t/($B/4)))"
STRV='sin(@)+0.3*sin(2*@)+0.1*sin(3*@)'
DRONE_L="$SWELL*0.12*($(DRONEV "$A1")+0.7*($(DRONEV "$E2")))+$TREM*$(pad "$STRV" 0.04 1)"
DRONE_R="$SWELL*0.12*($(DRONEV "$(fq 55.15)")+0.7*($(DRONEV "$(fq 82.6)")))+$TREM*$(pad "$STRV" 0.04 1.004)"
PULSE=0
for s in 0 1 2 3; do
  amp=$([ "$s" -eq 0 ] && echo 0.3 || echo 0.16)
  PULSE="$PULSE+$(bass "$(num "$s*$B/4")" "$B" "$amp" "110.00 110.00 110.00 110.00" "$(att 0.003)*exp(-ld(3)/0.05)" 'sin(@)+0.5*sin(2*@)+0.3*sin(3*@)')"
done
HEART="$(perc 0 "$B" 0.8 'sin(2*PI*(42*U+60*0.03*(1-exp(-U/0.03))))*exp(-U/0.16)')+$(perc "$(num "$B*0.28")" "$BAR" 0.45 'sin(2*PI*(42*U+60*0.03*(1-exp(-U/0.03))))*exp(-U/0.12)')"
TICK="$(perc 0 "$(num "$B/2")" 0.05 'sin(2*PI*3100*U)*exp(-U/0.004)')"
finish tense-pulse \
  "[0:a]lowpass=f=1500[d];[1:a]lowpass=f=900,$ST[p];[2:a]$ST[k];[3:a]aecho=0.8:0.8:$(num "$B*500"):0.35,$ST[t];[d][p][k][t]amix=inputs=4:normalize=0,$ROOM[m]" \
  "$DRONE_L|$DRONE_R" "$PULSE" "$HEART" "$TICK"

# ---- warm: 80 BPM, F Dm Bb C. Electric-piano chords on 1, 2-and, 3 and 4-and, a soft pad, half-note
# bass, a soft kick on 1 and 3, a rim on 2 and 4 and a quiet shaker.
tempo 80 16
XF=1.0
CH=("174.61 220.00 261.63 349.23" "146.83 220.00 293.66 349.23" "174.61 233.08 293.66 349.23" "196.00 261.63 329.63 392.00")
EPV='sin(@)+0.2*sin(2*@)+0.12*exp(-ld(3)/0.06)*sin(7*@)'
KEYS=0
HITS=(0 1.5 2 3.5)
AMPS=(0.12 0.07 0.09 0.06)
for i in 0 1 2 3; do
  for k in 0 1 2 3; do
    KEYS="$KEYS+$(note "$(num "${HITS[$i]}*$B")" "$BAR" "${AMPS[$i]}" "$k" 1 "$(att 0.004)*exp(-ld(3)/0.7)" "$EPV")"
  done
done
SOFT='sin(@)+0.1*sin(2*@)'
PAD_L="$(pad "$SOFT" 0.035 2)"
PAD_R="$(pad "$SOFT" 0.035 2.003)"
LOW=0
for h in 0 2; do
  LOW="$LOW+$(bass "$(num "$h*$B")" "$BAR" 0.32 "87.31 73.42 58.27 65.41" "$(att 0.008)*exp(-ld(3)/0.55)" 'sin(@)+0.3*sin(2*@)+0.1*sin(3*@)')"
done
DRUMS="$(perc 0 "$(num "2*$B")" 0.55 "$KICK")+$(perc "$B" "$(num "2*$B")" 0.12 'sin(2*PI*420*U)*exp(-U/0.02)')"
SHAKE="$(perc 0 "$(num "$B/2")" 0.07 "$NOISE*min(1,U/0.012)*exp(-U/0.04)")+$(perc "$B" "$(num "2*$B")" 0.18 "$NOISE*exp(-U/0.05)")"
finish warm-hearth \
  "[0:a]lowpass=f=4000,aecho=0.8:0.85:$(num "$B*1000"):0.18,$ST[k];[1:a][2:a]amerge=inputs=2,pan=stereo|c0=c0|c1=c1[p];[3:a]lowpass=f=900,$ST[b];[4:a]$ST[d];[5:a]bandpass=f=5000:width_type=o:w=2,$ST[s];[k][p][b][d][s]amix=inputs=5:normalize=0,$ROOM[m]" \
  "$KEYS" "$PAD_L" "$PAD_R" "$LOW" "$DRUMS" "$SHAKE"

# ---- epic: 100 BPM, Dm Bb F C. Brass-like chords that swell through each 4-bar phrase and fall
# back on the next downbeat, a high string octave that grows with them, low taiko-like drums on
# 1, 2-and, 3 and 4, a snare roll building through every fourth bar and a crash on each phrase.
tempo 100 20
XF=0.25
CH=("146.83 174.61 220.00 293.66" "116.54 146.83 174.61 233.08" "130.81 174.61 220.00 261.63" "130.81 164.81 196.00 261.63")
PH=$(num "4*$BAR")
PP="(mod(t,$PH)/$PH)"
RISE="if(lt($PP,0.95),0.5+0.5*$PP*$PP,0.5+0.45125*(1-$PP)/0.05)"
BRASS='sin(@)+0.6*sin(2*@)+0.45*sin(3*@)+0.3*sin(4*@)+0.2*sin(5*@)+0.12*sin(6*@)'
STR='sin(@)+0.15*sin(2*@)'
ROOTS_CH=("73.42" "58.27" "87.31" "65.41")
CH_SAVED=("${CH[@]}")
CH=("${ROOTS_CH[@]}")
LOW_L="$(pad 'sin(@)+0.4*sin(2*@)+0.2*sin(3*@)' 0.22 1 1)"
CH=("${CH_SAVED[@]}")
BR_L="$RISE*($(pad "$BRASS" 0.05 1)+$PP*$(pad "$STR" 0.05 2))+$LOW_L"
BR_R="$RISE*($(pad "$BRASS" 0.05 1.005)+$PP*$(pad "$STR" 0.05 2.004))+$LOW_L"
TOM='sin(2*PI*(45*U+60*0.05*(1-exp(-U/0.05))))*exp(-U/0.3)'
TOMS="$(perc 0 "$BAR" 0.9 "$TOM")+$(perc "$(num "1.5*$B")" "$BAR" 0.55 "$TOM")+$(perc "$(num "2*$B")" "$BAR" 0.75 "$TOM")+$(perc "$(num "3*$B")" "$BAR" 0.6 "$TOM")+$(perc 0 "$PH" 0.6 'sin(2*PI*(32*U+40*0.08*(1-exp(-U/0.08))))*exp(-U/0.8)')"
# The roll: sixteenths, only in the last bar of each phrase (decided at the strike, so tails ring
# on), getting louder through the bar.
ROLLPOS="mod(t-ld(3)+0.001,$PH)"
ROLL="(st(3,mod(t,$(num "$B/4")));if(gte($ROLLPOS,$(num "3*$BAR")),0.35*(0.2+0.8*($ROLLPOS-$(num "3*$BAR"))/$BAR),0)*$NOISE*exp(-ld(3)/0.05))"
CRASH="$(perc 0 "$PH" 0.25 "$NOISE*min(1,U/0.002)*exp(-U/1.4)")"
finish epic-rise \
  "[0:a]lowpass=f=2200,aecho=0.8:0.8:90|160|230:0.3|0.22|0.15[b];[1:a]lowpass=f=1200,$ST[t];[2:a]bandpass=f=1800:width_type=o:w=2,$ST[r];[3:a]highpass=f=5000,$ST[c];[b][t][r][c]amix=inputs=4:normalize=0,$ROOM[m]" \
  "$BR_L|$BR_R" "$TOMS" "$ROLL" "$CRASH"

# ---- playful: 128 BPM, C Am F G. A marimba-like plucked melody on eighths with a rest, staccato
# bass, kick on 1 and 3, a light clap on 2 and 4, hats and a woodblock.
tempo 128 24
CH=("261.63 329.63 392.00 523.25" "220.00 261.63 329.63 440.00" "174.61 220.00 261.63 349.23" "196.00 246.94 293.66 392.00")
MARIMBA='sin(@)+0.5*exp(-ld(3)/0.04)*sin(4*@)+0.2*exp(-ld(3)/0.02)*sin(10*@)'
MEL=0
PATTERN=(0 2 1 3 2 1 3 2)
AMPS=(0.16 0.09 0.12 0.09 0.15 0 0.12 0.1)
for s in 0 1 2 3 4 5 6 7; do
  [ "${AMPS[$s]}" = 0 ] && continue
  MEL="$MEL+$(note "$(num "$s*$B/2")" "$BAR" "${AMPS[$s]}" "${PATTERN[$s]}" 2 "$(att 0.002)*exp(-ld(3)/0.16)" "$MARIMBA")"
done
LOW=0
for h in 0 1 2 3 3.5; do
  LOW="$LOW+$(bass "$(num "$h*$B")" "$BAR" 0.3 "130.81 110.00 87.31 98.00" "$(att 0.003)*exp(-ld(3)/0.08)" 'sin(@)+0.5*sin(2*@)')"
done
DRUMS="$(perc 0 "$(num "2*$B")" 0.8 "$KICK")+$(perc "$(num "$B*2.5")" "$BAR" 0.12 'sin(2*PI*1250*U)*exp(-U/0.018)')+$(perc "$(num "$B*1.75")" "$BAR" 0.09 'sin(2*PI*950*U)*exp(-U/0.018)')"
HATS="$(perc "$(num "$B/2")" "$B" 0.12 "$NOISE*exp(-U/0.025)")+$(perc "$B" "$(num "2*$B")" 0.35 "$NOISE*(exp(-U/0.07)+0.6*exp(-mod(U,0.01)/0.004)*lt(U,0.03))")"
finish playful-hop \
  "[0:a]aecho=0.8:0.8:$(num "$B*750"):0.25,lowpass=f=6000,$ST[mel];[1:a]lowpass=f=1500,$ST[b];[2:a]$ST[d];[3:a]highpass=f=3000,$ST[h];[mel][b][d][h]amix=inputs=4:normalize=0,$ROOM[m]" \
  "$MEL" "$LOW" "$DRUMS" "$HATS"
