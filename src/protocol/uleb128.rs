use std::io::{self, Error, ErrorKind};

/// Writes a 64-bit integer encoded as ULEB128 into the destination vector.
pub fn write_uleb128(mut value: u64, buf: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// Reads a ULEB128 encoded integer from a byte slice cursor, advancing the slice.
pub fn read_uleb128(cursor: &mut &[u8]) -> io::Result<u64> {
    let mut result: u64 = 0;
    let mut shift = 0;

    loop {
        if cursor.is_empty() {
            return Err(Error::new(ErrorKind::UnexpectedEof, "Unexpected EOF while reading ULEB128"));
        }
        let byte = cursor[0];
        *cursor = &cursor[1..];

        result |= ((byte & 0x7F) as u64) << shift;
        if (byte & 0x80) == 0 {
            break;
        }
        shift += 7;
        if shift >= 64 {
            return Err(Error::new(ErrorKind::InvalidData, "ULEB128 integer overflow"));
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uleb128_roundtrip() {
        let cases = [0u64, 1, 127, 128, 255, 300, 16384, 1000000];
        for &val in &cases {
            let mut buf = Vec::new();
            write_uleb128(val, &mut buf);
            let mut slice = &buf[..];
            let read_val = read_uleb128(&mut slice).expect("read failed");
            assert_eq!(val, read_val);
            assert!(slice.is_empty());
        }
    }
}
