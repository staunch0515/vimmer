# coding: utf-8
"""
Trial patch: MediaPipe FaceLandmarker (Apache-2.0) as a replacement for the
InsightFace buffalo_l detectors (det_10g.onnx + 2d106det.onnx, non-commercial).

Enabled with env JOYVASA_DETECTOR=mediapipe. Returns LivePortrait's own 203-point
landmarks (landmark.onnx), seeded from the 478-point MediaPipe mesh, so the rest of
the pipeline (crop, lip/eye ratios) sees the same landmark format it already supports.
"""
import os
import time
import numpy as np
import mediapipe as mp
from mediapipe.tasks.python import vision
from mediapipe.tasks.python.core.base_options import BaseOptions


class MediaPipeFace(object):
    def __init__(self, model_path, min_conf=0.3, max_faces=4):
        opts = vision.FaceLandmarkerOptions(
            base_options=BaseOptions(model_asset_path=model_path),
            running_mode=vision.RunningMode.IMAGE,
            num_faces=max_faces,
            min_face_detection_confidence=min_conf,
            min_face_presence_confidence=min_conf,
        )
        self.landmarker = vision.FaceLandmarker.create_from_options(opts)
        self.last_elapsed = 0.0

    def mesh478(self, img_rgb):
        """Largest face as a 478x2 pixel array, or None."""
        t0 = time.time()
        h, w = img_rgb.shape[:2]
        image = mp.Image(image_format=mp.ImageFormat.SRGB, data=np.ascontiguousarray(img_rgb))
        res = self.landmarker.detect(image)
        self.last_elapsed = time.time() - t0
        if not res.face_landmarks:
            return None
        faces = [np.array([[p.x * w, p.y * h] for p in lm], dtype=np.float32) for lm in res.face_landmarks]
        areas = [np.ptp(f[:, 0]) * np.ptp(f[:, 1]) for f in faces]
        return faces[int(np.argmax(areas))]

    def lmk203(self, img_rgb, human_landmark_runner):
        mesh = self.mesh478(img_rgb)
        if mesh is None:
            return None
        return human_landmark_runner.run(img_rgb, mesh)
