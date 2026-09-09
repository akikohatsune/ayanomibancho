#![allow(dead_code)]

use super::uleb128::read_uleb128;
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{self, Cursor, Error, ErrorKind, Read};

pub struct PacketReader<'a> {
    cursor: Cursor<&'a [u8]>,
}

impl<'a> PacketReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            cursor: Cursor::new(data),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.cursor.position() as usize >= self.cursor.get_ref().len()
    }

    pub fn remaining(&self) -> usize {
        self.cursor.get_ref().len().saturating_sub(self.cursor.position() as usize)
    }

    pub fn read_packet_header(&mut self) -> io::Result<Option<(u16, usize)>> {
        if self.remaining() < 7 {
            return Ok(None);
        }

        let packet_id = self.cursor.read_u16::<LittleEndian>()?;
        let _compression = self.cursor.read_u8()?;
        let length = self.cursor.read_u32::<LittleEndian>()? as usize;

        Ok(Some((packet_id, length)))
    }

    pub fn read_u8(&mut self) -> io::Result<u8> {
        self.cursor.read_u8()
    }

    pub fn read_bool(&mut self) -> io::Result<bool> {
        Ok(self.cursor.read_u8()? != 0)
    }

    pub fn read_i8(&mut self) -> io::Result<i8> {
        self.cursor.read_i8()
    }

    pub fn read_u16(&mut self) -> io::Result<u16> {
        self.cursor.read_u16::<LittleEndian>()
    }

    pub fn read_i16(&mut self) -> io::Result<i16> {
        self.cursor.read_i16::<LittleEndian>()
    }

    pub fn read_u32(&mut self) -> io::Result<u32> {
        self.cursor.read_u32::<LittleEndian>()
    }

    pub fn read_i32(&mut self) -> io::Result<i32> {
        self.cursor.read_i32::<LittleEndian>()
    }

    pub fn read_u64(&mut self) -> io::Result<u64> {
        self.cursor.read_u64::<LittleEndian>()
    }

    pub fn read_i64(&mut self) -> io::Result<i64> {
        self.cursor.read_i64::<LittleEndian>()
    }

    pub fn read_f32(&mut self) -> io::Result<f32> {
        self.cursor.read_f32::<LittleEndian>()
    }

    pub fn read_f64(&mut self) -> io::Result<f64> {
        self.cursor.read_f64::<LittleEndian>()
    }

    pub fn read_bytes(&mut self, len: usize) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        self.cursor.read_exact(&mut buf)?;
        Ok(buf)
    }

    pub fn read_osu_string(&mut self) -> io::Result<String> {
        if self.remaining() == 0 {
            return Err(Error::new(ErrorKind::UnexpectedEof, "EOF before string indicator"));
        }

        let indicator = self.cursor.read_u8()?;
        if indicator == 0x00 {
            return Ok(String::new());
        }

        if indicator != 0x0B {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("Invalid osu string indicator: {:#x}", indicator),
            ));
        }

        let pos = self.cursor.position() as usize;
        let mut slice = &self.cursor.get_ref()[pos..];
        let len = read_uleb128(&mut slice)? as usize;
        let uleb_bytes_read = (self.cursor.get_ref().len() - pos) - slice.len();
        self.cursor.set_position((pos + uleb_bytes_read) as u64);

        let mut str_bytes = vec![0u8; len];
        self.cursor.read_exact(&mut str_bytes)?;

        String::from_utf8(str_bytes).map_err(|e| Error::new(ErrorKind::InvalidData, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::writer::PacketWriter;

    #[test]
    fn test_string_roundtrip() {
        let mut writer = PacketWriter::new();
        writer.write_osu_string("Hello, osu! Bancho");
        writer.write_osu_string("");
        writer.write_osu_string("Tiếng Việt có dấu");

        let bytes = writer.into_bytes();
        let mut reader = PacketReader::new(&bytes);

        assert_eq!(reader.read_osu_string().unwrap(), "Hello, osu! Bancho");
        assert_eq!(reader.read_osu_string().unwrap(), "");
        assert_eq!(reader.read_osu_string().unwrap(), "Tiếng Việt có dấu");
    }
}
