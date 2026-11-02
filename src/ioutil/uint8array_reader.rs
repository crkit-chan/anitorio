use js_sys::Uint8Array;
use std::io::{self, Read};

/// Custom reader over a JavaScript-owned Uint8Array.
pub struct Uint8ArrayReader {
    data: Uint8Array,
    pos: u32,
}

impl Uint8ArrayReader {
    pub fn new(data: Uint8Array) -> Self {
        Self { data, pos: 0 }
    }
}

impl Read for Uint8ArrayReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let remaining = self.data.length().saturating_sub(self.pos);
        let len = buf.len().min(remaining as usize);
        if len != 0 {
            self.data
                .subarray(self.pos, self.pos + len as u32)
                .copy_to(&mut buf[..len]);
            self.pos += len as u32;
        }
        Ok(len)
    }
}
