#!/bin/sh
# Installs the Python environment for `vimmer generate`: a venv with the official anthropic SDK.
# Nothing else is downloaded. The API key is NOT handled here: export ANTHROPIC_API_KEY in
# ~/.zshrc yourself (see README).
#
#   sh rust-core/crates/vimmer-storyboard/scripts/install_llm.sh
#
# Override the venv location with VIMMER_LLM_VENV. Uses uv when it is on PATH, else
# python -m venv + pip (interpreter: $PYTHON, default python3.12, else python3).
set -eu

here=$(cd "$(dirname "$0")" && pwd)
venv=${VIMMER_LLM_VENV:-$HOME/.local/share/vimmer/llm-venv}

echo "venv: $venv"
if command -v uv >/dev/null 2>&1; then
    if [ ! -x "$venv/bin/python" ]; then
        uv venv --python 3.12 "$venv" || uv venv "$venv"
    fi
    uv pip install --python "$venv/bin/python" -r "$here/requirements.txt"
else
    python=${PYTHON:-$(command -v python3.12 || command -v python3)}
    if [ ! -x "$venv/bin/python" ]; then
        "$python" -m venv "$venv"
    fi
    "$venv/bin/python" -m pip install --upgrade pip
    "$venv/bin/python" -m pip install -r "$here/requirements.txt"
fi
"$venv/bin/python" -c "import anthropic; print('anthropic', anthropic.__version__)"
echo "done. Generate with: vimmer generate story.txt --lang zh -o out.mp4"
