#!/bin/sh
# Installs the optional Kokoro narration engine for Vimmer: a Python venv with kokoro-onnx and
# misaki, and the model files. Everything stays local; nothing here is a paid or cloud service.
#
#   sh rust-core/crates/vimmer-media/scripts/install_kokoro.sh
#
# Override the locations with VIMMER_KOKORO_VENV and VIMMER_KOKORO_MODELS, and the interpreter
# that creates the venv with PYTHON (default: python3.12, else python3).
set -eu

here=$(cd "$(dirname "$0")" && pwd)
venv=${VIMMER_KOKORO_VENV:-$HOME/.local/share/vimmer/kokoro-venv}
models=${VIMMER_KOKORO_MODELS:-$HOME/.local/share/vimmer/models/kokoro}
python=${PYTHON:-$(command -v python3.12 || command -v python3)}

echo "venv:   $venv (from $python)"
echo "models: $models"

if [ ! -x "$venv/bin/python" ]; then
    "$python" -m venv "$venv"
fi
"$venv/bin/python" -m pip install --upgrade pip
"$venv/bin/python" -m pip install -r "$here/requirements.txt"

mkdir -p "$models"
fetch() { # url file
    if [ -s "$models/$2" ]; then
        echo "have $2"
    else
        echo "downloading $2"
        curl -fL --retry 3 -o "$models/$2.part" "$1" && mv "$models/$2.part" "$models/$2"
    fi
}
gh=https://github.com/thewh1teagle/kokoro-onnx/releases/download
fetch "$gh/model-files-v1.0/kokoro-v1.0.onnx" kokoro-v1.0.onnx
fetch "$gh/model-files-v1.0/voices-v1.0.bin" voices-v1.0.bin
fetch "$gh/model-files-v1.1/kokoro-v1.1-zh.onnx" kokoro-v1.1-zh.onnx
fetch "$gh/model-files-v1.1/voices-v1.1-zh.bin" voices-v1.1-zh.bin
fetch "https://huggingface.co/hexgrad/Kokoro-82M-v1.1-zh/resolve/main/config.json" config-v1.1-zh.json

echo "done. Render with: vimmer <timeline.json> -o out.mp4 --tts kokoro"
