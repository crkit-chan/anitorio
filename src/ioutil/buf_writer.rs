use std::io::{self, Write};
use wasm_bindgen::prelude::*;

enum BufWriterState<F: FnMut(Vec<u8>) -> Result<(), JsValue>> {
    Callback(io::BufWriter<crate::ioutil::CallbackWriter<F>>),
    Streaming(crate::ioutil::StreamingWriter<F>),
}
pub struct BufWriter<F: FnMut(Vec<u8>) -> Result<(), JsValue>> {
    writer: BufWriterState<F>,
}
impl<F: FnMut(Vec<u8>) -> Result<(), JsValue>> BufWriter<F> {
    pub fn new(capacity: usize, use_streaming: bool, on_chunk: F) -> Self {
        let writer = if use_streaming {
            BufWriterState::Streaming(crate::ioutil::StreamingWriter::new(capacity, on_chunk))
        } else {
            let writer = crate::ioutil::CallbackWriter::new(on_chunk);
            BufWriterState::Callback(io::BufWriter::with_capacity(capacity, writer))
        };
        Self { writer }
    }

    pub fn finish(&mut self) -> io::Result<()> {
        if let BufWriterState::Streaming(ref mut w) = self.writer {
            w.finish()?;
        }
        Ok(())
    }
}
impl<F: FnMut(Vec<u8>) -> Result<(), JsValue>> Write for BufWriter<F> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.writer {
            BufWriterState::Callback(ref mut w) => w.write(buf),
            BufWriterState::Streaming(ref mut w) => w.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.writer {
            BufWriterState::Callback(ref mut w) => w.flush(),
            BufWriterState::Streaming(ref mut w) => w.flush(),
        }
    }
}
