use std::io;

use crate::util::{JsCall, js_num};

pub type JsFileReaderCallback =
    js_sys::Function<fn(js_sys::Number, js_sys::Number) -> js_sys::Uint8Array>;
pub struct JsFileReader {
    callback: JsFileReaderCallback,
    pos: u64,
    size: u64,

    cache: Vec<u8>,
    cache_start: u64,
    chunk_size: usize,
}
impl JsFileReader {
    pub fn new(callback: JsFileReaderCallback, size: u64, chunk_size: usize) -> Self {
        Self {
            callback,
            pos: 0,
            size,
            cache: Vec::new(),
            cache_start: 0,
            chunk_size,
        }
    }

    fn fetch(&mut self) -> io::Result<()> {
        let start = self.pos;
        let end = (start + self.chunk_size as u64).min(self.size);

        if start >= end {
            self.cache.clear();
        } else {
            let array = (self.callback)
                .call_n((&js_num(start as f64), &js_num(end as f64)))
                .map_err(|e| {
                    io::Error::new(io::ErrorKind::Other, format!("JS call error: {e:?}"))
                })?;

            self.cache.resize(array.length() as usize, 0);
            array.copy_to(&mut self.cache);

            self.cache_start = start;
        }
        Ok(())
    }
}
impl io::Read for JsFileReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.pos >= self.size {
            return Ok(0);
        }

        let cache_end = self.cache_start + self.cache.len() as u64;
        if self.pos < self.cache_start || self.pos >= cache_end {
            self.fetch()?;
        }

        let offset = (self.pos - self.cache_start) as usize;
        let available = self.cache.len().saturating_sub(offset);
        let count = available.min(buf.len());

        if count != 0 {
            buf[..count].copy_from_slice(&self.cache[offset..offset + count]);
            self.pos += count as u64;
        }
        Ok(count)
    }
}
impl io::Seek for JsFileReader {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        let new_position = match pos {
            io::SeekFrom::Start(n) => n as i128,
            io::SeekFrom::End(n) => self.size as i128 + n as i128,
            io::SeekFrom::Current(n) => self.pos as i128 + n as i128,
        };
        if new_position < 0 || new_position > u64::MAX as i128 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid seek position",
            ));
        }
        self.pos = new_position as u64;

        Ok(self.pos)
    }
}
