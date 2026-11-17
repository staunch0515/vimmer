# Narrator avatar engine: sources and licenses

Nothing large is vendored into git. `install_avatar.sh` copies (or, only when allowed, downloads)
the upstream code and weights into `~/.local/share/vimmer/avatar/`.

## In this folder

| File | Origin | License |
|---|---|---|
| `vimmer_avatar.py`, `avatar_math.py`, `test_avatar_math.py`, `install_avatar.sh` | Vimmer. The animate loop in `vimmer_avatar.py` follows `LivePortraitPipeline.execute` from JoyVASA (whole-image path only). | Vimmer + MIT (JoyVASA, see `JoyVASA-LICENSE`) |
| `fastwarp.py`, `speedpatch.py`, `export_spade_coreml.py` | Vimmer avatar speed trial (exact MPS rewrite of LivePortrait's 3D ops, CoreML SPADE export). Operate on LivePortrait modules. | Vimmer |
| `face_mediapipe.py` | Vimmer trial: MediaPipe FaceLandmarker in place of InsightFace. | Vimmer; uses MediaPipe (Apache-2.0) |
| `joyvasa.patch` | Vimmer changes to JoyVASA files (device fixes, 478-point MediaPipe landmarks, detector switch). Contains JoyVASA context lines. | MIT (JoyVASA) |
| `JoyVASA-LICENSE` | Upstream license file of JoyVASA, kept verbatim. | MIT, (c) 2024 xuyangcao |

## Installed (not in git)

| Component | Source | License | Note |
|---|---|---|---|
| JoyVASA code, commit `916a90f` | github.com/jdh-algo/JoyVASA | MIT | Includes LivePortrait code (MIT, KlingTeam/Kuaishou). `src/utils/dependencies/{insightface,XPose}` code is in the tree but not used (human mode + MediaPipe). |
| JoyVASA motion generator + template | huggingface.co/jdh-algo/JoyVASA | MIT | |
| LivePortrait weights incl. `landmark.onnx` | huggingface.co/KlingTeam/LivePortrait | MIT | |
| `spade_fp16.mlpackage` | generated locally from the LivePortrait SPADE weights | derived from MIT weights | |
| MediaPipe + `face_landmarker.task` | PyPI `mediapipe==1.2.3`, storage.googleapis.com/mediapipe-models | Apache-2.0 | |
| `TencentGameMate/chinese-hubert-base` | huggingface.co/TencentGameMate/chinese-hubert-base | **TO VERIFY** | The model card's license was not verified. Check before commercial use. |
| PyTorch 2.14.1, torchvision, torchaudio, coremltools 9.0 | PyPI | BSD-3-Clause | |
| Other Python deps (`requirements.txt`) | PyPI | permissive (BSD/MIT/Apache) | `onnxruntime-silicon` MIT. |
| InsightFace `buffalo_l` weights | — | non-commercial | **Not installed** (excluded on copy, never downloaded). |

Credit: JoyVASA (Cao et al., JD Health), LivePortrait (Guo et al., Kuaishou/KlingTeam), MediaPipe (Google).
