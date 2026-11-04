use super::generator::BlueprintGenerator;
use crate::constants::FACTORIO_VERSION_PREFIX;
use crate::prelude::*;
use async_channel::{Receiver, Sender};
use base64::{
    engine::{GeneralPurpose, general_purpose::STANDARD},
    write::EncoderWriter,
};
use flate2::{Compression, write::ZlibEncoder};
use rayon::spawn;
use std::io::{self, Write};

enum CompressionMode {
    Blueprint,
    RawCompressed,
}

struct ChannelWriter {
    tx: Sender<Result<Vec<u8>, String>>,
}
impl Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        futures_lite::future::block_on(self.tx.send(Ok(buf.to_vec())))
            .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e.to_string()))?;

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn compress(
    mode: &CompressionMode,
    input: &Receiver<Vec<u8>>,
    output: &Sender<Result<Vec<u8>, String>>,
) -> io::Result<()> {
    let writer = ChannelWriter { tx: output.clone() };

    match mode {
        CompressionMode::Blueprint => {
            let base64 = EncoderWriter::new(writer, &STANDARD);
            let mut encoder = ZlibEncoder::new(base64, Compression::best());

            while let Ok(chunk) = futures_lite::future::block_on(input.recv()) {
                encoder.write_all(&chunk)?;
            }

            encoder.finish()?.finish()?;
        }

        CompressionMode::RawCompressed => {
            let mut encoder = ZlibEncoder::new(writer, Compression::best());

            while let Ok(chunk) = futures_lite::future::block_on(input.recv()) {
                encoder.write_all(&chunk)?;
            }

            encoder.finish()?;
        }
    }

    Ok(())
}

struct Compressor {
    input: Option<Sender<Vec<u8>>>,
    output: Receiver<Result<Vec<u8>, String>>,
}
impl Compressor {
    pub fn new(mode: CompressionMode) -> Self {
        let (input_tx, input_rx) = async_channel::bounded::<Vec<u8>>(4);
        let (output_tx, output_rx) = async_channel::unbounded::<Result<Vec<u8>, String>>();

        spawn(move || {
            if let Err(e) = compress(&mode, &input_rx, &output_tx) {
                let _ = futures_lite::future::block_on(output_tx.send(Err(e.to_string())));
            }
        });

        Self {
            input: Some(input_tx),
            output: output_rx,
        }
    }

    pub async fn next_chunk(&self) -> Option<Result<Vec<u8>, String>> {
        self.output.recv().await.ok()
    }

    pub fn try_next_chunk(&self) -> Option<Result<Vec<u8>, String>> {
        self.output.try_recv().ok()
    }

    pub fn finish(&mut self) {
        self.input.take();
    }
}
impl Write for Compressor {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        let input = self
            .input
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "Compressor finished"))?;

        futures_lite::future::block_on(input.send(buf.to_vec()))
            .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e.to_string()))?;

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

enum OutputWriter<'a, W: Write> {
    Blueprint(Box<ZlibEncoder<EncoderWriter<'a, GeneralPurpose, W>>>),
    RawCompressed(ZlibEncoder<W>),
    ThreadedCompressed { compressor: Compressor, writer: W },
    Json(W),
}
impl<W: Write> io::Write for OutputWriter<'_, W> {
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Blueprint(w) => w.flush(),
            Self::Json(w) => w.flush(),
            Self::RawCompressed(w) => w.flush(),
            Self::ThreadedCompressed { compressor: w, .. } => w.flush(),
        }
    }
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Blueprint(w) => w.write(buf),
            Self::Json(w) => w.write(buf),
            Self::RawCompressed(w) => w.write(buf),
            Self::ThreadedCompressed { compressor: w, .. } => w.write(buf),
        }
    }
}
impl<W: Write> OutputWriter<'_, W> {
    pub async fn drain(&mut self) -> Result<(), JsError> {
        if let Self::ThreadedCompressed { compressor, writer } = self {
            while let Some(chunk) = compressor.next_chunk().await {
                writer.write_all(&chunk.map_js_err("Write error")?)?;
            }
        }

        Ok(())
    }

    pub fn drain_available(&mut self) -> Result<(), JsError> {
        if let Self::ThreadedCompressed { compressor, writer } = self {
            while let Some(chunk) = compressor.try_next_chunk() {
                writer.write_all(&chunk.map_js_err("Write error")?)?;
            }
        }

        Ok(())
    }

    pub fn finish(self) -> Result<W, JsError> {
        match self {
            Self::Blueprint(zlib) => {
                let mut b64 = zlib.finish()?;
                b64.finish().map_js_err("Could not finish b64 writer")
            }
            Self::Json(mut w) => {
                w.flush()?;
                Ok(w)
            }
            Self::RawCompressed(zlib) => zlib.finish().map_js_err("Could not finish zlib writer"),
            Self::ThreadedCompressed { compressor, writer } => {
                drop(compressor);
                Ok(writer)
            }
        }
    }
}

pub struct BlueprintEncoder<'a, W: Write> {
    generator: BlueprintGenerator<'a>,
    writer: Option<OutputWriter<'a, W>>,
}
impl<'a, W: Write> BlueprintEncoder<'a, W> {
    pub fn new(
        generator: BlueprintGenerator<'a>,
        args: &BlueprintArgs,
        mut writer: W,
    ) -> Result<Self, JsError> {
        if args.output_format == OutputFormat::Blueprint {
            let prefix = FACTORIO_VERSION_PREFIX.as_bytes();
            writer.write_all(prefix)?;
        }
        let writer = Some(match args.output_format {
            OutputFormat::Blueprint => {
                if args.use_threads {
                    OutputWriter::ThreadedCompressed {
                        compressor: Compressor::new(CompressionMode::Blueprint),
                        writer,
                    }
                } else {
                    OutputWriter::Blueprint(Box::new(ZlibEncoder::new(
                        EncoderWriter::new(writer, &STANDARD),
                        Compression::best(),
                    )))
                }
            }
            OutputFormat::RawCompressed => {
                if args.use_threads {
                    OutputWriter::ThreadedCompressed {
                        compressor: Compressor::new(CompressionMode::RawCompressed),
                        writer,
                    }
                } else {
                    OutputWriter::RawCompressed(ZlibEncoder::new(writer, Compression::best()))
                }
            }
            _ => OutputWriter::Json(writer),
        });

        Ok(Self { generator, writer })
    }

    pub async fn next(&mut self) -> Result<Option<()>, JsError> {
        if self.generator.done() {
            return Ok(None);
        }

        let writer = self.writer.as_mut().unwrap();

        self.generator.next_chunk(writer).await?;
        writer.drain_available()?;

        Ok(Some(()))
    }

    pub async fn finish(mut self) -> Result<W, JsError> {
        report_progress(1.0, "Finishing...");

        let mut writer = self.writer.take().unwrap();

        if let OutputWriter::ThreadedCompressed { compressor, .. } = &mut writer {
            compressor.finish();
        }

        writer.drain().await?;
        writer.finish()
    }
}
