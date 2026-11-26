//! The LaMa inpainting model, built into the program.
//!
//! `lama_fp32.onnx` as published at <https://huggingface.co/Carve/LaMa-ONNX> (Apache-2.0),
//! unchanged. It lives in its own crate so its 200 MB are compiled once and not again every
//! time BestOfBurst's own code changes. The file is not kept in git: `scripts/fetch-lama.ps1`
//! puts it in `assets/models/` (see the README there), and `build.rs` asks for it when missing.

/// The model file's name.
pub const FILE: &str = "lama_fp32.onnx";
/// Its SHA-256 (from the Git LFS pointer of the Carve/LaMa-ONNX repository).
pub const SHA256: &str = "1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6";
/// The whole ONNX file.
pub static ONNX: &[u8] = include_bytes!("../../../assets/models/lama_fp32.onnx");
