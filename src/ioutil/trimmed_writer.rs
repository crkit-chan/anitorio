use std::io;

pub struct TrimmedWriter<'a, W: io::Write> {
    writer: &'a mut W,
    first: bool,
    pending: Option<u8>,
}

impl<'a, W: io::Write> TrimmedWriter<'a, W> {
    pub fn new(writer: &'a mut W) -> Self {
        Self {
            writer,
            first: true,
            pending: None,
        }
    }
    pub fn finish(mut self) -> io::Result<()> {
        self.pending.take(); // Discard the final byte (closing brace).
        Ok(())
    }
}

impl<W: io::Write> io::Write for TrimmedWriter<'_, W> {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        if input.is_empty() {
            return Ok(0);
        }

        let input_len = input.len();
        let buf = if self.first { &input[1..] } else { input };
        self.first = false;

        if !buf.is_empty() {
            if let Some(last) = self.pending.replace(*buf.last().unwrap_or(&0)) {
                self.writer.write_all(&[last])?;
            }
            if buf.len() > 1 {
                self.writer.write_all(&buf[..buf.len() - 1])?;
            }
        }

        Ok(input_len)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}
