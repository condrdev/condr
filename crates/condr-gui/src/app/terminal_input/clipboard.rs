use super::*;

impl Condr {
    pub(in crate::app) fn copy_terminal_selection(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        _cx: &mut Context<Self>,
    ) -> bool {
        let Some((selection, server_tracked)) = self.effective_selection(key, pane_id) else {
            return false;
        };
        let Some(columns) = self
            .terminal(key, pane_id)
            .map(|terminal| terminal.view.size.columns)
        else {
            return false;
        };
        if selection.selected_cell_range(columns).is_none() {
            return false;
        }

        // The Server's own selection also covers rows scrolled out of the viewport.
        let selection = (!server_tracked).then_some(selection);
        self.terminal_command(key, pane_id, TerminalCommand::Copy { selection })
    }

    pub(in crate::app) fn paste_into_terminal(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        self.clear_selection(cx);
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.terminal_command(key, pane_id, TerminalCommand::Paste(text));
        }
    }

    /// Returns whether a remote image-paste gesture was consumed (ADR 0012).
    pub(super) fn paste_clipboard_image(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let Some((format, bytes)) = item.entries().iter().find_map(|entry| match entry {
            gpui_kit::ClipboardEntry::Image(image) => {
                clipboard_image_format(image.format).map(|format| (format, &image.bytes))
            }
            _ => None,
        }) else {
            return false;
        };
        let Some(connection) = self.connection_mut(key) else {
            return false;
        };
        let (Some(server_id), Some(session_id)) = (connection.server_id, connection.session_id)
        else {
            return false;
        };
        if !connection.can_mutate()
            || connection
                .terminals
                .get(&pane_id)
                .is_none_or(|terminal| terminal.exited)
        {
            return false;
        }
        if bytes.len() > condr_core::protocol::MAX_CLIPBOARD_IMAGE_BYTES {
            connection.error = Some("Image exceeds 16 MiB".into());
            cx.notify();
            return true;
        }
        // The Pane is fixed here; a focus change while the upload is in flight must not
        // retarget it, which the message's own pane_id guarantees.
        connection.pasting_images.insert(pane_id);
        connection.send(ClientMessage::PasteImage {
            server_id,
            session_id,
            pane_id,
            format,
            bytes: bytes.clone(),
        });
        self.clear_selection(cx);
        // The header's "Pasting image…" must show now, not on the next terminal frame.
        cx.notify();
        true
    }
}

/// `Alt+V` alone: the image-paste gesture the common Agent CLIs use (ADR 0012).
pub(in crate::app) fn is_image_paste_gesture(stroke: &Keystroke) -> bool {
    let modifiers = stroke.modifiers;
    stroke.key == "v"
        && modifiers.alt
        && !modifiers.control
        && !modifiers.platform
        && !modifiers.shift
        && !modifiers.function
}

/// The wire format for a clipboard image, or `None` for one the Server does not stage.
pub(in crate::app) fn clipboard_image_format(
    format: gpui_kit::ImageFormat,
) -> Option<condr_core::protocol::ClipboardImageFormat> {
    use condr_core::protocol::ClipboardImageFormat as Wire;
    Some(match format {
        gpui_kit::ImageFormat::Png => Wire::Png,
        gpui_kit::ImageFormat::Jpeg => Wire::Jpeg,
        gpui_kit::ImageFormat::Gif => Wire::Gif,
        gpui_kit::ImageFormat::Webp => Wire::Webp,
        gpui_kit::ImageFormat::Bmp => Wire::Bmp,
        _ => return None,
    })
}

/// Prepare images on the connection's writer thread, preserving input order without
/// decoding on the UI thread. BMP clipboard data needs PNG for Agent image readers.
pub(in crate::app) fn prepare_clipboard_image(
    format: &mut condr_core::protocol::ClipboardImageFormat,
    bytes: &mut Vec<u8>,
) -> Result<(), String> {
    use condr_core::protocol::{ClipboardImageFormat, MAX_CLIPBOARD_IMAGE_BYTES};

    if bytes.len() > MAX_CLIPBOARD_IMAGE_BYTES {
        return Err("Image exceeds 16 MiB".into());
    }
    if *format != ClipboardImageFormat::Bmp {
        return Ok(());
    }
    let mut reader = image::ImageReader::with_format(
        std::io::Cursor::new(bytes.as_slice()),
        image::ImageFormat::Bmp,
    );
    // A compressed BMP must not request unbounded decoded pixel storage.
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let png = reader
        .decode()
        .and_then(|decoded| {
            let mut output = std::io::Cursor::new(Vec::new());
            decoded.write_to(&mut output, image::ImageFormat::Png)?;
            Ok(output.into_inner())
        })
        .map_err(|error| format!("Could not convert clipboard image to PNG: {error}"))?;
    if png.len() > MAX_CLIPBOARD_IMAGE_BYTES {
        return Err("Converted image exceeds 16 MiB".into());
    }
    *format = ClipboardImageFormat::Png;
    *bytes = png;
    Ok(())
}
