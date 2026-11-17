#!/bin/sh
# Installs the optional narrator-avatar engine (「数字人讲解人」) for Vimmer: JoyVASA + LivePortrait
# with the Vimmer patches, a Python 3.10 venv (torch 2.14.1, coremltools 9.0, MediaPipe), ~1.4 GB of
# MIT/Apache weights and a CoreML export of the SPADE generator. Apple silicon only. Everything stays
# local; nothing here is a paid or cloud service.
#
#   sh rust-core/crates/vimmer-media/scripts/avatar/install_avatar.sh            # install
#   sh rust-core/crates/vimmer-media/scripts/avatar/install_avatar.sh --dry-run  # only say what it would do
#
# It first REUSES an existing trial setup by copying (no network): VIMMER_AVATAR_FROM, default
# ~/vimmer-avatar-trial (venv .venv-torchnew + its uv Python, JoyVASA git clone, pretrained_weights,
# mediapipe_models/face_landmarker.task, speed/coreml/spade_fp16.mlpackage). Anything it cannot copy
# it would download; it lists exactly what and stops unless VIMMER_AVATAR_ALLOW_DOWNLOAD=1.
#
# Target: VIMMER_AVATAR_HOME, default ~/.local/share/vimmer/avatar.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
dest=${VIMMER_AVATAR_HOME:-$HOME/.local/share/vimmer/avatar}
from=${VIMMER_AVATAR_FROM:-$HOME/vimmer-avatar-trial}
allow=${VIMMER_AVATAR_ALLOW_DOWNLOAD:-0}
dry=0
[ "${1:-}" = "--dry-run" ] && dry=1
export PATH=/opt/homebrew/bin:/usr/local/bin:$PATH

JOYVASA_COMMIT=916a90f8de490e8648fee460c1200bd5d9a795af
MP_SHA=64184e229b263107bc2b804c6625db1341ff2bb731874b0bcc2fe6544e0bc9ff
ENGINE=vimmer-avatar-1

if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
    echo "The avatar engine needs an Apple-silicon Mac (M1 or newer). This machine is $(uname -s) $(uname -m)."
    echo "Install it on the Mac that renders videos."
    exit 1
fi
command -v ffmpeg >/dev/null || { echo "ffmpeg is missing (brew install ffmpeg)"; exit 1; }

echo "install to: $dest"
echo "reuse from: $from"

# ---- plan: what can be copied, what would be downloaded ----
downloads=""
want() { downloads="$downloads
  - $1"; }
trial_py=""
if [ -x "$from/.venv-torchnew/bin/python" ]; then
    trial_py=$(sed -n 's/^home = //p' "$from/.venv-torchnew/pyvenv.cfg")
    trial_py=${trial_py%/bin}
    [ -x "$trial_py/bin/python3.10" ] || trial_py=""
fi
[ -n "$trial_py" ] && echo "  copy venv  $from/.venv-torchnew (+ Python $trial_py)" || {
    want "Python 3.10 via uv (python-build-standalone, ~20 MB)"
    want "PyPI wheels: torch==2.14.1 torchvision==0.29.2 torchaudio==2.11.1 coremltools==9.0 + requirements.txt (~1 GB)"
    want "PyPI: mediapipe==1.2.3 absl-py==2.5.1 (--no-deps, ~110 MB)"; }
[ -d "$from/JoyVASA/.git" ] && echo "  clone code $from/JoyVASA (local git clone, commit ${JOYVASA_COMMIT%????????????????????????????????})" || \
    want "git clone https://github.com/jdh-algo/JoyVASA (MIT, ~30 MB)"
wts="$from/JoyVASA/pretrained_weights"
if [ -f "$wts/JoyVASA/motion_generator/motion_generator_hubert_chinese.pt" ] && [ -f "$wts/TencentGameMate:chinese-hubert-base/pytorch_model.bin" ] \
   && [ -f "$wts/liveportrait/base_models/spade_generator.pth" ]; then
    echo "  copy weights $wts (without insightface/, ~1.4 GB)"
else
    want "Hugging Face jdh-algo/JoyVASA: motion_generator/motion_generator_hubert_chinese.pt, motion_template/motion_template.pkl (MIT, ~415 MB)"
    want "Hugging Face TencentGameMate/chinese-hubert-base: config.json, preprocessor_config.json, pytorch_model.bin (~380 MB; license to verify)"
    want "Hugging Face KlingTeam/LivePortrait: liveportrait/base_models/{appearance_feature_extractor,motion_extractor,spade_generator,warping_module}.pth, liveportrait/landmark.onnx, liveportrait/retargeting_models/stitching_retargeting_module.pth (MIT, ~600 MB)"
fi
[ -f "$from/mediapipe_models/face_landmarker.task" ] && echo "  copy $from/mediapipe_models/face_landmarker.task" || \
    want "https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1/face_landmarker.task (Apache-2.0, 3.6 MB)"
[ -d "$from/speed/coreml/spade_fp16.mlpackage" ] && echo "  copy $from/speed/coreml/spade_fp16.mlpackage" || \
    echo "  export spade_fp16.mlpackage locally from the weights (no download, ~10 s)"

if [ -n "$downloads" ]; then
    echo "would download:$downloads"
    if [ "$allow" != 1 ]; then
        echo "Not downloading anything. Re-run with VIMMER_AVATAR_ALLOW_DOWNLOAD=1 to allow these downloads."
        exit 2
    fi
fi
[ "$dry" = 1 ] && { echo "dry run: nothing changed"; exit 0; }

mkdir -p "$dest"
tmp="$dest/.installing"
rm -rf "$tmp"; mkdir -p "$tmp"

# ---- 1. Python + venv ----
if [ -n "$trial_py" ]; then
    cp -R "$trial_py" "$tmp/python"
    cp -R "$from/.venv-torchnew" "$tmp/venv"
    # re-point the copied venv at the copied interpreter (the venv's own scripts are not used)
    sed -i '' "s#^home = .*#home = $dest/python/bin#" "$tmp/venv/pyvenv.cfg"
    ln -sfn "$dest/python/bin/python3.10" "$tmp/venv/bin/python"
else
    command -v uv >/dev/null || { echo "uv is missing (brew install uv)"; exit 1; }
    UV_PYTHON_INSTALL_DIR="$tmp/uv-python" uv venv --python 3.10 "$tmp/venv"
    py=$(sed -n 's/^home = //p' "$tmp/venv/pyvenv.cfg"); py=${py%/bin}
    mv "$py" "$tmp/python"
    sed -i '' "s#^home = .*#home = $dest/python/bin#" "$tmp/venv/pyvenv.cfg"
    ln -sfn "$dest/python/bin/python3.10" "$tmp/venv/bin/python"
    uv pip install --python "$tmp/venv/bin/python" torch==2.14.1 torchvision==0.29.2 torchaudio==2.11.1 coremltools==9.0 -r "$here/requirements.txt"
    uv pip install --python "$tmp/venv/bin/python" --no-deps mediapipe==1.2.3 absl-py==2.5.1
fi

# ---- 2. JoyVASA code at the tested commit + the Vimmer patch (device fixes, MediaPipe detector) ----
if [ -d "$from/JoyVASA/.git" ]; then
    git clone -q --no-hardlinks "$from/JoyVASA" "$tmp/JoyVASA"
else
    git clone -q https://github.com/jdh-algo/JoyVASA "$tmp/JoyVASA"
fi
git -C "$tmp/JoyVASA" checkout -q "$JOYVASA_COMMIT"
git -C "$tmp/JoyVASA" apply "$here/joyvasa.patch"
cp "$here/face_mediapipe.py" "$tmp/JoyVASA/src/utils/face_mediapipe.py"
rm -rf "$tmp/JoyVASA/.git" "$tmp/JoyVASA/assets/examples"

# ---- 3. Weights (no InsightFace: non-commercial and unused with the MediaPipe detector) ----
mkdir -p "$tmp/JoyVASA/pretrained_weights"
if [ -f "$wts/liveportrait/base_models/spade_generator.pth" ]; then
    rsync -a --exclude insightface "$wts/" "$tmp/JoyVASA/pretrained_weights/"
else
    "$tmp/venv/bin/python" - "$tmp/JoyVASA/pretrained_weights" <<'PY'
import os, sys
from huggingface_hub import hf_hub_download
W = sys.argv[1]
jobs = [("jdh-algo/JoyVASA", ["motion_generator/motion_generator_hubert_chinese.pt", "motion_template/motion_template.pkl"], "JoyVASA"),
        ("TencentGameMate/chinese-hubert-base", ["config.json", "preprocessor_config.json", "pytorch_model.bin"], "TencentGameMate:chinese-hubert-base"),
        ("KlingTeam/LivePortrait", ["liveportrait/base_models/appearance_feature_extractor.pth", "liveportrait/base_models/motion_extractor.pth",
          "liveportrait/base_models/spade_generator.pth", "liveportrait/base_models/warping_module.pth",
          "liveportrait/landmark.onnx", "liveportrait/retargeting_models/stitching_retargeting_module.pth"], "")]
for repo, files, sub in jobs:
    for f in files:
        print("ok", hf_hub_download(repo, f, local_dir=os.path.join(W, sub)), flush=True)
PY
fi

# ---- 4. MediaPipe FaceLandmarker model ----
mkdir -p "$tmp/mediapipe"
if [ -f "$from/mediapipe_models/face_landmarker.task" ]; then
    cp "$from/mediapipe_models/face_landmarker.task" "$tmp/mediapipe/"
else
    curl -fsSL -o "$tmp/mediapipe/face_landmarker.task" https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1/face_landmarker.task
fi
echo "$MP_SHA  $tmp/mediapipe/face_landmarker.task" | shasum -a 256 -c -

# ---- 5. SPADE generator on the Neural Engine ----
if [ -d "$from/speed/coreml/spade_fp16.mlpackage" ]; then
    cp -R "$from/speed/coreml/spade_fp16.mlpackage" "$tmp/"
else
    TORCH_FORCE_NO_WEIGHTS_ONLY_LOAD=1 "$tmp/venv/bin/python" "$here/export_spade_coreml.py" "$tmp/JoyVASA" "$tmp/spade_fp16.mlpackage"
fi

# ---- 6. Licenses + version stamp, then swap into place ----
mkdir -p "$tmp/licenses"
cp "$here/LICENSES.md" "$tmp/licenses/" 2>/dev/null || true
cp "$tmp/JoyVASA/LICENSE" "$tmp/licenses/JoyVASA-LICENSE" 2>/dev/null || true
echo "$ENGINE" > "$tmp/VERSION"
for d in python venv JoyVASA mediapipe spade_fp16.mlpackage licenses VERSION; do
    rm -rf "${dest:?}/$d"; mv "$tmp/$d" "$dest/$d"
done
rm -rf "$tmp"

VIMMER_AVATAR_HOME="$dest" "$dest/venv/bin/python" "$here/vimmer_avatar.py" selftest
echo "done ($(du -sh "$dest" | cut -f1)). Check with: vimmer avatar status"
