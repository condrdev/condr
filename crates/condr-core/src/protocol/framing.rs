use super::*;
use prost::Message as _;

/// A message that travels as one frame: the handshake types, `ClientMessage` and
/// `ServerMessage`. Sealed; the protobuf types behind it stay inside the crate.
pub trait WireMessage: Sized + private::Sealed {}

mod private {
    use super::*;

    pub trait Sealed: Sized {
        /// The body of one frame, at most `limit` bytes, behind its varint length.
        fn frame(&self, limit: usize) -> Result<Vec<u8>, FramingError>;
        fn decode(body: &[u8]) -> Result<Self, FramingError>;
        /// Refuses a body before it is decoded, where its size alone says it cannot be
        /// what it claims.
        fn admit(body: &[u8]) -> Result<(), FramingError>;
    }
}

macro_rules! wire_message {
    ($domain:ty, $wire:ty, encode: $encode:ident, admit: $admit:ident) => {
        impl WireMessage for $domain {}

        impl private::Sealed for $domain {
            fn frame(&self, limit: usize) -> Result<Vec<u8>, FramingError> {
                let wire: $wire = ($encode)(self)?;
                frame_body(&wire, limit)
            }

            fn decode(body: &[u8]) -> Result<Self, FramingError> {
                let wire = <$wire>::decode(body)
                    .map_err(|error| FramingError::Codec(error.to_string()))?;
                <$domain>::try_from(wire).map_err(|error| FramingError::Codec(error.to_string()))
            }

            fn admit(body: &[u8]) -> Result<(), FramingError> {
                $admit(body)
            }
        }
    };
}

fn admit_any(_body: &[u8]) -> Result<(), FramingError> {
    Ok(())
}

fn infallible<'a, D, W: From<&'a D>>(value: &'a D) -> Result<W, FramingError> {
    Ok(W::from(value))
}

fn fallible<'a, D, W: TryFrom<&'a D, Error = String>>(value: &'a D) -> Result<W, FramingError> {
    W::try_from(value).map_err(FramingError::Codec)
}

wire_message!(ClientHandshake, pb::ClientHandshake, encode: infallible, admit: admit_any);
wire_message!(Welcome, pb::Welcome, encode: infallible, admit: admit_any);
wire_message!(ClientMessage, pb::ClientMessage, encode: fallible, admit: admit_client_frame);

/// Only `PasteImage` may use the image allowance, and a frame over the normal limit is
/// checked to be exactly that one field before it is decoded: decoding a large frame of
/// something else could cost many times its size (ADR 0028).
fn admit_client_frame(body: &[u8]) -> Result<(), FramingError> {
    const PASTE_IMAGE: u8 = 9 << 3 | 2;
    if body.len() <= MAX_FRAME_SIZE {
        return Ok(());
    }
    let refused = || FramingError::Oversized {
        claimed: body.len(),
        max: MAX_FRAME_SIZE,
    };
    let (&tag, rest) = body.split_first().ok_or_else(refused)?;
    let length = prost::decode_length_delimiter(rest).map_err(|_| refused())?;
    if tag != PASTE_IMAGE || 1 + prost::length_delimiter_len(length) + length != body.len() {
        return Err(refused());
    }
    Ok(())
}
wire_message!(ServerMessage, pb::ServerMessage, encode: fallible, admit: admit_any);

/// The longest varint length a frame carries: ten bytes encode any `u64`.
pub const MAX_FRAME_PREFIX: usize = 10;

fn frame_body<M: prost::Message>(message: &M, limit: usize) -> Result<Vec<u8>, FramingError> {
    let length = message.encoded_len();
    if length > limit {
        return Err(FramingError::Oversized {
            claimed: length,
            max: limit,
        });
    }
    let mut frame = Vec::with_capacity(prost::length_delimiter_len(length) + length);
    message
        .encode_length_delimited(&mut frame)
        .expect("a Vec grows to fit");
    Ok(frame)
}

pub fn encode_bootstrap_record(record: &BootstrapRecord) -> Result<Vec<u8>, String> {
    encode_chunked_value(&pb::BootstrapRecord::from(record), "Bootstrap record")
}

/// `None` is a record kind this build does not know; the Bootstrap skips it (ADR 0028).
pub fn decode_bootstrap_record(payload: &[u8]) -> Result<Option<BootstrapRecord>, String> {
    let record: pb::BootstrapRecord = decode_chunked_value(payload, "Bootstrap record")?;
    pb::decode_bootstrap_record(record)
        .map_err(|error| format!("Bootstrap record cannot be decoded: {error}"))
}

pub fn encode_pane_terminal_frame(frame: &PaneTerminalFrame) -> Result<Vec<u8>, String> {
    encode_chunked_value(&pb::PaneTerminalFrame::from(frame), "Pane terminal frame")
}

/// `Ok(None)` is a frame from a newer protocol: the Client resynchronizes.
pub fn decode_pane_terminal_frame(payload: &[u8]) -> Result<Option<PaneTerminalFrame>, String> {
    let frame: pb::PaneTerminalFrame = decode_chunked_value(payload, "Pane terminal frame")?;
    match PaneTerminalFrame::try_from(frame) {
        Ok(frame) => Ok(Some(frame)),
        Err(pb::WireError::Unknown) => Ok(None),
        Err(error) => Err(format!("Pane terminal frame cannot be decoded: {error}")),
    }
}

fn encode_chunked_value<M: prost::Message>(
    value: &M,
    description: &str,
) -> Result<Vec<u8>, String> {
    let length = value.encoded_len();
    if length > MAX_CHUNKED_RECORD_SIZE {
        return Err(format!(
            "{description} is {length} bytes; limit is {MAX_CHUNKED_RECORD_SIZE} bytes"
        ));
    }
    Ok(value.encode_to_vec())
}

fn decode_chunked_value<M: prost::Message + Default>(
    payload: &[u8],
    description: &str,
) -> Result<M, String> {
    if payload.len() > MAX_CHUNKED_RECORD_SIZE {
        return Err(format!(
            "{description} is {} bytes; limit is {MAX_CHUNKED_RECORD_SIZE} bytes",
            payload.len()
        ));
    }
    M::decode(payload).map_err(|error| format!("{description} cannot be decoded: {error}"))
}

/// A terminal frame converted to its wire form once, so the Server can size a batch by it
/// and then encode it without converting again (ADR 0004, 0028).
pub struct WirePaneFrame(pb::PaneTerminalFrame);

impl WirePaneFrame {
    pub fn new(frame: &PaneTerminalFrame) -> Self {
        Self(frame.into())
    }

    /// The bytes this frame adds to a `TerminalFrame` batch, field tag and length included.
    pub fn batch_len(&self) -> usize {
        prost::encoding::message::encoded_len(3, &self.0)
    }

    /// The frame alone, as a chunked record.
    pub fn encode_record(&self) -> Result<Vec<u8>, String> {
        encode_chunked_value(&self.0, "Pane terminal frame")
    }
}

/// One `TerminalFrame` message from frames already converted, as a complete frame.
pub fn frame_terminal_batch(
    server_id: ServerId,
    session_id: SessionId,
    panes: Vec<WirePaneFrame>,
) -> Result<Vec<u8>, FramingError> {
    let message = pb::ServerMessage {
        message: Some(pb::server_message::Message::TerminalFrame(
            pb::TerminalFrameBatch {
                server_id: server_id.0,
                session_id: session_id.0,
                panes: panes.into_iter().map(|pane| pane.0).collect(),
            },
        )),
    };
    frame_body(&message, MAX_FRAME_SIZE)
}

/// The bytes an empty `TerminalFrame` batch for this Session takes, before any frame.
pub fn terminal_batch_overhead(server_id: ServerId, session_id: SessionId) -> usize {
    let message = pb::ServerMessage {
        message: Some(pb::server_message::Message::TerminalFrame(
            pb::TerminalFrameBatch {
                server_id: server_id.0,
                session_id: session_id.0,
                panes: Vec::new(),
            },
        )),
    };
    // Growing the batch can widen the outer lengths; count them at their widest.
    message.encoded_len() + 2 * MAX_FRAME_PREFIX
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

/// One complete frame, varint length included, for a writer queue to send as is.
pub fn encode_frame<M: WireMessage>(message: &M, limit: usize) -> Result<Vec<u8>, FramingError> {
    message.frame(limit)
}

pub fn write_message<W, M>(writer: &mut W, message: &M) -> Result<(), FramingError>
where
    W: Write,
    M: WireMessage,
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
    M: WireMessage,
{
    let frame = message.frame(limit)?;
    writer.write_all(&frame)?;
    writer.flush()?;
    Ok(())
}

pub fn read_message<R, M>(reader: &mut R) -> Result<M, FramingError>
where
    R: Read,
    M: WireMessage,
{
    read_message_with_limit(reader, MAX_FRAME_SIZE).map(|(message, _)| message)
}

/// Reads one frame of up to `limit` bytes, returning the message and the frame's body
/// size, so a reader that admits large frames can still hold most kinds to the normal one.
///
/// The varint length is read a byte at a time, so nothing past this frame is consumed
/// and the stream can be handed on (a Tunnel) right after; a length above `limit` is
/// refused before anything is allocated for it.
pub fn read_message_with_limit<R, M>(
    reader: &mut R,
    limit: usize,
) -> Result<(M, usize), FramingError>
where
    R: Read,
    M: WireMessage,
{
    let claimed = read_frame_length(reader)?;
    if claimed > limit as u64 {
        return Err(FramingError::Oversized {
            claimed: usize::try_from(claimed).unwrap_or(usize::MAX),
            max: limit,
        });
    }
    let claimed = claimed as usize;
    let mut body = vec![0; claimed];
    read_exact_or_eof(reader, &mut body)?;
    M::admit(&body)?;
    Ok((M::decode(&body)?, claimed))
}

fn read_frame_length<R: Read>(reader: &mut R) -> Result<u64, FramingError> {
    let mut prefix = [0u8; MAX_FRAME_PREFIX];
    for index in 0..MAX_FRAME_PREFIX {
        read_exact_or_eof(reader, &mut prefix[index..=index])?;
        if prefix[index] & 0x80 == 0 {
            return prost::decode_length_delimiter(&prefix[..=index])
                .map(|length| length as u64)
                .map_err(|error| FramingError::Codec(error.to_string()));
        }
    }
    Err(FramingError::Codec("frame length is not a varint".into()))
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
