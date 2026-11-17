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


def crop_padded(img, x0, y0, side):
    """Crop a side x side square at (x0, y0) from an HxWxC image, padding by edge replication."""
    h, w = img.shape[:2]
    pad_l, pad_t = max(0, -x0), max(0, -y0)
    pad_r, pad_b = max(0, x0 + side - w), max(0, y0 + side - h)
    if pad_l or pad_t or pad_r or pad_b:
        img = np.pad(img, ((pad_t, pad_b), (pad_l, pad_r), (0, 0)), mode="edge")
        x0 += pad_l
        y0 += pad_t
    return img[y0:y0 + side, x0:x0 + side]


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
