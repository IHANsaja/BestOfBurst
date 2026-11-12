# Face models

`face_detector.tflite` (BlazeFace short range) and `face_landmarks_detector.tflite` (478-point
face mesh) are taken unchanged from Google's MediaPipe Face Landmarker bundle:

https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1/face_landmarker.task

Copyright Google LLC, licensed under the Apache License 2.0
(https://www.apache.org/licenses/LICENSE-2.0). They are built into BestOfBurst and run on the
processor, offline, for the closed-eyes check (`src/faces.rs`).

# Inpainting model (optional, not in the repository)

The watermark remover's AI edge clean-up (`src/inpaint.rs`) uses **LaMa** (Suvorov et al.,
"Resolution-robust Large Mask Inpainting with Fourier Convolutions") as exported to ONNX by
Carve: `lama_fp32.onnx` from https://huggingface.co/Carve/LaMa-ONNX, licensed under the Apache
License 2.0.

- Size: 208,044,816 bytes
- SHA-256: `1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6`
- Input `image`: float32 [1, 3, 512, 512], RGB 0..1. Input `mask`: float32 [1, 1, 512, 512],
  1 = paint. Output `output`: float32 [1, 3, 512, 512], RGB 0..255.

It is too big to build in, so it is not committed. `scripts\fetch-lama.ps1` downloads it to
`%LOCALAPPDATA%\BestOfBurst\models\` and checks the hash; a `models` folder beside
`bestofburst.exe` works too. BestOfBurst checks the hash again before loading it and runs it with
`tract` on the processor, offline. Without it, the remover uses its classical edge fill.
