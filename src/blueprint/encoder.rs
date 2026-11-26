use std::io::{self, Write};

#[cfg(feature = "threads")]
use async_channel::{Receiver, Sender};
use base64::engine::GeneralPurpose;
use base64::engine::general_purpose::STANDARD;
use base64::write::EncoderWriter;
use flate2::Compression;
use flate2::write::ZlibEncoder;
#[cfg(feature = "threads")] use rayon::spawn;

use super::generator::BlueprintGenerator;
use crate::constants::FACTORIO_VERSION_PREFIX;
use crate::prelude::*;

#[cfg(feature = "threads")]
enum CompressionMode {
    Blueprint,
    RawCompressed,
}

#[cfg(feature = "threads")]
struct ChannelWriter {
    tx: Sender<Result<Vec<u8>, String>>,
}
#[cfg(feature = "threads")]
impl Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !buf.is_empty() {
            futures_lite::future::block_on(self.tx.send(Ok(buf.to_vec())))
                .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e))?;
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(feature = "threads")]
fn compress(
    mode: &CompressionMode, input: &Receiver<Vec<u8>>, output: &Sender<Result<Vec<u8>, String>>,
) -> io::Result<()> {
    let writer = ChannelWriter { tx: output.clone() };
    let writer = io::BufWriter::with_capacity(64 * 1024, writer);
    match *mode {
        CompressionMode::Blueprint => {
            let base64 = EncoderWriter::new(writer, &STANDARD);
            let mut encoder = ZlibEncoder::new(base64, Compression::best());
            while let Ok(chunk) = futures_lite::future::block_on(input.recv()) {
                encoder.write_all(&chunk)?;
            }
            encoder.finish()?.finish()
        },
        CompressionMode::RawCompressed => {
            let mut encoder = ZlibEncoder::new(writer, Compression::best());
            while let Ok(chunk) = futures_lite::future::block_on(input.recv()) {
                encoder.write_all(&chunk)?;
            }
            encoder.finish()
        },
    }?;
    Ok(())
}

#[cfg(feature = "threads")]
struct Compressor {
    input:  Option<Sender<Vec<u8>>>,
    output: Receiver<Result<Vec<u8>, String>>,
}
#[cfg(feature = "threads")]
impl Compressor {
    pub fn new(mode: CompressionMode) -> Self {
        let (input_tx, input_rx) = async_channel::bounded::<Vec<u8>>(4);
        let (output_tx, output_rx) = async_channel::unbounded::<Result<Vec<u8>, String>>();
        spawn(move || {
            if let Err(e) = compress(&mode, &input_rx, &output_tx) {
                _ = futures_lite::future::block_on(output_tx.send(Err(e.to_string())));
            }
        });
        Self { input: Some(input_tx), output: output_rx }
    }
    #[inline]
    pub async fn next_chunk(&self) -> Option<Result<Vec<u8>, String>> {
        self.output.recv().await.ok()
    }
    #[inline]
    pub fn try_next_chunk(&self) -> Option<Result<Vec<u8>, String>> {
        self.output.try_recv().ok()
    }
    #[inline]
    pub fn finish(&mut self) {
        self.input.take();
    }
}
#[cfg(feature = "threads")]
impl Write for Compressor {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !buf.is_empty() {
            let input = self
                .input
                .as_ref()
                .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "Compressor finished"))?;
            futures_lite::future::block_on(input.send(buf.to_vec()))
                .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e.to_string()))?;
        }
        Ok(buf.len())
    }
    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

enum OutputWriter<'a, W: Write> {
    Blueprint(Box<ZlibEncoder<EncoderWriter<'a, GeneralPurpose, W>>>),
    RawCompressed(ZlibEncoder<W>),
    #[cfg(feature = "threads")]
    ThreadedCompressed {
        compressor: Compressor,
        writer:     W,
    },
    Json(W),
}
impl<W: Write> io::Write for OutputWriter<'_, W> {
    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        match *self {
            Self::Blueprint(ref mut w) => w.flush(),
            Self::Json(ref mut w) => w.flush(),
            Self::RawCompressed(ref mut w) => w.flush(),
            #[cfg(feature = "threads")]
            Self::ThreadedCompressed { compressor: ref mut w, .. } => w.flush(),
        }
    }
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self {
            Self::Blueprint(ref mut w) => w.write(buf),
            Self::Json(ref mut w) => w.write(buf),
            Self::RawCompressed(ref mut w) => w.write(buf),
            #[cfg(feature = "threads")]
            Self::ThreadedCompressed { compressor: ref mut w, .. } => w.write(buf),
        }
    }
}
impl<W: Write> OutputWriter<'_, W> {
    #[cfg(feature = "threads")]
    pub fn new_threaded_compressed(mode: CompressionMode, writer: W) -> Self {
        Self::ThreadedCompressed { compressor: Compressor::new(mode), writer }
    }

    #[cfg(feature = "threads")]
    pub async fn drain(&mut self) -> Result<(), JsError> {
        if let Self::ThreadedCompressed { ref mut compressor, ref mut writer } = *self {
            while let Some(chunk) = compressor.next_chunk().await {
                writer.write_all(&chunk.map_js_err("Write error")?)?;
            }
        }
        Ok(())
    }
    #[cfg(feature = "threads")]
    pub fn drain_available(&mut self) -> Result<(), JsError> {
        if let Self::ThreadedCompressed { ref mut compressor, ref mut writer } = *self {
            while let Some(chunk) = compressor.try_next_chunk() {
                writer.write_all(&chunk.map_js_err("Write error")?)?;
            }
        }
        Ok(())
    }
    pub fn finish(self) -> Result<W, JsError> {
        Ok(match self {
            Self::Blueprint(z) => z.finish()?.finish()?,
            Self::Json(mut w) => {
                w.flush()?;
                w
            },
            Self::RawCompressed(z) => z.finish()?,
            #[cfg(feature = "threads")]
            Self::ThreadedCompressed { compressor, writer } => {
                drop(compressor);
                writer
            },
        })
    }
}

pub struct BlueprintEncoder<'a, W: Write> {
    generator: BlueprintGenerator<'a>,
    writer:    OutputWriter<'a, W>,
}
impl<'a, W: Write> BlueprintEncoder<'a, W> {
    pub fn new(
        generator: BlueprintGenerator<'a>, args: &BlueprintArgs, mut writer: W,
    ) -> Result<Self, JsError> {
        if args.output_format == OutputFormat::Blueprint {
            let prefix = FACTORIO_VERSION_PREFIX.as_bytes();
            writer.write_all(prefix)?;
        }
        let writer = match args.output_format {
            #[cfg(feature = "threads")]
            OutputFormat::Blueprint if args.use_threads => {
                OutputWriter::new_threaded_compressed(CompressionMode::Blueprint, writer)
            },
            OutputFormat::Blueprint => OutputWriter::Blueprint(Box::new(ZlibEncoder::new(
                EncoderWriter::new(writer, &STANDARD),
                Compression::best(),
            ))),
            #[cfg(feature = "threads")]
            OutputFormat::RawCompressed if args.use_threads => {
                OutputWriter::new_threaded_compressed(CompressionMode::RawCompressed, writer)
            },
            OutputFormat::RawCompressed => {
                OutputWriter::RawCompressed(ZlibEncoder::new(writer, Compression::best()))
            },
            _ => OutputWriter::Json(writer),
        };
        Ok(Self { generator, writer })
    }

    #[inline]
    pub async fn next(&mut self) -> Result<Option<()>, JsError> {
        if unlikely(self.generator.done()) {
            return Ok(None);
        }
        self.generator.next_chunk(&mut self.writer).await?;

        #[cfg(feature = "threads")]
        {
            self.writer.drain_available()?;
        };

        Ok(Some(()))
    }

    #[allow(clippy::unused_async, unused_mut, reason = "Only enabled with threads")]
    pub async fn finish(mut self) -> Result<W, JsError> {
        report_progress(1.0, "Finishing...");

        #[cfg(feature = "threads")]
        {
            if let OutputWriter::ThreadedCompressed { ref mut compressor, .. } = self.writer {
                compressor.finish();
            }
            self.writer.drain().await?;
        };

        self.writer.finish()
    }
}
