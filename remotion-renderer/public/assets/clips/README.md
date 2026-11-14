# Test clips

Small clips for the video-clip fixtures (`fixtures/video-clips-*.json`). All of them are generated
by ffmpeg's own test sources (`lavfi`), so they contain no third-party footage or sound and are free
to use. Each is under 1 MB.

| File | Size, rate, length | Made with |
| --- | --- | --- |
| `counter-6s.mp4` | 640x360, 30 fps, 6 s, no sound | `testsrc2`, the frame number in large text, and a frame code strip (below) |
| `mandelbrot-2s.mp4` | 480x270, 30 fps, 2 s, no sound | `mandelbrot` zooming in |
| `life-tone-4s.mp4` | 320x180, 30 fps, 4 s, 660 Hz tone | `life` (Game of Life) + `sine` |
| `pip-cellauto-2s.webm` | 480x270, 30 fps, 2 s, no sound, VP9 | `cellauto` rule 110 |

**Frame code strip** (`counter-6s.mp4`): the top 40 px hold 10 blocks of 64 px; block `i` (from the
left) is white when bit `i` of the frame number is 1. A script can read the source frame shown in a
render by sampling the block centers, which is how the frame accuracy of trim / speed / loop /
freeze is checked.

Commands (ffmpeg 8, macOS font path):

```bash
ffmpeg -f lavfi -i "testsrc2=s=640x360:r=30:d=6" -f lavfi -i "color=c=black:s=640x40:r=30:d=6" \
  -filter_complex "[1:v]geq=lum='255*mod(floor(N/pow(2,floor(X/64))),2)':cb=128:cr=128[code];[0:v][code]overlay=0:0,drawtext=fontfile='/System/Library/Fonts/Supplemental/Arial Bold.ttf':text='%{frame_num}':start_number=0:fontsize=96:fontcolor=white:box=1:boxcolor=black@0.6:x=(w-tw)/2:y=h-th-24[v]" \
  -map "[v]" -c:v libx264 -preset slow -crf 26 -pix_fmt yuv420p -movflags +faststart counter-6s.mp4
ffmpeg -f lavfi -i "mandelbrot=s=480x270:r=30:maxiter=256:end_scale=0.3:end_pts=2" -t 2 \
  -c:v libx264 -preset slow -crf 28 -pix_fmt yuv420p -movflags +faststart mandelbrot-2s.mp4
ffmpeg -f lavfi -i "life=s=320x180:r=30:mold=10:ratio=0.3:death_color=#1e1b4b:life_color=#a5b4fc" \
  -f lavfi -i "sine=f=660:sample_rate=48000:d=4,volume=0.5" -t 4 -c:v libx264 -preset slow -crf 30 \
  -pix_fmt yuv420p -c:a aac -b:a 96k -ac 2 -shortest -movflags +faststart life-tone-4s.mp4
ffmpeg -f lavfi -i "cellauto=s=480x270:r=30:rule=110:full=0,format=yuv420p" -t 2 \
  -c:v libvpx-vp9 -b:v 300k -deadline good pip-cellauto-2s.webm
```

The `life` pattern is random, so a rebuilt `life-tone-4s.mp4` differs from the committed one.
