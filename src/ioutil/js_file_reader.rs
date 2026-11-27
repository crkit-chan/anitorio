use std::io;

use crate::prelude::*;
use crate::ts;
use crate::util::JsCall as _;

pub type JsFileReaderCallback = ts::TsFn<fn(ts::TsNum, ts::TsNum) -> js_sys::Uint8Array>;

const MAX_SAFE_F64_INT: u64 = (1_u64 << 53) - 1;

// Note we use a ``u64`` for the position, because the file size can be larger
// than 2^32-1. Maximum pointer size for JS is 2^53.

/// A JavaScript file reader.
pub struct JsFileReader {
    callback: JsFileReaderCallback,
    pos:      u64,
    size:     u64,
}
impl JsFileReader {
    #[inline]
    #[must_use]
    pub const fn new(callback: JsFileReaderCallback, size: u64) -> Self {
        Self { callback, pos: 0, size }
    }

    #[inline]
    #[allow(unused, reason = "API")]
    pub const fn reset(&mut self) {
        self.pos = 0;
    }
}
impl io::Read for JsFileReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.pos >= self.size {
            return Ok(0);
        }

        let end = (self.pos + buf.len() as u64).min(self.size);
        assert!(
            end <= MAX_SAFE_F64_INT,
            "Cannot fetch beyond bounds: {} - {end}",
            self.pos,
        );

        let array = (self.callback)
            .call_n((&(self.pos as f64).into(), &(end as f64).into()))
            .map_err(|e| io::Error::other(format!("JS call error: {e:?}")))?;

        let count = (array.length() as usize).min(buf.len());
        array.copy_to(&mut buf[..count]);

        self.pos += count as u64;

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
        if unlikely(new_position < 0 || new_position > i128::from(u64::MAX)) {
            cold_path();
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid seek position",
            ));
        }
        self.pos = new_position as u64;
        Ok(self.pos)
    }
}
