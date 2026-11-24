//! 클립보드의 CF_DIB 데이터 <-> .bmp 파일 변환.
//! DIB 앞에 14바이트 BITMAPFILEHEADER 만 붙이면 BMP 파일이 됩니다.

const BI_BITFIELDS: u32 = 3;

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    b.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
fn i32_at(b: &[u8], i: usize) -> Option<i32> {
    u32_at(b, i).map(|v| v as i32)
}
fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

/// (가로, 세로) 픽셀 크기
pub fn dib_size(dib: &[u8]) -> Option<(u32, u32)> {
    let header = u32_at(dib, 0)?;
    if header == 12 {
        // BITMAPCOREHEADER
        return Some((u16_at(dib, 4)? as u32, u16_at(dib, 6)? as u32));
    }
    if header < 40 {
        return None;
    }
    Some((i32_at(dib, 4)?.unsigned_abs(), i32_at(dib, 8)?.unsigned_abs()))
}

/// 픽셀 데이터가 시작되는 위치(DIB 시작 기준)
pub fn pixel_offset(dib: &[u8]) -> Option<u32> {
    let header = u32_at(dib, 0)?;
    if header == 12 {
        let bits = u16_at(dib, 10)? as u32;
        let colors = if bits <= 8 { 1 << bits } else { 0 };
        return Some(12 + colors * 3);
    }
    let bits = u16_at(dib, 14)? as u32;
    let compression = u32_at(dib, 16)?;
    let clr_used = u32_at(dib, 32)?;
    let colors = if clr_used != 0 { clr_used } else if bits <= 8 { 1 << bits } else { 0 };
    let masks = if header == 40 && compression == BI_BITFIELDS { 12 } else { 0 };
    Some(header + masks + colors * 4)
}

pub fn dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    let off = 14 + pixel_offset(dib)?;
    let total = 14 + dib.len() as u32;
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&off.to_le_bytes());
    out.extend_from_slice(dib);
    Some(out)
}

pub fn bmp_to_dib(bmp: &[u8]) -> Option<&[u8]> {
    if bmp.len() < 14 + 12 || &bmp[0..2] != b"BM" {
        return None;
    }
    Some(&bmp[14..])
}

/// 위에서 아래 순서의 BGRA 픽셀(가로×세로×4) → 클립보드용 24비트 DIB(아래에서 위 순서).
/// 32비트 DIB 는 일부 프로그램이 알파(투명도)를 잘못 해석해서 24비트로 만듭니다.
pub fn bgra_to_dib24(width: u32, height: u32, bgra: &[u8]) -> Option<Vec<u8>> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || bgra.len() < w * h * 4 {
        return None;
    }
    let stride = (w * 3).div_ceil(4) * 4;
    let mut d = Vec::with_capacity(40 + stride * h);
    d.extend_from_slice(&40u32.to_le_bytes());
    d.extend_from_slice(&(width as i32).to_le_bytes());
    d.extend_from_slice(&(height as i32).to_le_bytes()); // 양수 = 아래에서 위
    d.extend_from_slice(&1u16.to_le_bytes());
    d.extend_from_slice(&24u16.to_le_bytes());
    d.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    d.extend_from_slice(&((stride * h) as u32).to_le_bytes());
    d.extend_from_slice(&[0u8; 16]);
    for y in (0..h).rev() {
        let row = &bgra[y * w * 4..(y + 1) * w * 4];
        let start = d.len();
        for px in row.chunks_exact(4) {
            d.extend_from_slice(&px[..3]);
        }
        d.resize(start + stride, 0);
    }
    Some(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_to_dib() {
        // 2×2: 위 줄 빨강·초록, 아래 줄 파랑·흰색
        let px = [0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 255, 255, 255, 255];
        let d = bgra_to_dib24(2, 2, &px).unwrap();
        assert_eq!(dib_size(&d), Some((2, 2)));
        assert_eq!(pixel_offset(&d), Some(40));
        assert_eq!(d.len(), 40 + 8 * 2);
        // 첫 줄(파일상)은 그림의 아래 줄
        assert_eq!(&d[40..46], &[255, 0, 0, 255, 255, 255]);
        assert_eq!(&d[48..54], &[0, 0, 255, 0, 255, 0]);
        assert!(bgra_to_dib24(2, 2, &px[..8]).is_none());
    }

    fn dib_2x1_24bit() -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&40u32.to_le_bytes());
        d.extend_from_slice(&2i32.to_le_bytes());
        d.extend_from_slice(&(-1i32).to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&24u16.to_le_bytes());
        d.extend_from_slice(&[0u8; 24]);
        d.extend_from_slice(&[255, 0, 0, 0, 255, 0, 0, 0]); // 2픽셀 + 패딩
        d
    }

    #[test]
    fn roundtrip() {
        let dib = dib_2x1_24bit();
        assert_eq!(dib_size(&dib), Some((2, 1)));
        let bmp = dib_to_bmp(&dib).unwrap();
        assert_eq!(&bmp[0..2], b"BM");
        assert_eq!(u32_at(&bmp, 10), Some(54));
        assert_eq!(u32_at(&bmp, 2), Some(bmp.len() as u32));
        assert_eq!(bmp_to_dib(&bmp).unwrap(), &dib[..]);
    }

    #[test]
    fn bitfields_offset() {
        let mut dib = dib_2x1_24bit();
        dib[14] = 32;
        dib[16] = BI_BITFIELDS as u8;
        assert_eq!(pixel_offset(&dib), Some(52));
    }

    #[test]
    fn garbage() {
        assert_eq!(dib_size(&[1, 2]), None);
        assert!(bmp_to_dib(b"nope").is_none());
    }
}
