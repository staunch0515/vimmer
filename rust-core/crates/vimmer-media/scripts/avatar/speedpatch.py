"""Speed patches for LivePortrait rendering on Apple-silicon MPS (from the Vimmer avatar speed trial).

- apply(wrapper): exact rewrite of every non-1x1x1 Conv3d in the warping module and the appearance
  extractor as 2D convs (+ an FFT version of the 7x7x7 mask conv), and spectral norm baked into the
  SPADE weights. Same weights, same math (outputs match to ~1e-6).
  IMPORTANT: torch 2.14 MPS Conv3d is numerically wrong for some channel counts (32, 110, 4), so no
  native Conv3d may run on MPS. apply() asserts that none are left.
- CoreMLSpade: the SPADE generator as a CoreML mlprogram (fp16) on the Neural Engine.
- FrameSink: a writer thread that runs SPADE (CoreML) and streams raw RGB frames to ffmpeg, so long
  clips use flat memory and the Neural Engine overlaps the GPU warp of the next frame.
"""
import os, sys, threading, queue, subprocess
import numpy as np
import torch, torch.nn as nn

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import fastwarp  # noqa: E402


def apply(wrapper):
    fastwarp.patch_mask_fft(wrapper.warping_module)  # must come first: patch() would otherwise convert the mask conv
    fastwarp.patch(wrapper.warping_module, "stack")
    assert type(wrapper.warping_module.dense_motion_network.mask).__name__ == "FFTConv3d"
    fastwarp.patch(wrapper.appearance_feature_extractor, "stack")
    left = [n for n, m in list(wrapper.warping_module.named_modules()) + list(wrapper.appearance_feature_extractor.named_modules())
            if isinstance(m, nn.Conv3d) and m.kernel_size != (1, 1, 1)]
    assert not left, f"native Conv3d left (wrong on torch>=2.14 MPS): {left}"
    for m in wrapper.spade_generator.modules():
        if hasattr(m, "weight_orig"):
            torch.nn.utils.remove_spectral_norm(m)


class CoreMLSpade:
    """Input: warped feature 1x256x64x64 float32 numpy. Output: uint8 HxWx3 RGB."""
    def __init__(self, path, units="ALL"):
        import coremltools as ct
        self.m = ct.models.MLModel(path, compute_units=getattr(ct.ComputeUnit, units))

    def __call__(self, feat):
        y = self.m.predict({"x": feat})["img"]  # 1x3xHxW in 0..1
        return (np.clip(y[0].transpose(1, 2, 0), 0, 1) * 255).astype(np.uint8)


class FrameSink:
    """Background writer: optional per-frame transform, then raw RGB to ffmpeg (x264, no audio)."""
    def __init__(self, path, fps=25, transform=None, ffmpeg="ffmpeg", crf=18):
        self.path, self.fps, self.transform, self.ffmpeg, self.crf = path, fps, transform, ffmpeg, crf
        self.q = queue.Queue(maxsize=16); self.proc = None; self.err = None; self.n = 0
        self.t = threading.Thread(target=self._run, daemon=True); self.t.start()

    def put(self, img):
        if self.err:
            raise self.err
        self.q.put(img)

    def _run(self):
        try:
            while True:
                img = self.q.get()
                if img is None:
                    break
                if self.transform is not None:
                    img = self.transform(img)
                if self.proc is None:
                    h, w = img.shape[:2]
                    self.proc = subprocess.Popen(
                        [self.ffmpeg, "-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgb24",
                         "-s", f"{w}x{h}", "-r", str(self.fps), "-i", "-", "-an", "-c:v", "libx264",
                         "-crf", str(self.crf), "-pix_fmt", "yuv420p", "-movflags", "+faststart", self.path],
                        stdin=subprocess.PIPE)
                self.proc.stdin.write(np.ascontiguousarray(img).tobytes()); self.n += 1
        except Exception as e:  # surfaced on the next put()/close()
            self.err = e

    def close(self):
        self.q.put(None); self.t.join()
        if self.proc:
            self.proc.stdin.close()
            if self.proc.wait() != 0 and not self.err:
                self.err = RuntimeError(f"ffmpeg exited with {self.proc.returncode}")
        if self.err:
            raise self.err
        return self.n

    def abort(self):
        try:
            self.q.put_nowait(None)
        except queue.Full:
            pass
        if self.proc:
            self.proc.kill()
