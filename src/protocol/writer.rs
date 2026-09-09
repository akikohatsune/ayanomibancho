#![allow(dead_code)]

use super::uleb128::write_uleb128;
use byteorder::{LittleEndian, WriteBytesExt};

#[derive(Default, Debug, Clone)]
pub struct PacketWriter {
    buffer: Vec<u8>,
}

impl PacketWriter {
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    pub fn write_u8(&mut self, val: u8) {
        self.buffer.push(val);
    }

    pub fn write_bool(&mut self, val: bool) {
        self.buffer.push(if val { 1 } else { 0 });
    }

    pub fn write_i8(&mut self, val: i8) {
        self.buffer.push(val as u8);
    }

    pub fn write_u16(&mut self, val: u16) {
        self.buffer.write_u16::<LittleEndian>(val).unwrap();
    }

    pub fn write_i16(&mut self, val: i16) {
        self.buffer.write_i16::<LittleEndian>(val).unwrap();
    }

    pub fn write_u32(&mut self, val: u32) {
        self.buffer.write_u32::<LittleEndian>(val).unwrap();
    }

    pub fn write_i32(&mut self, val: i32) {
        self.buffer.write_i32::<LittleEndian>(val).unwrap();
    }

    pub fn write_u64(&mut self, val: u64) {
        self.buffer.write_u64::<LittleEndian>(val).unwrap();
    }

    pub fn write_i64(&mut self, val: i64) {
        self.buffer.write_i64::<LittleEndian>(val).unwrap();
    }

    pub fn write_f32(&mut self, val: f32) {
        self.buffer.write_f32::<LittleEndian>(val).unwrap();
    }

    pub fn write_f64(&mut self, val: f64) {
        self.buffer.write_f64::<LittleEndian>(val).unwrap();
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    pub fn write_osu_string(&mut self, str_val: &str) {
        if str_val.is_empty() {
            self.buffer.push(0x00);
        } else {
            self.buffer.push(0x0B);
            write_uleb128(str_val.len() as u64, &mut self.buffer);
            self.buffer.extend_from_slice(str_val.as_bytes());
        }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buffer
    }

    /// Wraps a raw payload into a complete Bancho packet:
    /// [packet_id: u16][0: u8][length: u32][payload: bytes]
    pub fn build_packet(packet_id: u16, payload: &[u8]) -> Vec<u8> {
        let mut packet = Vec::with_capacity(7 + payload.len());
        packet.write_u16::<LittleEndian>(packet_id).unwrap();
        packet.push(0); // compression flag (unused in osu! stable)
        packet.write_u32::<LittleEndian>(payload.len() as u32).unwrap();
        packet.extend_from_slice(payload);
        packet
    }
}
