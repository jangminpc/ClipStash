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

#[cfg(test)]
mod tests {
    use super::*;

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
