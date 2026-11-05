use super::*;
use crate::WorkspaceId;
use crate::protocol::{
    StreamClientMessage, StreamError as DomainError, StreamFile as DomainFile, StreamMessage,
    StreamServerMessage, StreamTarget,
};

impl From<DomainError> for StreamError {
    fn from(error: DomainError) -> Self {
        match error {
            DomainError::ConnectionRefused => Self::ConnectionRefused,
            DomainError::NotFound => Self::NotFound,
            DomainError::AlreadyExists => Self::AlreadyExists,
            DomainError::NotAFile => Self::NotAFile,
            DomainError::InvalidPath => Self::InvalidPath,
            DomainError::InvalidPort => Self::InvalidPort,
            DomainError::WorkspaceGone => Self::WorkspaceGone,
            DomainError::TooManyStreams => Self::TooManyStreams,
            DomainError::Io => Self::Io,
            DomainError::Unsupported => Self::Unsupported,
            DomainError::Cancelled => Self::Cancelled,
            DomainError::Other => Self::Other,
        }
    }
}

fn decode_error(raw: i32) -> WireResult<DomainError> {
    Ok(match enum_or(raw, "stream error", StreamError::Other)? {
        StreamError::Unspecified => unreachable!("enum_or refuses 0"),
        StreamError::ConnectionRefused => DomainError::ConnectionRefused,
        StreamError::NotFound => DomainError::NotFound,
        StreamError::AlreadyExists => DomainError::AlreadyExists,
        StreamError::NotAFile => DomainError::NotAFile,
        StreamError::InvalidPath => DomainError::InvalidPath,
        StreamError::InvalidPort => DomainError::InvalidPort,
        StreamError::WorkspaceGone => DomainError::WorkspaceGone,
        StreamError::TooManyStreams => DomainError::TooManyStreams,
        StreamError::Io => DomainError::Io,
        StreamError::Unsupported => DomainError::Unsupported,
        StreamError::Cancelled => DomainError::Cancelled,
        StreamError::Other => DomainError::Other,
    })
}

fn encode_file(file: &DomainFile) -> StreamFile {
    StreamFile {
        workspace: file.workspace.as_u64(),
        path: file.path.as_str().to_owned(),
    }
}

fn decode_file(file: StreamFile) -> WireResult<DomainFile> {
    Ok(DomainFile {
        workspace: WorkspaceId::from_u64(file.workspace),
        path: relative_path(file.path)?,
    })
}

fn wrong_direction(message: &StreamMessage) -> String {
    format!("{message:?} cannot be sent in this direction")
}

impl TryFrom<&StreamClientMessage> for super::StreamClientMessage {
    type Error = String;

    fn try_from(message: &StreamClientMessage) -> Result<Self, String> {
        use stream_client_message::Message;
        let message = match &message.0 {
            StreamMessage::Open { stream, target } => {
                use stream_open::Target;
                let target = match target {
                    StreamTarget::Forward { port } => Target::ForwardPort(*port),
                    StreamTarget::Download(file) => Target::Download(encode_file(file)),
                    StreamTarget::Upload { file, replace } => Target::Upload(StreamUpload {
                        file: Some(encode_file(file)),
                        replace: *replace,
                    }),
                    StreamTarget::Unknown => return Err("an unknown target cannot be sent".into()),
                };
                Message::Open(StreamOpen {
                    stream: *stream,
                    target: Some(target),
                })
            }
            StreamMessage::Data { stream, data } => Message::Data(StreamData {
                stream: *stream,
                data: data.clone(),
            }),
            StreamMessage::Credit { stream, max_offset } => Message::Credit(StreamCredit {
                stream: *stream,
                max_offset: *max_offset,
            }),
            StreamMessage::Finish { stream } => Message::Finish(StreamFinish { stream: *stream }),
            StreamMessage::Reset {
                stream,
                error,
                detail,
            } => Message::Reset(StreamReset {
                stream: *stream,
                error: StreamError::from(*error).into(),
                detail: detail.clone(),
            }),
            StreamMessage::Ping => Message::Ping(Empty {}),
            other => return Err(wrong_direction(other)),
        };
        Ok(Self {
            message: Some(message),
        })
    }
}

impl TryFrom<super::StreamClientMessage> for StreamClientMessage {
    type Error = WireError;

    /// An unknown member becomes [`StreamMessage::Unknown`], and an `Open` whose target is
    /// unknown becomes [`StreamTarget::Unknown`]; neither closes the connection.
    fn try_from(message: super::StreamClientMessage) -> WireResult<Self> {
        use stream_client_message::Message;
        let Some(message) = message.message else {
            return Ok(Self(StreamMessage::Unknown));
        };
        Ok(Self(match message {
            Message::Open(open) => {
                use stream_open::Target;
                let target = match open.target {
                    None => StreamTarget::Unknown,
                    Some(Target::ForwardPort(port)) => StreamTarget::Forward { port },
                    Some(Target::Download(file)) => StreamTarget::Download(decode_file(file)?),
                    Some(Target::Upload(upload)) => StreamTarget::Upload {
                        file: decode_file(required(upload.file, "upload file")?)?,
                        replace: upload.replace,
                    },
                };
                StreamMessage::Open {
                    stream: open.stream,
                    target,
                }
            }
            Message::Data(data) => StreamMessage::Data {
                stream: data.stream,
                data: data.data,
            },
            Message::Credit(credit) => StreamMessage::Credit {
                stream: credit.stream,
                max_offset: credit.max_offset,
            },
            Message::Finish(finish) => StreamMessage::Finish {
                stream: finish.stream,
            },
            Message::Reset(reset) => StreamMessage::Reset {
                stream: reset.stream,
                error: decode_error(reset.error)?,
                detail: reset.detail,
            },
            Message::Ping(_) => StreamMessage::Ping,
        }))
    }
}

impl TryFrom<&StreamServerMessage> for super::StreamServerMessage {
    type Error = String;

    fn try_from(message: &StreamServerMessage) -> Result<Self, String> {
        use stream_server_message::Message;
        let message = match &message.0 {
            StreamMessage::Opened { stream, size } => Message::Opened(StreamOpened {
                stream: *stream,
                size: *size,
            }),
            StreamMessage::OpenFailed {
                stream,
                error,
                detail,
            } => Message::OpenFailed(StreamOpenFailed {
                stream: *stream,
                error: StreamError::from(*error).into(),
                detail: detail.clone(),
            }),
            StreamMessage::Data { stream, data } => Message::Data(StreamData {
                stream: *stream,
                data: data.clone(),
            }),
            StreamMessage::Credit { stream, max_offset } => Message::Credit(StreamCredit {
                stream: *stream,
                max_offset: *max_offset,
            }),
            StreamMessage::Finish { stream } => Message::Finish(StreamFinish { stream: *stream }),
            StreamMessage::Reset {
                stream,
                error,
                detail,
            } => Message::Reset(StreamReset {
                stream: *stream,
                error: StreamError::from(*error).into(),
                detail: detail.clone(),
            }),
            StreamMessage::Pong => Message::Pong(Empty {}),
            other => return Err(wrong_direction(other)),
        };
        Ok(Self {
            message: Some(message),
        })
    }
}

impl TryFrom<super::StreamServerMessage> for StreamServerMessage {
    type Error = WireError;

    /// An unknown member becomes [`StreamMessage::Unknown`] and is ignored.
    fn try_from(message: super::StreamServerMessage) -> WireResult<Self> {
        use stream_server_message::Message;
        let Some(message) = message.message else {
            return Ok(Self(StreamMessage::Unknown));
        };
        Ok(Self(match message {
            Message::Opened(opened) => StreamMessage::Opened {
                stream: opened.stream,
                size: opened.size,
            },
            Message::OpenFailed(failed) => StreamMessage::OpenFailed {
                stream: failed.stream,
                error: decode_error(failed.error)?,
                detail: failed.detail,
            },
            Message::Data(data) => StreamMessage::Data {
                stream: data.stream,
                data: data.data,
            },
            Message::Credit(credit) => StreamMessage::Credit {
                stream: credit.stream,
                max_offset: credit.max_offset,
            },
            Message::Finish(finish) => StreamMessage::Finish {
                stream: finish.stream,
            },
            Message::Reset(reset) => StreamMessage::Reset {
                stream: reset.stream,
                error: decode_error(reset.error)?,
                detail: reset.detail,
            },
            Message::Pong(_) => StreamMessage::Pong,
        }))
    }
}
