use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, SendError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

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
    // One visual generation stays shared so a Bootstrap can discard its unsent frames.
    render: Option<VecDeque<Vec<u8>>>,
    senders: usize,
    writer_alive: bool,
}

impl ClientWriter {
    pub(crate) fn channel() -> (Self, ClientWriterReceiver) {
        let queue = Arc::new(ClientWriterQueue {
            state: Mutex::new(ClientWriterQueueState {
                reliable: VecDeque::new(),
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

    pub(crate) fn send_reliable(&self, data: Vec<u8>) -> Result<(), SendError<Vec<u8>>> {
        let mut state = self.queue.lock_state();
        if !state.writer_alive {
            return Err(SendError(data));
        }
        state.reliable.push_back(ClientWriteItem::Reliable(data));
        self.queue.ready.notify_one();
        Ok(())
    }

    pub(crate) fn send_reliable_batch(
        &self,
        frames: Vec<Vec<u8>>,
    ) -> Result<(), SendError<Vec<Vec<u8>>>> {
        let mut state = self.queue.lock_state();
        if !state.writer_alive {
            return Err(SendError(frames));
        }
        state
            .reliable
            .push_back(ClientWriteItem::ReliableBatch(frames));
        self.queue.ready.notify_one();
        Ok(())
    }

    /// Queues the final reliable frame after earlier reliable work and closes this sender.
    pub(crate) fn send_closing_reliable(
        &self,
        data: Vec<u8>,
    ) -> Result<ClientWriteReceipt, SendError<Vec<u8>>> {
        let mut state = self.queue.lock_state();
        if !state.writer_alive {
            return Err(SendError(data));
        }
        let (delivered, receipt) = sync_channel(1);
        state
            .reliable
            .push_back(ClientWriteItem::ClosingReliable { data, delivered });
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
        if !state.writer_alive {
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
                return Some(item);
            }
            if state.render.is_some() {
                let (data, slot_drained) = {
                    let frames = state.render.as_mut().expect("render slot must exist");
                    let data = frames.pop_front().expect("render slot must not be empty");
                    (data, frames.is_empty())
                };
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
        state.render = None;
        self.queue.ready.notify_all();
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
}
