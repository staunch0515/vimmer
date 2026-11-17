"""Exact MPS-friendly rewrite of LivePortrait's 3D ops (no new packages).

- nn.Conv3d (stride 1, groups 1) -> sum over depth taps of Conv2d on a (B*D, C, H, W) layout.
- AvgPool3d((1,2,2)) and nearest Upsample (1,2,2) -> 2D ops on (B, C*D, H, W) (avoids MPS CPU fallbacks).
Same weights, same math; only the kernels differ.
"""
import torch, torch.nn as nn, torch.nn.functional as F

INVERSE_ON_DEVICE = tuple(int(v) for v in torch.__version__.split("+")[0].split(".")[:2]) >= (2, 14)


class Conv3dAs2d(nn.Module):
    def __init__(self, conv: nn.Conv3d, mode="loop"):
        super().__init__()
        assert conv.stride == (1, 1, 1) and conv.groups == 1 and conv.dilation == (1, 1, 1)
        self.kd, kh, kw = conv.kernel_size
        self.pd, self.ph, self.pw = conv.padding
        w = conv.weight.detach()  # O, C, kd, kh, kw
        self.O, self.C = w.shape[:2]
        self.mode = mode
        if mode == "loop":
            self.w = nn.Parameter(w.permute(2, 0, 1, 3, 4).contiguous(), requires_grad=False)  # kd, O, C, kh, kw
        else:  # stacked single conv2d: channels ordered (C, kd)
            self.w = nn.Parameter(w.reshape(self.O, self.C * self.kd, kh, kw).contiguous(), requires_grad=False)
        self.b = None if conv.bias is None else nn.Parameter(conv.bias.detach().clone(), requires_grad=False)

    half = False  # set True to run the conv in fp16 (input/output stay fp32)

    def forward(self, x):
        if self.half:
            if self.w.dtype != torch.float16:
                self.w.data = self.w.data.half()
                if self.b is not None: self.b.data = self.b.data.half()
            return self._fwd(x.half()).float()
        return self._fwd(x)

    def _fwd(self, x):
        B, C, D, H, W = x.shape
        xt = x.permute(0, 2, 1, 3, 4)  # B, D, C, H, W
        if self.pd:
            xt = F.pad(xt, (0, 0, 0, 0, 0, 0, self.pd, self.pd))
        xt = xt.contiguous()
        if self.mode == "loop":
            out = None
            for j in range(self.kd):
                xj = xt[:, j:j + D].reshape(B * D, C, H, W)
                y = F.conv2d(xj, self.w[j], None, 1, (self.ph, self.pw))
                out = y if out is None else out + y
            if self.b is not None:
                out = out + self.b.view(1, -1, 1, 1)
        else:
            cols = torch.stack([xt[:, j:j + D] for j in range(self.kd)], dim=3)  # B, D, C, kd, H, W
            out = F.conv2d(cols.reshape(B * D, C * self.kd, H, W), self.w, self.b, 1, (self.ph, self.pw))
        return out.view(B, D, self.O, H, W).permute(0, 2, 1, 3, 4).contiguous()


class Pool122(nn.Module):
    def forward(self, x):
        B, C, D, H, W = x.shape
        return F.avg_pool2d(x.reshape(B, C * D, H, W), 2).view(B, C, D, H // 2, W // 2)


def _up_forward(self, x):  # UpBlock3d.forward with a 2D nearest upsample
    B, C, D, H, W = x.shape
    out = F.interpolate(x.reshape(B, C * D, H, W), scale_factor=2, mode="nearest").view(B, C, D, H * 2, W * 2)
    out = self.conv(out)
    out = self.norm(out)
    return F.relu(out)


def patch(module: nn.Module, mode="loop", min_kernel=1):
    """Replace 3D ops in-place inside `module` (e.g. the warping network)."""
    import types
    for name, child in list(module.named_children()):
        if isinstance(child, nn.Conv3d) and child.kernel_size[0] >= min_kernel and child.kernel_size != (1, 1, 1):
            setattr(module, name, Conv3dAs2d(child, mode).to(child.weight.device))
        elif isinstance(child, nn.AvgPool3d) and tuple(child.kernel_size) == (1, 2, 2):
            setattr(module, name, Pool122())
        else:
            if type(child).__name__ == "UpBlock3d":
                child.forward = types.MethodType(_up_forward, child)
            patch(child, mode, min_kernel)
    return module


def set_half(module, on=True):
    for m in module.modules():
        if isinstance(m, Conv3dAs2d):
            m.half = on
    return module


class HalfWrap(nn.Module):
    """Run a 2D network (SPADE generator) in fp16, fp32 in/out."""
    def __init__(self, net):
        super().__init__(); self.net = net.half()
    def forward(self, feature):
        return self.net(feature=feature.half()).float()


class FFTConv3d(nn.Module):
    """Exact 'same' Conv3d via FFT (for the big 7x7x7 mask conv). Weight spectrum is precomputed.
    MPS in torch 2.3 computes forward rfftn correctly but irfftn wrongly, so the (small, 22-channel)
    inverse transform runs on CPU. Extra memory: ~1.6 GB for the 22x142 kernel spectrum."""
    def __init__(self, conv: nn.Conv3d, fft_size=None):
        super().__init__()
        assert conv.stride == (1, 1, 1) and conv.groups == 1
        self.k = conv.kernel_size; self.p = conv.padding
        self.fft_size = fft_size
        self.O, self.C = conv.weight.shape[:2]
        self.register_buffer("bias", None if conv.bias is None else conv.bias.detach().clone())
        self._w = conv.weight.detach().cpu().clone()
        self.wcat = None

    def _prep(self, D, H, W, dev):
        S = self.fft_size or tuple(((n + k - 1 + 7) // 8) * 8 for n, k in zip((D, H, W), self.k))
        self.wcat = None
        Fn = S[0] * S[1] * (S[2] // 2 + 1)
        wcat = torch.empty(Fn, 2 * self.O, self.C, device=dev)  # one output channel at a time: low peak memory
        for o in range(self.O):
            wf = torch.fft.rfftn(torch.flip(self._w[o], dims=(1, 2, 3)), s=S, dim=(1, 2, 3)).reshape(self.C, Fn)
            wcat[:, o, :] = wf.real.t().to(dev); wcat[:, self.O + o, :] = wf.imag.t().to(dev)
        self.wcat = wcat; self.S = S; self.key = (D, H, W, str(dev))

    def forward(self, x):
        B, C, D, H, W = x.shape
        if self.wcat is None or self.key != (D, H, W, str(x.device)):
            self._prep(D, H, W, x.device)
        S, O = self.S, self.O
        outs = []
        for b in range(B):
            xp = F.pad(x[b].float(), (0, S[2] - W, 0, S[1] - H, 0, S[0] - D))
            xf = torch.fft.rfftn(xp, dim=(1, 2, 3))
            fshape = xf.shape[1:]; Fn = xf[0].numel()
            xc = torch.stack([xf.real.reshape(C, Fn), xf.imag.reshape(C, Fn)], 2).permute(1, 0, 2)  # F, C, 2
            o = torch.bmm(self.wcat, xc)  # F, 2O, 2
            o_r = o[:, :O, 0] - o[:, O:, 1]; o_i = o[:, :O, 1] + o[:, O:, 0]
            if INVERSE_ON_DEVICE:  # torch >= 2.14: MPS irfftn is correct
                of = torch.complex(o_r.t().reshape(O, *fshape), o_i.t().reshape(O, *fshape))
            else:  # torch 2.3: MPS irfftn is wrong, do the small inverse on CPU
                of = torch.complex(o_r.t().reshape(O, *fshape).cpu(), o_i.t().reshape(O, *fshape).cpu())
            y = torch.fft.irfftn(of, s=S, dim=(1, 2, 3))
            pd, ph, pw = (k - 1 - p for k, p in zip(self.k, self.p))
            y = y[:, pd:pd + D, ph:ph + H, pw:pw + W].to(x.device)
            outs.append(y)
        y = torch.stack(outs)
        if self.bias is not None:
            y = y + self.bias.to(y.device).view(1, -1, 1, 1, 1)
        return y.to(x.dtype)


def patch_mask_fft(warping_module):
    dm = warping_module.dense_motion_network
    if isinstance(dm.mask, nn.Conv3d):
        dm.mask = FFTConv3d(dm.mask)
    return warping_module
