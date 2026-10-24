use crate::{Result, error, resources};
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    ops::Range,
};
use yy_encoding::{Decoder, Encoding, EscapeMode};

#[derive(Clone, Debug)]
pub struct Record {
    pub data: Vec<u8>,
    pub fields: Vec<Range<usize>>,
}
impl Record {
    pub fn size(&self) -> usize {
        self.data.capacity() + self.fields.capacity() * std::mem::size_of::<Range<usize>>()
    }
    pub fn value(&self, position: usize, length: usize, csv: bool) -> Result<&[u8]> {
        if csv {
            self.fields
                .get(position - 1)
                .map(|r| &self.data[r.clone()])
                .ok_or_else(|| error(format!("CSV column {position} does not exist")))
        } else {
            let start = position - 1;
            let end = start
                .checked_add(length)
                .ok_or_else(|| error("field offset overflow"))?;
            self.data.get(start..end).ok_or_else(|| {
                error(format!(
                    "field {position},{length} exceeds record length {}",
                    self.data.len()
                ))
            })
        }
    }
}
struct Decoded<R> {
    reader: R,
    decoder: Decoder,
    buffer: Vec<u8>,
    offset: usize,
    eof: bool,
    first: bool,
}
impl<R: Read> Read for Decoded<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        while self.offset == self.buffer.len() && !self.eof {
            resources::check().map_err(io::Error::other)?;
            let mut block = [0u8; 8192];
            let n = self.reader.read(&mut block)?;
            self.eof = n == 0;
            self.offset = 0;
            self.buffer.clear();
            self.decoder.decode(&block[..n], &mut self.buffer, self.eof);
            if self.decoder.stats().invalid > 0 {
                return Err(io::Error::other("invalid encoded CSV text"));
            }
            if self.first && !self.buffer.is_empty() {
                self.first = false;
                if self.buffer.starts_with(&[0xef, 0xbb, 0xbf]) {
                    self.offset = 3;
                }
            }
        }
        let n = out.len().min(self.buffer.len() - self.offset);
        out[..n].copy_from_slice(&self.buffer[self.offset..self.offset + n]);
        self.offset += n;
        Ok(n)
    }
}
pub enum Reader {
    Fixed {
        reader: BufReader<std::fs::File>,
        length: usize,
        max: usize,
    },
    Csv {
        reader: BufReader<Box<dyn Read>>,
        max: usize,
        allow_lf: bool,
        columns: Option<usize>,
    },
}
impl Reader {
    pub fn open(
        path: &std::path::Path,
        length: Option<usize>,
        enc: Encoding,
        max: usize,
        allow_lf: bool,
    ) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        if let Some(length) = length {
            if length == 0 || length > max {
                return Err(error("fixed record length must be within --max-record"));
            }
            Ok(Self::Fixed {
                reader: BufReader::with_capacity(65536, file),
                length,
                max,
            })
        } else {
            let d = Decoded {
                reader: file,
                decoder: enc.new_decoder(false),
                buffer: Vec::new(),
                offset: 0,
                eof: false,
                first: true,
            };
            Ok(Self::Csv {
                reader: BufReader::new(Box::new(d)),
                max,
                allow_lf,
                columns: None,
            })
        }
    }
    pub fn next_record(&mut self) -> Result<Option<Record>> {
        resources::check()?;
        match self {
            Self::Fixed {
                reader,
                length,
                max,
            } => {
                let _ = max;
                let mut data = vec![0; *length];
                let mut count = 0;
                while count < data.len() {
                    let n = reader.read(&mut data[count..])?;
                    if n == 0 {
                        if count == 0 {
                            return Ok(None);
                        }
                        return Err(error("incomplete final fixed-length record"));
                    }
                    count += n;
                }
                Ok(Some(Record {
                    data,
                    fields: Vec::new(),
                }))
            }
            Self::Csv {
                reader,
                max,
                allow_lf,
                columns,
            } => {
                let mut record = Record {
                    data: Vec::new(),
                    fields: Vec::new(),
                };
                let mut start = 0;
                let mut state = 0u8;
                let mut seen = false;
                let mut raw = 0usize;
                // 0 field start, 1 unquoted, 2 quoted, 3 closing quote.
                loop {
                    let next = byte(reader)?;
                    if next.is_none() {
                        if !seen {
                            return Ok(None);
                        }
                        if state == 2 {
                            return Err(error("unterminated CSV quoted field"));
                        }
                        record.fields.push(start..record.data.len());
                        break;
                    }
                    seen = true;
                    raw += 1;
                    if raw > *max || record.size() > max.saturating_mul(20) {
                        return Err(error("CSV record exceeds --max-record"));
                    }
                    if raw & 8191 == 0 {
                        resources::check()?;
                    }
                    let b = next.unwrap();
                    if b < 32 && !matches!(b, b'\r' | b'\n') {
                        return Err(error("CSV contains a control byte disallowed by RFC 4180"));
                    }
                    if state == 2 {
                        if b == b'"' {
                            state = 3;
                        } else {
                            record.data.push(b);
                        }
                        continue;
                    }
                    if state == 3 && b == b'"' {
                        record.data.push(b'"');
                        state = 2;
                        continue;
                    }
                    match b {
                        b'"' if state == 0 => state = 2,
                        b'"' => return Err(error("quote in an unquoted CSV field")),
                        b',' => {
                            record.fields.push(start..record.data.len());
                            start = record.data.len();
                            state = 0;
                        }
                        b'\r' | b'\n' => {
                            if b == b'\r' {
                                if byte(reader)? != Some(b'\n') {
                                    return Err(error("CSV CR must be followed by LF"));
                                }
                            } else if !*allow_lf {
                                return Err(error(
                                    "CSV requires CRLF; use --allow-lf for Unix input",
                                ));
                            }
                            record.fields.push(start..record.data.len());
                            break;
                        }
                        _ if state == 3 => {
                            return Err(error("unexpected text after CSV closing quote"));
                        }
                        _ => {
                            record.data.push(b);
                            state = 1;
                        }
                    }
                }
                std::str::from_utf8(&record.data)?;
                if let Some(n) = columns {
                    if *n != record.fields.len() {
                        return Err(error("CSV records have different column counts"));
                    }
                } else {
                    *columns = Some(record.fields.len());
                }
                Ok(Some(record))
            }
        }
    }
}
fn byte(reader: &mut impl BufRead) -> io::Result<Option<u8>> {
    let buffer = reader.fill_buf()?;
    let b = buffer.first().copied();
    if b.is_some() {
        reader.consume(1);
    }
    Ok(b)
}

pub struct Output<W> {
    writer: W,
    encoder: yy_encoding::Encoder,
    csv: bool,
}
impl<W: Write> Output<W> {
    pub fn new(writer: W, encoding: Encoding, csv: bool) -> Self {
        Self {
            writer,
            encoder: encoding.new_encoder(EscapeMode::Reject),
            csv,
        }
    }
    fn encoded(&mut self, bytes: &[u8], last: bool) -> Result<()> {
        let mut out = Vec::new();
        let mut invalid = false;
        self.encoder
            .encode(bytes, &mut out, last, &mut |_| invalid = true);
        if invalid {
            return Err(error(
                "CSV output cannot be represented in the selected encoding",
            ));
        }
        self.writer.write_all(&out)?;
        Ok(())
    }
    pub fn record(&mut self, r: &Record) -> Result<()> {
        if !self.csv {
            self.writer.write_all(&r.data)?;
            return Ok(());
        }
        for (i, field) in r.fields.iter().enumerate() {
            if i > 0 {
                self.encoded(b",", false)?;
            }
            let value = &r.data[field.clone()];
            let quote = value
                .iter()
                .any(|b| matches!(b, b',' | b'"' | b'\r' | b'\n'))
                || r.fields.len() == 1 && value.is_empty();
            if quote {
                self.encoded(b"\"", false)?;
                for part in value.split_inclusive(|b| *b == b'"') {
                    self.encoded(part, false)?;
                    if part.last() == Some(&b'"') {
                        self.encoded(b"\"", false)?;
                    }
                }
                self.encoded(b"\"", false)?;
            } else {
                self.encoded(value, false)?;
            }
        }
        self.encoded(b"\r\n", false)
    }
    pub fn finish(mut self) -> Result<()> {
        if self.csv {
            self.encoded(b"", true)?;
        }
        self.writer.flush()?;
        Ok(())
    }
}

pub fn replace_fields(record: &mut Record, updates: &[(usize, Vec<u8>)]) -> Result<()> {
    let mut data = Vec::new();
    let mut fields = Vec::with_capacity(record.fields.len());
    for (i, field) in record.fields.iter().enumerate() {
        let start = data.len();
        data.extend_from_slice(
            updates
                .iter()
                .find(|(p, _)| *p == i + 1)
                .map(|(_, v)| v.as_slice())
                .unwrap_or(&record.data[field.clone()]),
        );
        fields.push(start..data.len());
    }
    if updates.iter().any(|(p, _)| *p == 0 || *p > fields.len()) {
        return Err(error("SUM CSV column does not exist"));
    }
    record.data = data;
    record.fields = fields;
    Ok(())
}
