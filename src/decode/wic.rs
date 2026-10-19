//! Windows Imaging Component fallback: HEIC, HEIF, AVIF, JPEG XR, and RAW without a usable preview.

use std::path::Path;

use windows::core::HSTRING;
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;

use super::RgbImage;

/// COM stays initialized for the life of each worker thread.
struct ComGuard;

impl ComGuard {
    fn new() -> Self {
        // SAFETY: plain COM initialization; failure (e.g. already initialized in another
        // apartment mode) still leaves COM usable for in-proc WIC.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        ComGuard
    }
}

thread_local! {
    static COM: ComGuard = ComGuard::new();
}

/// HRESULT for "no codec installed for this format".
const WINCODEC_ERR_COMPONENTNOTFOUND: u32 = 0x88982F50;

pub const MISSING_CODEC_MSG: &str = "No Windows codec for this format. Install HEIF Image Extensions + \
    HEVC Video Extensions (HEIC), AV1 Video Extension (AVIF), or Raw Image Extension (RAW).";

pub fn decode_wic(path: &Path) -> anyhow::Result<RgbImage> {
    COM.with(|_| ());
    // SAFETY: standard WIC call sequence on interfaces owned by this function; the pixel
    // buffer is sized to stride * height before CopyPixels writes into it.
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let decoder = factory
            .CreateDecoderFromFilename(
                &HSTRING::from(path.as_os_str()),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )
            .map_err(wic_error)?;
        let frame = decoder.GetFrame(0).map_err(wic_error)?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h)?;
        if w == 0 || h == 0 {
            anyhow::bail!("wic: empty image");
        }
        let conv = factory.CreateFormatConverter()?;
        conv.Initialize(
            &frame,
            &GUID_WICPixelFormat24bppRGB,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )
        .map_err(wic_error)?;
        let stride = w * 3;
        let mut rgb = vec![0u8; stride as usize * h as usize];
        conv.CopyPixels(std::ptr::null(), stride, &mut rgb).map_err(wic_error)?;
        RgbImage::new(w, h, rgb)
    }
}

/// WINCODEC_ERR_COMPONENTNOTFOUND (0x88982F50) means no codec is installed.
fn wic_error(e: windows::core::Error) -> anyhow::Error {
    if e.code().0 as u32 == WINCODEC_ERR_COMPONENTNOTFOUND {
        anyhow::anyhow!(MISSING_CODEC_MSG)
    } else {
        anyhow::anyhow!("wic: {e}")
    }
}

pub fn is_missing_codec(msg: &str) -> bool {
    msg.starts_with("No Windows codec")
}
