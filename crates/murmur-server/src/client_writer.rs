use std::collections::VecDeque;
use std::sync::mpsc::{SendError, TrySendError};
use std::sync::{Arc, Condvar, Mutex};

#[derive(Debug)]
pub(crate) struct ClientWriter {
    queue: Arc<ClientWriterQueue>,
}

pub(crate) struct ClientWriterReceiver {
    queue: Arc<ClientWriterQueue>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ClientWriteItem {
    Reliable(Vec<u8>),
    ReliableBatch(Vec<Vec<u8>>),
    Render { data: Vec<u8>, slot_drained: bool },
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
}
