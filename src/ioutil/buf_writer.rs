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
        if let BufWriterState::Streaming(w) = &mut self.writer {
            w.finish()?;
        }
        Ok(())
    }
}
impl<F: FnMut(Vec<u8>) -> Result<(), JsValue>> Write for BufWriter<F> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        match &mut self.writer {
            BufWriterState::Callback(w) => w.write(data),
            BufWriterState::Streaming(w) => w.write(data),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.writer {
            BufWriterState::Callback(w) => w.flush(),
            BufWriterState::Streaming(w) => w.flush(),
        }
    }
}
