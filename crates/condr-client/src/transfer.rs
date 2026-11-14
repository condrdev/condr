//! File transfers (ADR 0061): one Stream each, a single regular file inside a Workspace.
//! A download is written beside its destination and placed only once complete; an upload
//! is complete when the Server, having placed it, finishes its side. The GUI and the CLI
//! both use these.

use crate::forward::StreamOpener;
use crate::streams::{Stream, StreamFailure};
use condr_core::TerminalImageKey;
use condr_core::protocol::{MAX_STREAM_CHUNK, StreamError, StreamFile, StreamTarget};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

/// Cancels a transfer from another thread, at once even while it waits on the Device:
/// cancelling resets its Stream, which wakes whatever it was blocked on.
#[derive(Clone, Default)]
pub struct TransferCancel(Arc<Mutex<CancelState>>);

#[derive(Default)]
struct CancelState {
    cancelled: bool,
    stream: Option<Arc<Stream>>,
}

impl TransferCancel {
    pub fn cancel(&self) {
        let mut state = self.lock();
        state.cancelled = true;
        if let Some(stream) = &state.stream {
            stream.reset(StreamError::Cancelled, "");
        }
    }

    /// No transfer holds this token's Stream any more: whatever it wrote beside its
    /// target is gone, so quitting may go on.
    pub fn settled(&self) -> bool {
        self.lock().stream.is_none()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CancelState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Lends the transfer's Stream to [`Self::cancel`] until the guard drops.
    fn attach(&self, stream: &Arc<Stream>) -> Result<Attached<'_>, StreamFailure> {
        let mut state = self.lock();
        if state.cancelled {
            stream.reset(StreamError::Cancelled, "");
            return Err(failure(StreamError::Cancelled, ""));
        }
        state.stream = Some(Arc::clone(stream));
        Ok(Attached(self))
    }
}

struct Attached<'a>(&'a TransferCancel);

impl Drop for Attached<'_> {
    fn drop(&mut self) {
        self.0.lock().stream = None;
    }
}

fn failure(error: StreamError, detail: impl Into<String>) -> StreamFailure {
    StreamFailure {
        error,
        detail: detail.into(),
    }
}

fn io_failure(error: &io::Error) -> StreamFailure {
    let kind = match error.kind() {
        io::ErrorKind::NotFound => StreamError::NotFound,
        io::ErrorKind::AlreadyExists => StreamError::AlreadyExists,
        _ => StreamError::Io,
    };
    failure(kind, error.to_string())
}

/// Downloads `file` to `destination`; `progress` hears bytes written so far and the size
/// the Server named when it opened. An existing destination fails without `replace`.
pub fn download(
    opener: &dyn StreamOpener,
    file: StreamFile,
    destination: &Path,
    replace: bool,
    cancel: &TransferCancel,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<u64, StreamFailure> {
    if !replace && fs::symlink_metadata(destination).is_ok() {
        return Err(failure(
            StreamError::AlreadyExists,
            format!("{} already exists", destination.display()),
        ));
    }
    let stream = Arc::new(opener.open(StreamTarget::Download(file))?);
    let _attached = cancel.attach(&stream)?;
    let size = stream.opened()?;
    let (temp, mut output) =
        condr_core::create_transfer_temp(destination, "download").map_err(|error| {
            stream.reset(StreamError::Io, error.to_string());
            io_failure(&error)
        })?;
    // Removes the temporary file on every way out but success.
    struct Temp<'a>(&'a Path, bool);
    impl Drop for Temp<'_> {
        fn drop(&mut self) {
            if !self.1 {
                let _ = fs::remove_file(self.0);
            }
        }
    }
    let mut guard = Temp(&temp, false);
    let mut done = 0;
    while let Some(data) = stream.read()? {
        if let Err(error) = output.write_all(&data) {
            stream.reset(StreamError::Io, error.to_string());
            return Err(io_failure(&error));
        }
        done += data.len() as u64;
        progress(done, size);
    }
    let placed = output
        .sync_all()
        .and_then(|()| condr_core::place_transfer(&temp, destination, replace));
    if let Err(error) = placed {
        stream.reset(StreamError::Io, error.to_string());
        return Err(io_failure(&error));
    }
    guard.1 = true;
    stream.finish();
    Ok(done)
}

/// The most bytes one terminal image's PNG may take; the Server's own limits keep a real
/// one well below it.
const MAX_IMAGE_PNG_BYTES: usize = 128 * 1024 * 1024;

/// A terminal image's PNG by its key (ADR 0065).
pub fn fetch_image(
    opener: &dyn StreamOpener,
    key: TerminalImageKey,
) -> Result<Vec<u8>, StreamFailure> {
    let stream = opener.open(StreamTarget::Image(key))?;
    let size = stream.opened()?;
    let mut png = Vec::with_capacity(
        size.and_then(|size| usize::try_from(size).ok())
            .unwrap_or(0)
            .min(MAX_IMAGE_PNG_BYTES),
    );
    while let Some(data) = stream.read()? {
        if png.len() + data.len() > MAX_IMAGE_PNG_BYTES {
            stream.reset(StreamError::Other, "image too large");
            return Err(failure(StreamError::Other, "image too large"));
        }
        png.extend_from_slice(&data);
    }
    stream.finish();
    Ok(png)
}

/// Uploads `source` as `file`; `progress` hears bytes handed to the Stream, at most a
/// window ahead of what the Server wrote, and the file's size. Complete when the Server
/// answers with its own `Finish`, the file in place.
pub fn upload(
    opener: &dyn StreamOpener,
    source: &Path,
    file: StreamFile,
    replace: bool,
    cancel: &TransferCancel,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<u64, StreamFailure> {
    // Looked at before opening: Windows refuses to open a directory at all.
    let meta = fs::metadata(source).map_err(|error| io_failure(&error))?;
    if !meta.is_file() {
        return Err(failure(
            StreamError::NotAFile,
            "only files can be uploaded; archive a folder first",
        ));
    }
    let mut input = File::open(source).map_err(|error| io_failure(&error))?;
    let size = meta.len();
    let stream = Arc::new(opener.open(StreamTarget::Upload { file, replace })?);
    let _attached = cancel.attach(&stream)?;
    let mut buffer = vec![0; MAX_STREAM_CHUNK];
    let mut done = 0;
    loop {
        let read = match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                stream.reset(StreamError::Io, error.to_string());
                return Err(io_failure(&error));
            }
        };
        stream.write(&buffer[..read])?;
        done += read as u64;
        progress(done, Some(size));
    }
    stream.finish();
    while stream.read()?.is_some() {}
    Ok(done)
}
