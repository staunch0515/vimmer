#!/bin/sh
# Synthesizes the bundled sound effects and the beat test tracks with ffmpeg alone: no samples,
# nothing downloaded. Deterministic (fixed noise seeds), so running it again rewrites the same
# sounds. See public/assets/sfx/README.md and public/assets/music/README.md.
set -eu
cd "$(dirname "$0")/../public/assets"
mkdir -p sfx music
FF="ffmpeg -y -hide_banner -loglevel error"
WAV="-ac 1 -ar 32000 -c:a pcm_s16le -fflags +bitexact -flags:a +bitexact -map_metadata -1"

# click: 2 kHz blip with a 4 ms decay.
$FF -f lavfi -i "aevalsrc='0.9*sin(2*PI*2000*t)*exp(-t/0.004)':s=32000:d=0.04" $WAV sfx/click.wav

# pop: sine falling from 900 to 300 Hz (phase is the integral of the frequency), 30 ms decay.
$FF -f lavfi -i "aevalsrc='0.9*sin(2*PI*(300*t+600*0.025*(1-exp(-t/0.025))))*exp(-t/0.03)*min(1,t/0.001)':s=32000:d=0.15" $WAV sfx/pop.wav

# ding: bell partials at 1320/2640/3960/5280 Hz, 2 ms attack, 350 ms decay (upper partials faster).
$FF -f lavfi -i "aevalsrc='min(1,t/0.002)*(0.55*sin(2*PI*1320*t)*exp(-t/0.35)+0.25*sin(2*PI*2640*t)*exp(-t/0.2)+0.12*sin(2*PI*3960*t)*exp(-t/0.12)+0.06*sin(2*PI*5280*t)*exp(-t/0.08))':s=32000:d=1.2" $WAV sfx/ding.wav

# whoosh: pink noise in two bands, the low band peaking at 0.22 s and the high band at 0.34 s, so
# the sound sweeps up; a 15 ms attack to a third of the peak makes the start audible right away.
$FF -f lavfi -i "anoisesrc=color=pink:seed=7:r=32000:d=0.6" -f lavfi -i "anoisesrc=color=white:seed=11:r=32000:d=0.6" -filter_complex "\
[0:a]bandpass=f=900:width_type=o:w=1.5,volume=eval=frame:volume='2.4*(0.35*min(1,t/0.015)+0.65*pow(sin(PI*min(t,0.44)/0.44),2))*if(gt(t,0.44),exp(-(t-0.44)/0.05),1)'[lo];\
[1:a]bandpass=f=4000:width_type=o:w=1.5,volume=eval=frame:volume='0.9*(0.3*min(1,t/0.015)+0.7*pow(sin(PI*min(t,0.56)/0.56),2))*if(gt(t,0.56),exp(-(t-0.56)/0.02),1)'[hi];\
[lo][hi]amix=inputs=2:normalize=0,asetnsamples=n=64,alimiter=limit=0.9:level=disabled:latency=1[a]" -map "[a]" $WAV sfx/whoosh.wav

# swoosh-down: the reverse sweep (high band first, low band last), shorter decay at the end.
$FF -f lavfi -i "anoisesrc=color=white:seed=13:r=32000:d=0.7" -f lavfi -i "anoisesrc=color=pink:seed=17:r=32000:d=0.7" -filter_complex "\
[0:a]bandpass=f=5000:width_type=o:w=1.5,volume=eval=frame:volume='1.0*(0.4*min(1,t/0.01)+0.6*pow(sin(PI*min(t,0.3)/0.3),2))*if(gt(t,0.3),exp(-(t-0.3)/0.04),1)'[hi];\
[1:a]bandpass=f=700:width_type=o:w=1.5,volume=eval=frame:volume='2.4*pow(sin(PI*min(t,0.6)/0.6),2)*if(gt(t,0.6),exp(-(t-0.6)/0.03),1)'[lo];\
[hi][lo]amix=inputs=2:normalize=0,asetnsamples=n=64,alimiter=limit=0.9:level=disabled:latency=1[a]" -map "[a]" $WAV sfx/swoosh-down.wav

# riser: 2 s of noise plus a sine gliding 200 -> 1400 Hz, both swelling to full level at the end
# (place it 2 s before the moment it leads into).
$FF -f lavfi -i "aevalsrc='pow(t/2,2.5)*0.5*sin(2*PI*(200*t+1200*t*t/4))':s=32000:d=2" -f lavfi -i "anoisesrc=color=white:seed=19:r=32000:d=2" -filter_complex "\
[1:a]highpass=f=1500,volume=eval=frame:volume='0.35*pow(t/2,3)'[n];\
[0:a][n]amix=inputs=2:normalize=0,afade=t=out:st=1.985:d=0.015[a]" -map "[a]" $WAV sfx/riser.wav

# ---- Beat test tracks: a kick on every beat (accented on each bar's first beat), a hat on every
# eighth, and a quiet bass note per bar so it is not only percussion. Kick: sine falling from
# 120 to 50 Hz (phase = integral), 120 ms decay. Ground truth is in music/README.md.
beat_track() { # name bpm first_beat_s seconds
  P=$(echo "60/$2" | bc -l); O=$3; D=$4
  T="(t-$O)"; U="mod($T,$P)"; I="floor($T/$P)"; H="mod($T,$P/2)"
  KICK="if(gte(t,$O),if(eq(mod($I,4),0),1,0.65)*sin(2*PI*(50*$U+70*0.03*(1-exp(-$U/0.03))))*exp(-$U/0.12),0)"
  HAT="if(gte(t,$O),(random(0)*2-1)*exp(-$H/0.012)*if(lt(mod($T,$P),$P/4),0.12,0.2),0)"
  BASS="0.12*sin(2*PI*55*t)*if(gte(t,$O),exp(-mod($T,4*$P)/0.8),0.6)"
  PAD="if(lt(t,$O),0.08*(sin(2*PI*220*t)+sin(2*PI*277.18*t)+sin(2*PI*329.63*t))*min(1,t/0.2),0)"
  $FF -f lavfi -i "aevalsrc='0.8*$KICK+$BASS+$PAD':s=44100:d=$D" \
      -f lavfi -i "aevalsrc='$HAT':s=44100:d=$D" -filter_complex \
      "[1:a]highpass=f=6000[h];[0:a][h]amix=inputs=2:normalize=0[a]" -map "[a]" \
      -ac 1 -c:a aac -b:a 64k -map_metadata -1 "music/$1.m4a"
}
beat_track beat-120bpm 120 0 20
beat_track beat-96bpm-intro 96 0.75 20.75
