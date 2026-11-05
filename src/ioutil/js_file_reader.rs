use crate::{ts, util::JsCall};
use std::io;

pub type JsFileReaderCallback = ts::TsFn<fn(ts::TsNum, ts::TsNum) -> js_sys::Uint8Array>;

const MAX_SAFE_F64_INT: u64 = (1u64 << 53) - 1;

// Note we use a ``u64`` for the position, because the file size can be larger than 2^32-1.
// Maximum pointer size for JS is 2^53.

/// A JavaScript file reader.
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
            assert!(start <= MAX_SAFE_F64_INT && end <= MAX_SAFE_F64_INT);

            let array = (self.callback)
                .call_n((&(start as f64).into(), &(end as f64).into()))
                .map_err(|e| io::Error::other(format!("JS call error: {e:?}")))?;

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
            io::SeekFrom::Start(n) => i128::from(n),
            io::SeekFrom::End(n) => i128::from(self.size) + i128::from(n),
            io::SeekFrom::Current(n) => i128::from(self.pos) + i128::from(n),
        };
        if new_position < 0 || new_position > i128::from(u64::MAX) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid seek position",
            ));
        }
        self.pos = new_position as u64;

        Ok(self.pos)
    }
}
