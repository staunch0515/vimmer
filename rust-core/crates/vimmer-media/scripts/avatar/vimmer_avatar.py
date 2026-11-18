"""Vimmer narrator avatar runner: one face photo + one speech track -> a muted talking-head clip.

JoyVASA (audio -> facial motion) + LivePortrait (photo + motion -> frames), with the Vimmer trial's
changes: MediaPipe face detector (instead of non-commercial InsightFace), clip-mean expression
reference with calmer eyes and brows (--expression), lips closed in the pauses of the speech, a
moving-mean head-pose reference (drift fix), exact MPS speed rewrite, SPADE on the Neural Engine
(CoreML), and frames streamed to ffmpeg. The clip shows only the part of the frame the photo
really covers.

Usage (run with the avatar venv's python; VIMMER_AVATAR_HOME = the install dir):
  vimmer_avatar.py generate --photo P --audio A.wav --out OUT.mp4 [--size 512] [--fps 25] [--seed 42]
  vimmer_avatar.py probe --photo P        # face check only (MediaPipe, ~1 s)
  vimmer_avatar.py selftest               # versions + MPS availability

stdout is JSON lines: {"event": "stage"|"progress"|"face"|"done"|"error"|"info", ...}.
Exit codes: 0 ok, 2 no face in the photo, 3 bad input, 1 anything else.
"""
import argparse, json, os, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import avatar_math as am  # noqa: E402

HOME = os.environ.get("VIMMER_AVATAR_HOME") or os.path.expanduser("~/.local/share/vimmer/avatar")
JOYVASA = os.path.join(HOME, "JoyVASA")
MP_MODEL = os.path.join(HOME, "mediapipe", "face_landmarker.task")
SPADE_ML = os.path.join(HOME, "spade_fp16.mlpackage")


def emit(event, **kw):
    kw["event"] = event
    sys.stdout.write(json.dumps(kw, ensure_ascii=False) + "\n")
    sys.stdout.flush()


class Fail(Exception):
    def __init__(self, code, message, exit_code=1):
        super().__init__(message)
        self.code, self.exit_code = code, exit_code


def load_rgb(path):
    import cv2
    img = cv2.imread(path, cv2.IMREAD_COLOR)
    if img is None:
        raise Fail("bad_photo", f"cannot read image: {path}", 3)
    return cv2.cvtColor(img, cv2.COLOR_BGR2RGB)


def face_box(img_rgb):
    sys.path.insert(0, HERE)
    from face_mediapipe import MediaPipeFace
    mesh = MediaPipeFace(MP_MODEL).mesh478(img_rgb)
    if mesh is None:
        return None
    return [float(mesh[:, 0].min()), float(mesh[:, 1].min()), float(mesh[:, 0].max()), float(mesh[:, 1].max())]


def cmd_probe(a):
    img = load_rgb(a.photo)
    box = face_box(img)
    h, w = img.shape[:2]
    emit("face", found=box is not None, box=box, width=w, height=h)
    return 0 if box is not None else 2


def cmd_selftest(_a):
    import torch, numpy, cv2  # noqa: F401
    info = {"torch": torch.__version__, "mps": bool(torch.backends.mps.is_available()),
            "joyvasa": os.path.isdir(os.path.join(JOYVASA, "src")),
            "mediapipe_model": os.path.isfile(MP_MODEL), "spade_coreml": os.path.isdir(SPADE_ML)}
    try:
        import coremltools  # noqa: F401
        info["coremltools"] = coremltools.__version__
    except Exception:
        info["coremltools"] = None
    emit("info", **info)
    return 0 if info["mps"] and info["joyvasa"] and info["mediapipe_model"] else 1


def cmd_generate(a):
    t_start = time.time()
    emit("stage", stage="load")
    os.environ.setdefault("TORCH_FORCE_NO_WEIGHTS_ONLY_LOAD", "1")  # JoyVASA ckpt holds an argparse.Namespace
    os.environ.setdefault("PYTORCH_ENABLE_MPS_FALLBACK", "1")
    os.environ.setdefault("HF_HUB_OFFLINE", "1")
    os.environ["JOYVASA_DETECTOR"] = "mediapipe"
    os.environ["JOYVASA_MP_MODEL"] = MP_MODEL
    import numpy as np
    import cv2
    import torch
    if not torch.backends.mps.is_available():
        raise Fail("no_mps", "Apple-silicon GPU (MPS) not available")

    # 1. Photo -> square head-and-shoulders frame at --size (whole-image mode: no paste-back seams)
    img = load_rgb(a.photo)
    box = face_box(img)
    if box is None:
        raise Fail("no_face", "no face detected in the photo", 2)
    h, w = img.shape[:2]
    x0, y0, side = am.square_frame(box, w, h, scale=a.frame_scale)
    framed = cv2.resize(am.crop_padded(img, x0, y0, side), (a.size, a.size), interpolation=cv2.INTER_AREA)
    # the clip shows only the part of the frame the photo really covers (no made-up border)
    vx, vy, vside = am.visible_square(box, w, h, (x0, y0, side))
    k = a.size / float(side)
    crop = None
    if vside < side:
        cs = max(2, int(round(vside * k)) // 2 * 2)
        crop = (min(max(int(round((vx - x0) * k)), 0), a.size - cs),
                min(max(int(round((vy - y0) * k)), 0), a.size - cs), cs)
    emit("info", frame=[x0, y0, side], visible=[vx, vy, vside], crop=crop)
    work = os.path.dirname(os.path.abspath(a.out))
    src_png = os.path.join(work, os.path.basename(a.out) + ".src.png")
    cv2.imwrite(src_png, cv2.cvtColor(framed, cv2.COLOR_RGB2BGR))

    # 2. Models (JoyVASA code at a pinned commit + the small vimmer patch; see install_avatar.sh)
    sys.path.insert(0, JOYVASA)
    os.chdir(JOYVASA)  # JoyVASA resolves pretrained_weights/ relative to the cwd
    from src.config.argument_config import ArgumentConfig
    from src.config.inference_config import InferenceConfig
    from src.config.crop_config import CropConfig
    from src.live_portrait_wmg_pipeline import LivePortraitPipeline
    from src.utils.camera import get_rotation_matrix
    from src.utils.helper import dct2device
    import speedpatch

    args = ArgumentConfig(reference=src_png, audio=os.path.abspath(a.audio), output_dir=work,
                          animation_mode="human", cfg_scale=a.cfg_scale)
    pick = lambda cls: cls(**{k: v for k, v in args.__dict__.items() if hasattr(cls, k)})
    pipe = LivePortraitPipeline(inference_cfg=pick(InferenceConfig), crop_cfg=pick(CropConfig))
    lp = pipe.live_portrait_wrapper
    inf = lp.inference_cfg
    dev = lp.device
    speedpatch.apply(lp)
    spade = speedpatch.CoreMLSpade(SPADE_ML) if os.path.isdir(SPADE_ML) else None

    # 3. Motion for the whole clip first (~17 s per minute of audio)
    emit("stage", stage="motion")
    torch.manual_seed(a.seed)
    np.random.seed(a.seed)
    motion = lp.gen_motion_sequence(args)
    n = int(motion["n_frames"])
    frames = motion["motion"][:n]
    rots = np.stack([np.asarray(m["R"] if "R" in m else m["R_d"]).reshape(3, 3) for m in frames])
    ts = np.stack([np.asarray(m["t"]).reshape(-1) for m in frames])
    ss = np.stack([np.asarray(m["scale"]).reshape(-1)[0] for m in frames])
    r_ref, t_ref, s_ref = am.pose_refs(rots, ts, ss, a.pose_ref, int(a.pose_window * a.fps))
    exp_ref = torch.from_numpy(np.stack([np.asarray(m["exp"]) for m in frames]).mean(axis=0)).float().to(dev)
    # lips go back to the photo's own mouth in the pauses of the speech (see avatar_math.pause_gate)
    import librosa
    wav, _sr = librosa.load(os.path.abspath(a.audio), sr=16000, mono=True)
    gate = am.pause_gate(wav, 16000, a.fps, n) if a.close_in_pauses else np.zeros(n)
    emit("info", pause_frames=int((gate > 0.5).sum()))
    p0, p1 = am.pitch_deg(rots), am.pitch_deg(np.einsum("nij,nkj->nik", rots, r_ref))
    emit("info", frames=n, pitch_range_raw=[round(float(p0.min()), 1), round(float(p0.max()), 1)],
         pitch_range_used=[round(float(p1.min()), 1), round(float(p1.max()), 1)])

    # 4. Source keypoints and features
    I_s = lp.prepare_source(cv2.resize(framed, (256, 256)))
    x_s_info = lp.get_kp_info(I_s)
    x_c_s = x_s_info["kp"]
    R_s = get_rotation_matrix(x_s_info["pitch"], x_s_info["yaw"], x_s_info["roll"])
    f_s = lp.extract_feature_3d(I_s)
    x_s = lp.transform_keypoint(x_s_info)

    # 5. Animate (whole-image path of LivePortraitPipeline.execute with the default JoyVASA flags:
    #    no stitching, no relative-motion multiplier, driving_multiplier 1.1) and stream to ffmpeg.
    emit("stage", stage="render", total=n)
    sink = speedpatch.FrameSink(a.out, fps=a.fps, transform=spade, crop=crop, size=a.size)
    try:
        for i in range(n):
            d = dct2device(dict(frames[i]), dev)
            R_d = d["R"] if "R" in d else d["R_d"]
            R_ref = torch.from_numpy(r_ref[i]).float().to(dev).reshape(1, 3, 3)
            R_new = (R_d @ R_ref.permute(0, 2, 1)) @ R_s
            delta = x_s_info["exp"] + (d["exp"] - exp_ref) * a.expression
            g = float(gate[i])
            for j in am.LIP_IDX:
                delta[:, j, :] = d["exp"][:, j, :] * (1.0 - g) + x_s_info["exp"][:, j, :] * g
            scale_new = x_s_info["scale"] * (d["scale"] / float(s_ref[i]))
            t_new = x_s_info["t"] + (d["t"] - torch.from_numpy(t_ref[i]).float().to(dev).reshape(d["t"].shape))
            t_new[..., 2].fill_(0)
            x_d = scale_new * (x_c_s @ R_new + delta) + t_new
            x_d = x_s + (x_d - x_s) * a.driving_multiplier
            with torch.no_grad():
                if spade is not None:
                    sink.put(lp.warping_module(f_s, kp_source=x_s, kp_driving=x_d)["out"].float().cpu().numpy())
                else:
                    sink.put(lp.parse_output(lp.warp_decode(f_s, x_s, x_d)["out"])[0])
            if i % 5 == 4 or i == n - 1:
                emit("progress", done=i + 1, total=n)
        written = sink.close()
    except BaseException:
        sink.abort()
        raise
    finally:
        try:
            os.remove(src_png)
        except OSError:
            pass
    emit("done", frames=written, out=a.out, seconds=round(time.time() - t_start, 1))
    return 0


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    g = sub.add_parser("generate")
    g.add_argument("--photo", required=True)
    g.add_argument("--audio", required=True)
    g.add_argument("--out", required=True)
    g.add_argument("--size", type=int, default=512)
    g.add_argument("--fps", type=int, default=25)
    g.add_argument("--seed", type=int, default=42)
    g.add_argument("--cfg-scale", type=float, default=2.0)
    g.add_argument("--frame-scale", type=float, default=2.4)
    g.add_argument("--no-close-in-pauses", dest="close_in_pauses", action="store_false",
                   help="keep the model's own lips in the pauses of the speech")
    g.add_argument("--expression", type=float, default=0.6,
                   help="strength of the eye/brow/cheek motion around the photo's own expression (lips excluded)")
    g.add_argument("--driving-multiplier", type=float, default=1.0)
    g.add_argument("--pose-ref", choices=["frame0", "mean", "window"], default="window")
    g.add_argument("--pose-window", type=float, default=8.0, help="seconds, for --pose-ref window")
    pr = sub.add_parser("probe")
    pr.add_argument("--photo", required=True)
    sub.add_parser("selftest")
    a = p.parse_args()
    # The model code needs the JoyVASA dir as cwd, so make the user's paths absolute first.
    for k in ("photo", "audio", "out"):
        if getattr(a, k, None):
            setattr(a, k, os.path.abspath(getattr(a, k)))
    try:
        return {"generate": cmd_generate, "probe": cmd_probe, "selftest": cmd_selftest}[a.cmd](a)
    except Fail as e:
        emit("error", code=e.code, message=str(e))
        return e.exit_code
    except KeyboardInterrupt:
        emit("error", code="cancelled", message="cancelled")
        return 130
    except Exception as e:  # noqa: BLE001
        emit("error", code="runner", message=f"{type(e).__name__}: {e}")
        return 1


if __name__ == "__main__":
    sys.exit(main())
