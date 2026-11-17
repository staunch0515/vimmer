"""export_spade_coreml.py <JoyVASA dir> <out .mlpackage>: LivePortrait SPADE generator -> CoreML mlprogram (fp16).
Spectral norm is baked into the weights first. Runs in ~5 s; no downloads."""
import os, sys, yaml, torch
J, OUT = sys.argv[1], sys.argv[2]; sys.path.insert(0, J)
from src.utils.helper import load_model
import coremltools as ct
W = f"{J}/pretrained_weights/liveportrait/base_models"
cfg = yaml.safe_load(open(f"{J}/src/config/models.yaml"))
g = load_model(f"{W}/spade_generator.pth", cfg, "cpu", "spade_generator")
g.load_state_dict(torch.load(f"{W}/spade_generator.pth", map_location="cpu", weights_only=False)); g.eval()
for m in g.modules():
    if hasattr(m, "weight_orig"): torch.nn.utils.remove_spectral_norm(m)
class G(torch.nn.Module):
    def __init__(s, g): super().__init__(); s.g = g
    def forward(s, x): return s.g(feature=x)
x = torch.randn(1, 256, 64, 64) * 0.3
with torch.no_grad(): tr = torch.jit.trace(G(g).eval(), x)
ml = ct.convert(tr, inputs=[ct.TensorType(name="x", shape=x.shape)], outputs=[ct.TensorType(name="img")],
                convert_to="mlprogram", compute_precision=ct.precision.FLOAT16, minimum_deployment_target=ct.target.macOS13)
ml.save(OUT); print("saved", OUT)
