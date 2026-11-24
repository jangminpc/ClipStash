//! 이미지 속 글자 추출(OCR). Windows 10/11 에 들어 있는 Windows.Media.Ocr 을 씁니다.
//! 한국어는 "한국어" 언어 팩(또는 그 안의 광학 문자 인식 기능)이 깔려 있어야 합니다.

use std::ptr::null_mut;

use windows_sys::Win32::Graphics::Gdi::*;

/// 클립보드 DIB → 위에서 아래 순서의 BGRA 픽셀. 형식 변환은 GDI 에 맡깁니다.
/// 글자 인식이 잘 되도록 작은 그림은 2배로 키우고, 너무 큰 그림은 줄입니다.
pub fn dib_to_bgra(dib: &[u8], max_side: u32) -> Option<(u32, u32, Vec<u8>)> {
    let (w, h) = clipstash::bmp::dib_size(dib)?;
    let off = clipstash::bmp::pixel_offset(dib)? as usize;
    if w == 0 || h == 0 || off >= dib.len() {
        return None;
    }
    let big = w.max(h) as f64;
    let s = if big > max_side as f64 {
        max_side as f64 / big
    } else if big < 600.0 {
        2.0
    } else {
        1.0
    };
    let tw = ((w as f64 * s) as u32).max(1);
    let th = ((h as f64 * s) as u32).max(1);
    unsafe {
        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = tw as i32;
        bi.bmiHeader.biHeight = -(th as i32);
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        let screen = GetDC(null_mut());
        let mut bits: *mut core::ffi::c_void = null_mut();
        let sec = CreateDIBSection(screen, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        let mem = CreateCompatibleDC(screen);
        ReleaseDC(null_mut(), screen);
        if sec.is_null() || bits.is_null() {
            DeleteDC(mem);
            return None;
        }
        let old = SelectObject(mem, sec as HGDIOBJ);
        SetStretchBltMode(mem, HALFTONE);
        SetBrushOrgEx(mem, 0, 0, null_mut());
        StretchDIBits(
            mem,
            0,
            0,
            tw as i32,
            th as i32,
            0,
            0,
            w as i32,
            h as i32,
            dib.as_ptr().add(off) as *const _,
            dib.as_ptr() as *const BITMAPINFO,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
        GdiFlush();
        let len = (tw * th * 4) as usize;
        let mut px = std::slice::from_raw_parts(bits as *const u8, len).to_vec();
        SelectObject(mem, old);
        DeleteObject(sec as HGDIOBJ);
        DeleteDC(mem);
        for a in px.iter_mut().skip(3).step_by(4) {
            *a = 255;
        }
        Some((tw, th, px))
    }
}

/// 글자 인식(오래 걸릴 수 있으니 작업 스레드에서 부르세요).
pub fn recognize(width: u32, height: u32, bgra: Vec<u8>) -> Result<String, String> {
    use windows::core::HSTRING;
    use windows::Globalization::Language;
    use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::DataWriter;
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

    let err = |what: &str, e: windows::core::Error| format!("{}: {}", what, e.message());
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    let engine = OcrEngine::TryCreateFromUserProfileLanguages()
        .ok()
        .or_else(|| {
            ["ko", "ko-KR", "en-US"].iter().find_map(|tag| {
                Language::CreateLanguage(&HSTRING::from(*tag)).and_then(|l| OcrEngine::TryCreateFromLanguage(&l)).ok()
            })
        })
        .ok_or_else(|| {
            "이 PC 에 글자 인식(OCR) 언어가 설치되어 있지 않습니다.\nWindows 설정 → 시간 및 언어 → 언어 및 지역 → 한국어 → 언어 옵션에서 \"광학 문자 인식\"을 설치해 주세요."
                .to_string()
        })?;
    let writer = DataWriter::new().map_err(|e| err("메모리", e))?;
    writer.WriteBytes(&bgra).map_err(|e| err("메모리", e))?;
    let buffer = writer.DetachBuffer().map_err(|e| err("메모리", e))?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, width as i32, height as i32)
        .map_err(|e| err("이미지 변환", e))?;
    let result = engine
        .RecognizeAsync(&bitmap)
        .and_then(|op| op.get())
        .map_err(|e| err("글자 인식", e))?;
    let mut lines = Vec::new();
    for line in result.Lines().map_err(|e| err("글자 인식", e))? {
        if let Ok(t) = line.Text() {
            let t = t.to_string_lossy();
            if !t.trim().is_empty() {
                lines.push(t);
            }
        }
    }
    if lines.is_empty() {
        return Err("이미지에서 글자를 찾지 못했습니다.".into());
    }
    Ok(lines.join("\n"))
}

/// OCR 에 넘길 최대 크기(엔진 제한 10000 보다 넉넉히 작게)
pub const MAX_SIDE: u32 = 4000;
