use std::io::{self, Write};
use wasm_bindgen::prelude::*;

/// Custom writer that invokes a callback on every write
pub struct CallbackWriter<F: FnMut(Vec<u8>) -> Result<(), JsValue>> {
    on_chunk: F,
}
impl<F: FnMut(Vec<u8>) -> Result<(), JsValue>> CallbackWriter<F> {
    pub const fn new(on_chunk: F) -> Self {
        Self { on_chunk }
    }
}

impl<F: FnMut(Vec<u8>) -> Result<(), JsValue>> Write for CallbackWriter<F> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        (self.on_chunk)(data.to_vec())
            .map_err(|e| io::Error::other(e.as_string().unwrap_or_else(|| format!("{e:?}"))))?;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
