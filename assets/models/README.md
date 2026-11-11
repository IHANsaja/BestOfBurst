# Face models

`face_detector.tflite` (BlazeFace short range) and `face_landmarks_detector.tflite` (478-point
face mesh) are taken unchanged from Google's MediaPipe Face Landmarker bundle:

https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1/face_landmarker.task

Copyright Google LLC, licensed under the Apache License 2.0
(https://www.apache.org/licenses/LICENSE-2.0). They are built into BestOfBurst and run on the
processor, offline, for the closed-eyes check (`src/faces.rs`).
