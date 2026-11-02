use std::io;

pub struct TrimmedWriter<'a, W: io::Write> {
    last: Option<u8>,
    is_first: bool,
    writer: &'a mut W,
}

impl<'a, W: io::Write> TrimmedWriter<'a, W> {
    pub fn new(writer: &'a mut W) -> Self {
        Self {
            last: None,
            is_first: true,
            writer,
        }
    }

    pub fn finish(mut self) -> io::Result<()> {
        self.last.take(); // Discard closing brace.
        self.writer.flush()
    }
}

impl<W: io::Write> io::Write for TrimmedWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !buf.is_empty() {
            let trimmed = if self.is_first { &buf[1..] } else { buf };
            if !trimmed.is_empty() {
                if let Some(prev_last) = self.last.replace(*trimmed.last().unwrap()) {
                    self.writer.write_all(&[prev_last])?;
                }
                self.writer.write_all(&trimmed[..trimmed.len() - 1])?;
            }
            self.is_first = false;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}
