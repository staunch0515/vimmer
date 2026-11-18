"""python3 -m unittest discover -s rust-core/crates/vimmer-media/scripts/avatar  (needs numpy only)"""
import math
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import numpy as np  # noqa: E402
import avatar_math as am  # noqa: E402


def rot_x(deg):
    a = math.radians(deg)
    return np.array([[1, 0, 0], [0, math.cos(a), -math.sin(a)], [0, math.sin(a), math.cos(a)]])


class SquareFrame(unittest.TestCase):
    def test_centered_below_face(self):
        x0, y0, side = am.square_frame((400, 200, 600, 450), 1000, 1000, scale=2.4, down=0.12)
        self.assertEqual(side, 600)
        self.assertEqual(x0 + side // 2, 500)
        self.assertGreater(y0 + side / 2, 325)  # centre is below the face centre

    def test_crop_padded_pads_outside(self):
        img = np.arange(4 * 4 * 3, dtype=np.uint8).reshape(4, 4, 3)
        out = am.crop_padded(img, -2, -1, 6)
        self.assertEqual(out.shape, (6, 6, 3))
        self.assertTrue((out[2, 3] == img[1, 1]).all())  # inside the photo: untouched

    def test_crop_inside_is_exact(self):
        img = np.arange(8 * 8 * 3, dtype=np.uint8).reshape(8, 8, 3)
        self.assertTrue((am.crop_padded(img, 1, 2, 4) == img[2:6, 1:5]).all())

    def test_padding_is_soft_not_streaked(self):
        # a photo whose bottom row has a hard vertical edge: the made-up rows below must blur it
        img = np.zeros((100, 100, 3), dtype=np.uint8)
        img[:, 50:] = 255
        out = am.crop_padded(img, 0, 0, 200).astype(int)
        self.assertTrue((out[:90, :90] == np.pad(img, ((0, 0), (0, 0), (0, 0)))[:90, :90]).all())
        far = out[190, :100, 0]
        self.assertLess(np.abs(np.diff(far)).max(), 60)  # no hard step far from the photo

    def test_visible_square(self):
        # the photo covers the frame: show the whole frame
        self.assertEqual(am.visible_square((400, 300, 600, 500), 1000, 1000, (260, 220, 480)), (260, 220, 480))
        # portrait photo cut at the shoulders (600x900, frame 950 wide): a 600 square of real pixels
        box = (122, 215, 461, 611)
        frame = am.square_frame(box, 600, 900)
        vx, vy, side = am.visible_square(box, 600, 900, frame)
        self.assertEqual((vx, side), (0, 600))
        self.assertTrue(0 <= vy and vy + side <= 900)
        self.assertTrue(vy < 215 and vy + side >= 611 + 0.08 * side)  # forehead to chin, with room
        # a photo that is nearly all face keeps at least 60 % of the frame
        frame = am.square_frame((10, 10, 190, 190), 200, 200)
        vx, vy, side = am.visible_square((10, 10, 190, 190), 200, 200, frame)
        self.assertEqual(side, int(np.ceil(0.6 * frame[2])))
        self.assertTrue(frame[0] <= vx and vx + side <= frame[0] + frame[2])


class PauseGate(unittest.TestCase):
    def test_gate_marks_long_pauses_only(self):
        sr, fps = 16000, 25
        hop = sr // fps
        x = np.ones(50 * hop) * 0.3
        x[10 * hop:20 * hop] = 0.0   # a 10-frame pause
        x[30 * hop:32 * hop] = 0.0   # 2 frames: too short to count
        g = am.pause_gate(x, sr, fps, 50)
        self.assertEqual(g.shape, (50,))
        self.assertTrue((g[13:17] == 1.0).all())
        self.assertTrue(0 < g[10] < 1 and 0 < g[19] < 1)  # ramps at both ends
        self.assertTrue((g[:7] == 0).all() and (g[23:] == 0).all())

    def test_gate_of_silence_and_of_steady_sound(self):
        self.assertTrue((am.pause_gate(np.zeros(16000), 16000, 25, 25) == 1).all())
        self.assertTrue((am.pause_gate(np.ones(16000), 16000, 25, 25) == 0).all())


class PoseRefs(unittest.TestCase):
    def test_moving_mean_edges_and_full(self):
        v = np.arange(10, dtype=float)
        self.assertTrue(np.allclose(am.moving_mean(v, 0), 4.5))
        m = am.moving_mean(v, 3)
        self.assertAlmostEqual(m[0], 0.5)
        self.assertAlmostEqual(m[5], 5.0)

    def test_frame0_is_upstream(self):
        rots = np.stack([rot_x(d) for d in (5, 10, 15)])
        r, t, s = am.pose_refs(rots, np.zeros((3, 3)), np.ones(3), "frame0")
        self.assertTrue(np.allclose(r, rot_x(5)))

    def test_window_removes_slow_drift(self):
        # pitch drifts 0 -> 30 deg over 1000 frames plus a fast 3 deg nod
        n = 1000
        pitch = np.linspace(0, 30, n) + 3 * np.sin(np.arange(n) / 5.0)
        rots = np.stack([rot_x(p) for p in pitch])
        r_ref, _, _ = am.pose_refs(rots, np.zeros((n, 3)), np.ones(n), "window", 200)
        rel = am.pitch_deg(np.einsum("nij,nkj->nik", rots, r_ref))
        self.assertLess(np.abs(rel[100:-100]).max(), 5.0)          # drift gone
        self.assertGreater(np.abs(rel[100:-100]).max(), 2.0)       # nod kept
        r0, _, _ = am.pose_refs(rots, np.zeros((n, 3)), np.ones(n), "frame0")
        rel0 = am.pitch_deg(np.einsum("nij,nkj->nik", rots, r0))
        self.assertGreater(np.abs(rel0).max(), 25.0)               # upstream drifts

    def test_refs_are_rotations(self):
        rots = np.stack([rot_x(d) for d in np.linspace(-20, 20, 50)])
        r, _, _ = am.pose_refs(rots, np.zeros((50, 3)), np.ones(50), "mean")
        self.assertTrue(np.allclose(r[0] @ r[0].T, np.eye(3), atol=1e-9))


if __name__ == "__main__":
    unittest.main()
