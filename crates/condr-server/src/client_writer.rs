use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use condr_core::protocol::{MAX_FRAME_PREFIX, MAX_FRAME_SIZE, MAX_FRAMED_BOOTSTRAP_BYTES};

const MAX_RELIABLE_QUEUE_ITEMS: usize = 512;
// Room for the largest framed Bootstrap the protocol allows plus one ordinary frame, so a
// legitimate recovery Bootstrap can never itself trip the lag bound.
const MAX_RELIABLE_QUEUE_BYTES: usize =
    MAX_FRAMED_BOOTSTRAP_BYTES + MAX_FRAME_SIZE + MAX_FRAME_PREFIX;

/// Why a reliable send was refused. `Lagged` means the backlog was replaced by the lag
/// notice and the client will re-Bootstrap; the connection itself is still alive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReliableSendError {
    Lagged,
    Disconnected,
}

#[derive(Debug)]
pub(crate) struct ClientWriter {
    queue: Arc<ClientWriterQueue>,
}

pub(crate) struct ClientWriterReceiver {
    queue: Arc<ClientWriterQueue>,
}

#[derive(Debug)]
pub(crate) enum ClientWriteItem {
    Reliable(Vec<u8>),
    ReliableBatch(Vec<Vec<u8>>),
    ClosingReliable {
        data: Vec<u8>,
        delivered: SyncSender<bool>,
    },
    Render {
        data: Vec<u8>,
        slot_drained: bool,
    },
}

pub(crate) struct ClientWriteReceipt {
    delivered: Receiver<bool>,
}

#[derive(Debug)]
struct ClientWriterQueue {
    state: Mutex<ClientWriterQueueState>,
    ready: Condvar,
}

#[derive(Debug)]
struct ClientWriterQueueState {
    reliable: VecDeque<ClientWriteItem>,
    reliable_bytes: usize,
    lag_notice: Option<Vec<u8>>,
    lagged: bool,
    clipboard: Option<Vec<u8>>,
    // One visual generation stays shared so a Bootstrap can discard its unsent frames.
    render: Option<VecDeque<Vec<u8>>>,
    senders: usize,
    writer_alive: bool,
}

impl ClientWriter {
    #[cfg(test)]
    pub(crate) fn channel() -> (Self, ClientWriterReceiver) {
        Self::channel_inner(None)
    }

    pub(crate) fn channel_with_lag_notice(lag_notice: Vec<u8>) -> (Self, ClientWriterReceiver) {
        Self::channel_inner(Some(lag_notice))
    }

    fn channel_inner(lag_notice: Option<Vec<u8>>) -> (Self, ClientWriterReceiver) {
        let queue = Arc::new(ClientWriterQueue {
            state: Mutex::new(ClientWriterQueueState {
                reliable: VecDeque::new(),
                reliable_bytes: 0,
                lag_notice,
                lagged: false,
                clipboard: None,
                render: None,
                senders: 1,
                writer_alive: true,
            }),
            ready: Condvar::new(),
        });
        (
            Self {
                queue: Arc::clone(&queue),
            },
            ClientWriterReceiver { queue },
        )
    }

    pub(crate) fn send_reliable(&self, data: Vec<u8>) -> Result<(), ReliableSendError> {
        let mut state = self.queue.lock_state();
        state.refusal()?;
        if let Err(error) = state.reserve_reliable(data.len()) {
            self.queue.ready.notify_all();
            return Err(error);
        }
        state.reliable.push_back(ClientWriteItem::Reliable(data));
        self.queue.ready.notify_one();
        Ok(())
    }

    pub(crate) fn send_reliable_batch(
        &self,
        frames: Vec<Vec<u8>>,
    ) -> Result<(), ReliableSendError> {
        let mut state = self.queue.lock_state();
        state.refusal()?;
        let bytes = frames
            .iter()
            .fold(0usize, |total, frame| total.saturating_add(frame.len()));
        if let Err(error) = state.reserve_reliable(bytes) {
            self.queue.ready.notify_all();
            return Err(error);
        }
        state
            .reliable
            .push_back(ClientWriteItem::ReliableBatch(frames));
        self.queue.ready.notify_one();
        Ok(())
    }

    /// Queues the latest clipboard state without allowing OSC 52 bursts to grow this queue.
    pub(crate) fn send_clipboard(&self, data: Vec<u8>) -> Result<(), ReliableSendError> {
        let mut state = self.queue.lock_state();
        state.refusal()?;
        state.clipboard = Some(data);
        self.queue.ready.notify_one();
        Ok(())
    }

    /// Queues the final reliable frame after earlier reliable work and closes this sender.
    pub(crate) fn send_closing_reliable(
        &self,
        data: Vec<u8>,
    ) -> Result<ClientWriteReceipt, ReliableSendError> {
        let mut state = self.queue.lock_state();
        state.refusal()?;
        if let Err(error) = state.reserve_reliable(data.len()) {
            self.queue.ready.notify_all();
            return Err(error);
        }
        let (delivered, receipt) = sync_channel(1);
        state
            .reliable
            .push_back(ClientWriteItem::ClosingReliable { data, delivered });
        state.clipboard = None;
        state.render = None;
        state.writer_alive = false;
        self.queue.ready.notify_one();
        Ok(ClientWriteReceipt { delivered: receipt })
    }

    pub(crate) fn try_send_render(
        &self,
        frames: Vec<Vec<u8>>,
    ) -> Result<(), TrySendError<Vec<Vec<u8>>>> {
        assert!(
            !frames.is_empty(),
            "render slot requires at least one frame"
        );
        let mut state = self.queue.lock_state();
        if !state.writer_alive || state.lagged {
            return Err(TrySendError::Disconnected(frames));
        }
        if state.render.is_some() {
            return Err(TrySendError::Full(frames));
        }
        state.render = Some(frames.into());
        self.queue.ready.notify_one();
        Ok(())
    }

    pub(crate) fn clear_render(&self) {
        self.queue.lock_state().render = None;
    }

    /// Reopens a lagged queue for the recovery Bootstrap. The reliable queue is kept: after an
    /// overflow it holds only the lag notice, which must still reach the client (the Bootstrap
    /// may answer a visual-gap request sent before the lag, and the client only drops its
    /// subscription when it reads the notice). Droppable slots are cleared.
    pub(crate) fn resume_after_lag(&self) {
        let mut state = self.queue.lock_state();
        if !state.lagged {
            return;
        }
        state.clipboard = None;
        state.render = None;
        state.lagged = false;
    }
}

impl ClientWriteReceipt {
    pub(crate) fn wait(self, timeout: Duration) -> bool {
        matches!(self.delivered.recv_timeout(timeout), Ok(true))
    }
}

impl Clone for ClientWriter {
    fn clone(&self) -> Self {
        let mut state = self.queue.lock_state();
        state.senders = state.senders.saturating_add(1);
        drop(state);
        Self {
            queue: Arc::clone(&self.queue),
        }
    }
}

impl Drop for ClientWriter {
    fn drop(&mut self) {
        let mut state = self.queue.lock_state();
        state.senders = state.senders.saturating_sub(1);
        self.queue.ready.notify_one();
    }
}

impl ClientWriterReceiver {
    pub(crate) fn recv(&self) -> Option<ClientWriteItem> {
        let mut state = self.queue.lock_state();
        loop {
            if let Some(item) = state.reliable.pop_front() {
                state.reliable_bytes = state
                    .reliable_bytes
                    .saturating_sub(reliable_item_bytes(&item));
                return Some(item);
            }
            if let Some(data) = state.clipboard.take() {
                return Some(ClientWriteItem::Reliable(data));
            }
            if let Some(frames) = state.render.as_mut() {
                let data = frames.pop_front().expect("render slot must not be empty");
                let slot_drained = frames.is_empty();
                if slot_drained {
                    state.render = None;
                }
                return Some(ClientWriteItem::Render { data, slot_drained });
            }
            if state.senders == 0 || !state.writer_alive {
                return None;
            }
            state = self
                .queue
                .ready
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    pub(crate) fn close(&self) {
        let mut state = self.queue.lock_state();
        state.writer_alive = false;
        state.reliable.clear();
        state.reliable_bytes = 0;
        state.lagged = false;
        state.clipboard = None;
        state.render = None;
        self.queue.ready.notify_all();
    }
}

impl ClientWriterQueueState {
    fn refusal(&self) -> Result<(), ReliableSendError> {
        if !self.writer_alive {
            Err(ReliableSendError::Disconnected)
        } else if self.lagged {
            Err(ReliableSendError::Lagged)
        } else {
            Ok(())
        }
    }

    fn reserve_reliable(&mut self, bytes: usize) -> Result<(), ReliableSendError> {
        let next_bytes = self.reliable_bytes.checked_add(bytes);
        if self.reliable.len() >= MAX_RELIABLE_QUEUE_ITEMS
            || next_bytes.is_none_or(|bytes| bytes > MAX_RELIABLE_QUEUE_BYTES)
        {
            self.reliable.clear();
            self.reliable_bytes = 0;
            self.clipboard = None;
            self.render = None;
            return if let Some(notice) = self.lag_notice.clone() {
                self.reliable_bytes = notice.len();
                self.reliable.push_back(ClientWriteItem::Reliable(notice));
                self.lagged = true;
                Err(ReliableSendError::Lagged)
            } else {
                self.writer_alive = false;
                Err(ReliableSendError::Disconnected)
            };
        }
        self.reliable_bytes = next_bytes.expect("checked above");
        Ok(())
    }
}

fn reliable_item_bytes(item: &ClientWriteItem) -> usize {
    match item {
        ClientWriteItem::Reliable(data) | ClientWriteItem::ClosingReliable { data, .. } => {
            data.len()
        }
        ClientWriteItem::ReliableBatch(frames) => frames
            .iter()
            .fold(0usize, |total, frame| total.saturating_add(frame.len())),
        ClientWriteItem::Render { .. } => 0,
    }
}

impl ClientWriterQueue {
    fn lock_state(&self) -> std::sync::MutexGuard<'_, ClientWriterQueueState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl PartialEq for ClientWriteItem {
        fn eq(&self, other: &Self) -> bool {
            match (self, other) {
                (Self::Reliable(left), Self::Reliable(right)) => left == right,
                (Self::ReliableBatch(left), Self::ReliableBatch(right)) => left == right,
                (
                    Self::Render {
                        data: left_data,
                        slot_drained: left_drained,
                    },
                    Self::Render {
                        data: right_data,
                        slot_drained: right_drained,
                    },
                ) => left_data == right_data && left_drained == right_drained,
                _ => false,
            }
        }
    }

    impl Eq for ClientWriteItem {}

    #[test]
    fn reliable_messages_take_priority_over_the_single_render_slot() {
        let (writer, receiver) = ClientWriter::channel();
        writer.try_send_render(vec![b"render".to_vec()]).unwrap();
        writer.send_reliable(b"control".to_vec()).unwrap();

        assert_eq!(
            receiver.recv(),
            Some(ClientWriteItem::Reliable(b"control".to_vec()))
        );
        assert_eq!(
            receiver.recv(),
            Some(ClientWriteItem::Render {
                data: b"render".to_vec(),
                slot_drained: true,
            })
        );
    }

    #[test]
    fn clipboard_slot_keeps_only_the_latest_value() {
        let (writer, receiver) = ClientWriter::channel();
        writer.try_send_render(vec![b"render".to_vec()]).unwrap();
        writer.send_clipboard(b"old".to_vec()).unwrap();
        writer.send_clipboard(b"latest".to_vec()).unwrap();

        assert_eq!(
            receiver.recv(),
            Some(ClientWriteItem::Reliable(b"latest".to_vec()))
        );
        assert_eq!(
            receiver.recv(),
            Some(ClientWriteItem::Render {
                data: b"render".to_vec(),
                slot_drained: true,
            })
        );
    }

    #[test]
    fn reliable_messages_preempt_render_between_bounded_frames() {
        let (writer, receiver) = ClientWriter::channel();
        writer
            .try_send_render(vec![b"first".to_vec(), b"second".to_vec()])
            .unwrap();

        assert_eq!(
            receiver.recv(),
            Some(ClientWriteItem::Render {
                data: b"first".to_vec(),
                slot_drained: false,
            })
        );
        writer.send_reliable(b"control".to_vec()).unwrap();
        assert_eq!(
            receiver.recv(),
            Some(ClientWriteItem::Reliable(b"control".to_vec()))
        );
        assert_eq!(
            receiver.recv(),
            Some(ClientWriteItem::Render {
                data: b"second".to_vec(),
                slot_drained: true,
            })
        );
    }

    #[test]
    fn render_slot_is_bounded_and_can_be_cleared_for_a_bootstrap() {
        let (writer, receiver) = ClientWriter::channel();
        writer.try_send_render(vec![b"old".to_vec()]).unwrap();
        assert!(matches!(
            writer.try_send_render(vec![b"new".to_vec()]),
            Err(TrySendError::Full(_))
        ));

        writer.clear_render();
        writer.try_send_render(vec![b"fresh".to_vec()]).unwrap();
        assert_eq!(
            receiver.recv(),
            Some(ClientWriteItem::Render {
                data: b"fresh".to_vec(),
                slot_drained: true,
            })
        );
    }

    #[test]
    fn bootstrap_stays_after_an_in_flight_render_and_before_a_fresh_render() {
        let (writer, receiver) = ClientWriter::channel();
        writer
            .try_send_render(vec![b"in-flight".to_vec(), b"stale-tail".to_vec()])
            .unwrap();
        let in_flight = receiver.recv();

        writer.clear_render();
        writer.send_reliable(b"bootstrap".to_vec()).unwrap();
        writer.try_send_render(vec![b"fresh".to_vec()]).unwrap();

        assert_eq!(
            in_flight,
            Some(ClientWriteItem::Render {
                data: b"in-flight".to_vec(),
                slot_drained: false,
            })
        );
        assert_eq!(
            receiver.recv(),
            Some(ClientWriteItem::Reliable(b"bootstrap".to_vec()))
        );
        assert_eq!(
            receiver.recv(),
            Some(ClientWriteItem::Render {
                data: b"fresh".to_vec(),
                slot_drained: true,
            })
        );
    }

    #[test]
    fn reliable_batch_is_one_ordered_queue_item() {
        let (writer, receiver) = ClientWriter::channel();
        writer
            .send_reliable_batch(vec![b"header".to_vec(), b"chunk".to_vec()])
            .unwrap();
        writer.send_reliable(b"event".to_vec()).unwrap();

        assert_eq!(
            receiver.recv(),
            Some(ClientWriteItem::ReliableBatch(vec![
                b"header".to_vec(),
                b"chunk".to_vec()
            ]))
        );
        assert_eq!(
            receiver.recv(),
            Some(ClientWriteItem::Reliable(b"event".to_vec()))
        );
    }

    #[test]
    fn closing_reliable_preserves_order_returns_a_receipt_and_ends_the_queue() {
        let (writer, receiver) = ClientWriter::channel();
        writer.send_reliable(b"queued".to_vec()).unwrap();
        let receipt = writer.send_closing_reliable(b"stopping".to_vec()).unwrap();

        assert_eq!(
            receiver.recv(),
            Some(ClientWriteItem::Reliable(b"queued".to_vec()))
        );
        let Some(ClientWriteItem::ClosingReliable { data, delivered }) = receiver.recv() else {
            panic!("closing lifecycle response should follow queued reliable work");
        };
        assert_eq!(data, b"stopping");
        delivered.send(true).unwrap();
        assert!(receipt.wait(Duration::from_millis(10)));
        assert!(receiver.recv().is_none());
        assert!(writer.send_reliable(b"late".to_vec()).is_err());
    }

    #[test]
    fn reliable_queue_disconnects_a_writer_that_falls_behind() {
        let (writer, receiver) = ClientWriter::channel();
        for _ in 0..MAX_RELIABLE_QUEUE_ITEMS {
            writer.send_reliable(vec![0]).unwrap();
        }

        assert_eq!(
            writer.send_reliable(vec![0]),
            Err(ReliableSendError::Disconnected)
        );
        assert!(receiver.recv().is_none());
        assert_eq!(
            writer.send_reliable(vec![0]),
            Err(ReliableSendError::Disconnected)
        );
    }

    #[test]
    fn lag_notice_replaces_overflow_and_bootstrap_reopens_the_queue() {
        let (writer, receiver) = ClientWriter::channel_with_lag_notice(b"resync".to_vec());
        for _ in 0..MAX_RELIABLE_QUEUE_ITEMS {
            writer.send_reliable(vec![0]).unwrap();
        }

        assert_eq!(
            writer.send_reliable(vec![0]),
            Err(ReliableSendError::Lagged)
        );
        assert_eq!(
            receiver.recv(),
            Some(ClientWriteItem::Reliable(b"resync".to_vec()))
        );
        assert_eq!(
            writer.send_reliable(vec![0]),
            Err(ReliableSendError::Lagged)
        );

        writer.resume_after_lag();
        writer.send_reliable(b"bootstrap".to_vec()).unwrap();
        assert_eq!(
            receiver.recv(),
            Some(ClientWriteItem::Reliable(b"bootstrap".to_vec()))
        );
    }

    #[test]
    fn largest_allowed_bootstrap_fits_while_an_ordinary_backlog_still_lags() {
        use condr_core::protocol::{
            BOOTSTRAP_BATCH_FRAME_OVERHEAD, MAX_BOOTSTRAP_BATCHES, MAX_BOOTSTRAP_TOTAL_SIZE,
        };
        let batches = MAX_BOOTSTRAP_BATCHES as usize;
        let payload_per_batch = MAX_BOOTSTRAP_TOTAL_SIZE / batches;
        let mut frames = Vec::with_capacity(batches + 1);
        frames.push(vec![0; MAX_FRAME_SIZE + MAX_FRAME_PREFIX]);
        frames.extend(
            (0..batches).map(|_| vec![0; BOOTSTRAP_BATCH_FRAME_OVERHEAD + payload_per_batch]),
        );
        assert_eq!(
            frames.iter().map(Vec::len).sum::<usize>(),
            MAX_FRAMED_BOOTSTRAP_BYTES
        );

        let (writer, receiver) = ClientWriter::channel_with_lag_notice(b"resync".to_vec());
        writer.send_reliable_batch(frames).unwrap();
        writer.send_reliable(vec![0; MAX_FRAME_SIZE + MAX_FRAME_PREFIX]).unwrap();
        assert_eq!(
            writer.send_reliable(vec![0; MAX_FRAME_SIZE + MAX_FRAME_PREFIX]),
            Err(ReliableSendError::Lagged)
        );
        assert_eq!(
            receiver.recv(),
            Some(ClientWriteItem::Reliable(b"resync".to_vec()))
        );
    }

    #[test]
    fn resuming_before_the_lag_notice_is_read_keeps_it_ahead_of_the_bootstrap() {
        let (writer, receiver) = ClientWriter::channel_with_lag_notice(b"resync".to_vec());
        for _ in 0..MAX_RELIABLE_QUEUE_ITEMS {
            writer.send_reliable(vec![0]).unwrap();
        }
        assert_eq!(
            writer.send_reliable(vec![0]),
            Err(ReliableSendError::Lagged)
        );

        // A SnapshotRequest sent before the lag reopens the queue while the notice is unread.
        writer.resume_after_lag();
        writer.send_reliable(b"bootstrap".to_vec()).unwrap();
        assert_eq!(
            receiver.recv(),
            Some(ClientWriteItem::Reliable(b"resync".to_vec()))
        );
        assert_eq!(
            receiver.recv(),
            Some(ClientWriteItem::Reliable(b"bootstrap".to_vec()))
        );
    }
}
