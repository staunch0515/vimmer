"""Pure helpers for the Vimmer avatar runner (numpy only, no torch), so they can be unit-tested anywhere."""
import numpy as np

LIP_IDX = (6, 12, 14, 17, 19, 20)


def square_frame(box, img_w, img_h, scale=2.4, down=0.12):
    """Square head-and-shoulders frame around a face box (x0, y0, x1, y1), in pixels.

    The face (forehead to chin) takes about 1/scale of the frame height; the frame centre sits
    `down` x side below the face centre so the shoulders show. The square may extend past the
    photo; the caller pads. Returns (x0, y0, side) as ints.
    """
    x0, y0, x1, y1 = box
    fw, fh = x1 - x0, y1 - y0
    side = max(fw, fh) * scale
    # never larger than needed to show the whole photo's short side twice over
    side = min(side, 2.0 * max(img_w, img_h))
    cx = (x0 + x1) / 2.0
    cy = (y0 + y1) / 2.0 + down * side
    return int(round(cx - side / 2)), int(round(cy - side / 2)), max(int(round(side)), 1)


def visible_square(box, img_w, img_h, frame, min_share=0.6, down=0.05):
    """The part of a square_frame worth showing: (x0, y0, side) in photo pixels, inside `frame`.

    The model wants the face small in its input (square_frame), so a photo cut close to the head
    gets a made-up border. The clip should not show that border: this is the largest square
    inside both the frame and the photo, placed around the face (its centre `down` x side below
    the face centre, moved up to keep the hair in as long as the chin keeps some room). It never gets smaller than min_share x the frame, so the face still fits;
    a photo tighter than that keeps some (softened) border. When the photo covers the whole
    frame the result is the frame itself."""
    fx, fy, fside = frame
    x0, y0, x1, y1 = box
    side = int(min(fside, max(min(img_w, img_h), int(np.ceil(min_share * fside)))))

    def clamp(p, lo_photo, hi_photo, lo_frame):
        if hi_photo - lo_photo >= side:
            p = min(max(p, lo_photo), hi_photo - side)
        else:  # the photo is narrower than the square: share the border between both sides
            p = (lo_photo + hi_photo - side) / 2.0
        return int(round(min(max(p, lo_frame), lo_frame + fside - side)))

    vx = clamp((x0 + x1) / 2.0 - side / 2.0, 0, img_w, fx)
    fh = y1 - y0
    vy = (y0 + y1) / 2.0 + down * side - side / 2.0
    vy = min(vy, y0 - 0.45 * fh)            # keep the hair in (the face box starts at the forehead)
    vy = max(vy, y1 + 0.08 * side - side)   # but the chin, with room to open the mouth, comes first
    return vx, clamp(vy, 0, img_h, fy), side


def _box_blur(a, r):
    """Box blur of an HxWxC float array with radius r (edge-clamped), via cumulative sums."""
    if r < 1:
        return a
    for axis in (0, 1):
        n = a.shape[axis]
        pad = [(0, 0)] * a.ndim
        pad[axis] = (r + 1, r)
        c = np.cumsum(np.pad(a, pad, mode="edge"), axis=axis, dtype=np.float64)
        hi = np.take(c, np.arange(n) + 2 * r + 1, axis=axis)
        lo = np.take(c, np.arange(n), axis=axis)
        a = ((hi - lo) / (2 * r + 1)).astype(np.float32)
    return a


def crop_padded(img, x0, y0, side, soften=0.10, feather=0.02):
    """Crop a side x side square at (x0, y0) from an HxWxC uint8 image.

    Where the square reaches past the photo, the missing border continues the photo's edge
    pixels, and that made-up part is blurred: fully from `soften` x side outside the photo,
    blending in from `feather` x side inside it so no seam shows. Unblurred edge replication
    (the first version) drew hard streaks below a photo that ends at the shoulders; blurred, the
    shoulders and the background just carry on softly."""
    h, w = img.shape[:2]
    pad_l, pad_t = max(0, -x0), max(0, -y0)
    pad_r, pad_b = max(0, x0 + side - w), max(0, y0 + side - h)
    if not (pad_l or pad_t or pad_r or pad_b):
        return img[y0:y0 + side, x0:x0 + side]
    padded = np.pad(img, ((pad_t, pad_b), (pad_l, pad_r), (0, 0)), mode="edge")
    crop = padded[y0 + pad_t:y0 + pad_t + side, x0 + pad_l:x0 + pad_l + side].astype(np.float32)
    # distance (px) of every pixel from the photo's rectangle, shrunk by the feather on padded sides
    inset = feather * side
    ys = np.arange(side, dtype=np.float32)[:, None]
    xs = np.arange(side, dtype=np.float32)[None, :]
    dx = np.zeros_like(xs)
    dy = np.zeros_like(ys)
    if pad_l:
        dx = np.maximum(dx, (pad_l + inset) - xs)
    if pad_r:
        dx = np.maximum(dx, xs - (side - 1 - pad_r - inset))
    if pad_t:
        dy = np.maximum(dy, (pad_t + inset) - ys)
    if pad_b:
        dy = np.maximum(dy, ys - (side - 1 - pad_b - inset))
    t = np.clip(np.hypot(dx, dy) / max(soften * side, 1.0), 0.0, 1.0)
    t = (t * t * (3 - 2 * t))[:, :, None]  # smoothstep
    r = max(int(round(0.03 * side)), 1)
    blurred = _box_blur(_box_blur(_box_blur(crop, r), r), r)
    return np.clip(np.rint(crop + (blurred - crop) * t), 0, 255).astype(np.uint8)


def moving_mean(values, window):
    """Centred moving average along axis 0 with the window shrunk at the edges.
    window <= 0 or >= len means the plain clip mean for every frame."""
    v = np.asarray(values, dtype=np.float64)
    n = v.shape[0]
    if window <= 0 or window >= n:
        return np.broadcast_to(v.mean(axis=0), v.shape).copy()
    half = window // 2
    c = np.concatenate([np.zeros((1,) + v.shape[1:]), np.cumsum(v, axis=0)], axis=0)
    out = np.empty_like(v)
    for i in range(n):
        a, b = max(0, i - half), min(n, i + half + 1)
        out[i] = (c[b] - c[a]) / (b - a)
    return out


def orthonormalize(m):
    """Nearest rotation matrix (SVD) for each 3x3 matrix in m (..., 3, 3)."""
    u, _, vt = np.linalg.svd(m)
    r = u @ vt
    det = np.linalg.det(r)
    if np.any(det < 0):
        u = u.copy()
        u[det < 0, :, -1] *= -1
        r = u @ vt
    return r


def pose_refs(rots, ts, scales, mode="window", window=200):
    """Per-frame reference pose for the relative head motion.

    JoyVASA drives the head relative to frame 0 of the sampled motion. On long clips the sampled
    pose wanders, so the head drifts (the trial saw it look down after ~30 s). Here the reference is
      - "frame0": frame 0 (upstream behaviour),
      - "mean":   the clip mean (like the expression fix),
      - "window": a centred moving mean over `window` frames, which removes slow drift and keeps
                  the faster nods and turns.
    rots: (N, 3, 3); ts: (N, 3); scales: (N,). Returns (R_ref (N,3,3), t_ref (N,3), s_ref (N,)).
    """
    rots = np.asarray(rots, dtype=np.float64).reshape(-1, 3, 3)
    ts = np.asarray(ts, dtype=np.float64).reshape(len(rots), -1)
    scales = np.asarray(scales, dtype=np.float64).reshape(len(rots))
    n = len(rots)
    if mode == "frame0":
        return (np.repeat(rots[:1], n, 0), np.repeat(ts[:1], n, 0), np.repeat(scales[:1], n, 0))
    w = 0 if mode == "mean" else int(window)
    return (orthonormalize(moving_mean(rots, w)), moving_mean(ts, w), moving_mean(scales, w))


def pitch_deg(r):
    """Pitch (x-axis rotation) of rotation matrices, degrees; for drift diagnostics only."""
    r = np.asarray(r).reshape(-1, 3, 3)
    return np.degrees(np.arcsin(np.clip(-r[:, 1, 2], -1, 1)))


def pause_gate(samples, sr, fps, n_frames, min_pause=3, rel=0.02, ramp=5):
    """Per-frame weight in 0..1: 1 inside pauses of the speech, 0 while there is sound.

    The motion model leaves the mouth half open in pauses when they are digital silence (as in
    Kokoro's speech). The runner uses this weight to bring the lips back to the photo's own
    mouth there. A frame is quiet when its RMS is below `rel` x the clip's loud level (95th
    percentile); only runs of at least `min_pause` quiet frames count; the edges ramp over `ramp`
    frames, so the mouth closes just after the sound ends and starts to open just before it
    resumes."""
    x = np.asarray(samples, dtype=np.float64).reshape(-1)
    hop = sr / float(fps)
    rms = np.zeros(n_frames)
    for i in range(n_frames):
        seg = x[int(round(i * hop)):int(round((i + 1) * hop))]
        rms[i] = np.sqrt(np.mean(seg * seg)) if len(seg) else 0.0
    loud = np.percentile(rms, 95) if n_frames else 0.0
    quiet = rms <= rel * loud if loud > 0 else np.ones(n_frames, dtype=bool)
    gate = np.zeros(n_frames)
    i = 0
    while i < n_frames:
        if quiet[i]:
            j = i
            while j < n_frames and quiet[j]:
                j += 1
            if j - i >= min_pause:
                gate[i:j] = 1.0
            i = j
        else:
            i += 1
    return moving_mean(gate, ramp) if 1 < ramp < n_frames else gate
