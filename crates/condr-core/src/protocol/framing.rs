use super::*;

pub fn encode_bootstrap_record(record: &BootstrapRecord) -> Result<Vec<u8>, String> {
    encode_chunked_value(record, "Bootstrap record")
}

pub fn decode_bootstrap_record(payload: &[u8]) -> Result<BootstrapRecord, String> {
    decode_chunked_value(payload, "Bootstrap record")
}

pub fn encode_pane_terminal_frame(frame: &PaneTerminalFrame) -> Result<Vec<u8>, String> {
    encode_chunked_value(frame, "Pane terminal frame")
}

pub fn decode_pane_terminal_frame(payload: &[u8]) -> Result<PaneTerminalFrame, String> {
    decode_chunked_value(payload, "Pane terminal frame")
}

fn encode_chunked_value<T>(value: &T, description: &str) -> Result<Vec<u8>, String>
where
    T: Serialize,
{
    let payload = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_CHUNKED_RECORD_SIZE as u64)
        .serialize(value)
        .map_err(|error| format!("{description} cannot be encoded: {error}"))?;
    if payload.len() > MAX_CHUNKED_RECORD_SIZE {
        return Err(format!(
            "{description} is {} bytes; limit is {MAX_CHUNKED_RECORD_SIZE} bytes",
            payload.len()
        ));
    }
    Ok(payload)
}

fn decode_chunked_value<T>(payload: &[u8], description: &str) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    if payload.len() > MAX_CHUNKED_RECORD_SIZE {
        return Err(format!(
            "{description} is {} bytes; limit is {MAX_CHUNKED_RECORD_SIZE} bytes",
            payload.len()
        ));
    }
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_CHUNKED_RECORD_SIZE as u64)
        .reject_trailing_bytes()
        .deserialize(payload)
        .map_err(|error| format!("{description} cannot be decoded: {error}"))
}

#[derive(Debug)]
pub enum FramingError {
    Io(io::Error),
    UnexpectedEof,
    Oversized { claimed: usize, max: usize },
    Codec(String),
}

impl fmt::Display for FramingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::UnexpectedEof => formatter.write_str("unexpected end of stream"),
            Self::Oversized { claimed, max } => {
                write!(formatter, "frame size {claimed} exceeds maximum {max}")
            }
            Self::Codec(error) => write!(formatter, "protocol codec error: {error}"),
        }
    }
}

impl std::error::Error for FramingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for FramingError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn write_message<W, M>(writer: &mut W, message: &M) -> Result<(), FramingError>
where
    W: Write,
    M: Serialize,
{
    write_message_with_limit(writer, message, MAX_FRAME_SIZE)
}

/// Writes a Client message under the allowance its kind is entitled to.
pub fn write_client_message<W: Write>(
    writer: &mut W,
    message: &ClientMessage,
) -> Result<(), FramingError> {
    write_message_with_limit(writer, message, message.frame_limit())
}

pub fn write_message_with_limit<W, M>(
    writer: &mut W,
    message: &M,
    limit: usize,
) -> Result<(), FramingError>
where
    W: Write,
    M: Serialize,
{
    let payload = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(limit as u64)
        .serialize(message)
        .map_err(|error| FramingError::Codec(error.to_string()))?;
    if payload.len() > limit {
        return Err(FramingError::Oversized {
            claimed: payload.len(),
            max: limit,
        });
    }
    let length = u32::try_from(payload.len()).map_err(|_| {
        FramingError::Codec("payload length does not fit in the frame prefix".into())
    })?;
    writer.write_all(&length.to_le_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

pub fn read_message<R, M>(reader: &mut R) -> Result<M, FramingError>
where
    R: Read,
    M: for<'de> Deserialize<'de>,
{
    read_message_with_limit(reader, MAX_FRAME_SIZE).map(|(message, _)| message)
}

/// Reads one frame of up to `limit` bytes, returning the message and the frame's payload
/// size, so a reader that admits large frames can still hold most kinds to the normal one.
pub fn read_message_with_limit<R, M>(
    reader: &mut R,
    limit: usize,
) -> Result<(M, usize), FramingError>
where
    R: Read,
    M: for<'de> Deserialize<'de>,
{
    let mut prefix = [0; 4];
    read_exact_or_eof(reader, &mut prefix)?;
    let claimed = u32::from_le_bytes(prefix) as usize;
    if claimed > limit {
        return Err(FramingError::Oversized {
            claimed,
            max: limit,
        });
    }

    let mut payload = vec![0; claimed];
    read_exact_or_eof(reader, &mut payload)?;
    let message = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(limit as u64)
        .reject_trailing_bytes()
        .deserialize(&payload)
        .map_err(|error| FramingError::Codec(error.to_string()))?;
    Ok((message, claimed))
}

fn read_exact_or_eof<R: Read>(reader: &mut R, buffer: &mut [u8]) -> Result<(), FramingError> {
    reader.read_exact(buffer).map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            FramingError::UnexpectedEof
        } else {
            FramingError::Io(error)
        }
    })
}
