//! XMODEM / ZMODEM の CRC。

/// CRC-16/XMODEM（多項式 0x1021、初期値 0）
pub fn crc16(data: &[u8]) -> u16 {
    crc16_update(0, data)
}

pub fn crc16_update(mut crc: u16, data: &[u8]) -> u16 {
    for &b in data {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// CRC-32（ZMODEM の 32 ビット CRC。IEEE 802.3、初期値 0xFFFFFFFF、最後に反転）
pub fn crc32(data: &[u8]) -> u32 {
    !crc32_update(0xffff_ffff, data)
}

pub fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    crc
}

/// XMODEM のチェックサム（全バイトの和の下位 8 ビット）
pub fn checksum(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |a, &b| a.wrapping_add(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        // 標準の検査値（"123456789"）
        assert_eq!(crc16(b"123456789"), 0x31c3);
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(checksum(&[0xff, 0x02]), 0x01);
    }
}
