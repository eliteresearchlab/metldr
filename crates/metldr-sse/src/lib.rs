#![doc = include_str!("../README.md")]

type Result<T> = std::result::Result<T, String>;

#[cfg(test)]
mod checks;

pub struct Sse {
    provider: &'static str,
    line: Vec<u8>,
    data: String,
    event: String,
    started: bool,
    after_cr: bool,
    bound: usize,
}

impl Sse {
    pub fn new(provider: &'static str) -> Self {
        Self::with_bound(provider, 131_072)
    }

    pub fn with_bound(provider: &'static str, bound: usize) -> Self {
        Self {
            provider,
            line: Vec::new(),
            data: String::new(),
            event: String::new(),
            started: false,
            after_cr: false,
            bound,
        }
    }

    /// Feeds one byte; prefer [`Sse::next`] for whole chunks.
    pub fn byte(&mut self, byte: u8) -> Result<Option<(String, String)>> {
        let mut input = &[byte][..];
        self.next(&mut input)
    }

    /// Consumes `input` up to and including the line that completes the next event, and returns
    /// that event; returns `None` once `input` is used up. Line ends are found with a SIMD scan,
    /// and line bytes are copied in bulk.
    pub fn next(&mut self, input: &mut &[u8]) -> Result<Option<(String, String)>> {
        while let Some((&first, rest)) = input.split_first() {
            if std::mem::take(&mut self.after_cr) && first == b'\n' {
                *input = rest;
                continue;
            }
            let Some(at) = memchr::memchr2(b'\r', b'\n', input) else {
                self.extend(input)?;
                *input = &[];
                break;
            };
            self.extend(&input[..at])?;
            self.after_cr = input[at] == b'\r';
            *input = &input[at + 1..];
            if let Some(frame) = self.line()? {
                return Ok(Some(frame));
            }
        }
        Ok(None)
    }

    fn extend(&mut self, bytes: &[u8]) -> Result<()> {
        if self.line.len().saturating_add(bytes.len()) > self.bound {
            return Err(format!("{} SSE line exceeds bound", self.provider));
        }
        self.line.extend_from_slice(bytes);
        Ok(())
    }

    fn line(&mut self) -> Result<Option<(String, String)>> {
        let line = std::str::from_utf8(&self.line)
            .map_err(|_| format!("{} SSE is not UTF8", self.provider))?;
        let line = if self.started {
            line
        } else {
            line.strip_prefix('\u{feff}').unwrap_or(line)
        };
        self.started = true;
        let frame = if line.is_empty() {
            let event = std::mem::take(&mut self.event);
            if self.data.is_empty() {
                None
            } else {
                self.data.pop();
                Some((event, std::mem::take(&mut self.data)))
            }
        } else {
            let (field, value) = line.split_once(':').unwrap_or((line, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "data" => {
                    if self.data.len().saturating_add(value.len() + 1) > self.bound {
                        return Err(format!("{} SSE event exceeds bound", self.provider));
                    }
                    self.data.push_str(value);
                    self.data.push('\n');
                }
                "event" => self.event = value.into(),
                _ => {}
            }
            None
        };
        self.line.clear();
        Ok(frame)
    }

    pub fn finish(self) -> Result<()> {
        if self.line.is_empty() && self.data.is_empty() && self.event.is_empty() {
            Ok(())
        } else {
            Err(format!("{} SSE ended within an event", self.provider))
        }
    }
}
