use super::*;

#[derive(Debug)]
pub struct BootstrapAssembler {
    header: BootstrapHeader,
    next_batch_index: u32,
    next_record_index: u32,
    next_chunk_index: u32,
    current_chunk_count: Option<u32>,
    current_payload: Vec<u8>,
    total_payload_size: usize,
    terminals: Vec<PaneTerminalSnapshot>,
    agents: Vec<PaneAgentSnapshot>,
    workspace_git: Vec<WorkspaceGitSnapshot>,
    zoomed_panes: Vec<PaneId>,
    terminal_ids: HashSet<PaneId>,
    agent_ids: HashSet<PaneId>,
    workspace_git_ids: HashSet<WorkspaceId>,
    zoomed_pane_ids: HashSet<PaneId>,
}

fn checked_bootstrap_total_size(current: usize, additional: usize) -> Result<usize, String> {
    let total = current
        .checked_add(additional)
        .ok_or_else(|| "Bootstrap total size overflowed usize".to_string())?;
    if total > MAX_BOOTSTRAP_TOTAL_SIZE {
        return Err(format!(
            "Bootstrap exceeds the {MAX_BOOTSTRAP_TOTAL_SIZE}-byte aggregate limit"
        ));
    }
    Ok(total)
}

impl BootstrapAssembler {
    pub fn new(header: BootstrapHeader) -> Result<Self, String> {
        if header.batch_count > MAX_BOOTSTRAP_BATCHES {
            return Err(format!(
                "Bootstrap declares {} batches; limit is {MAX_BOOTSTRAP_BATCHES}",
                header.batch_count
            ));
        }

        Ok(Self {
            header,
            next_batch_index: 0,
            next_record_index: 0,
            next_chunk_index: 0,
            current_chunk_count: None,
            current_payload: Vec::new(),
            total_payload_size: 0,
            terminals: Vec::new(),
            agents: Vec::new(),
            workspace_git: Vec::new(),
            zoomed_panes: Vec::new(),
            terminal_ids: HashSet::new(),
            agent_ids: HashSet::new(),
            workspace_git_ids: HashSet::new(),
            zoomed_pane_ids: HashSet::new(),
        })
    }

    pub fn push(&mut self, batch: BootstrapBatch) -> Result<(), String> {
        if self.next_batch_index >= self.header.batch_count {
            return Err("Bootstrap contains more batches than declared".into());
        }
        if batch.server_id != self.header.server_id || batch.session_id != self.header.session_id {
            return Err("Bootstrap batch identity does not match its header".into());
        }
        if batch.batch_index != self.next_batch_index {
            return Err(format!(
                "Bootstrap batch index {} is out of order; expected {}",
                batch.batch_index, self.next_batch_index
            ));
        }
        if batch.record_index != self.next_record_index {
            return Err(format!(
                "Bootstrap record index {} is out of order; expected {}",
                batch.record_index, self.next_record_index
            ));
        }
        if batch.chunk_count == 0 || batch.chunk_index >= batch.chunk_count {
            return Err("Bootstrap batch has an invalid chunk range".into());
        }
        if batch.chunk_index != self.next_chunk_index {
            return Err(format!(
                "Bootstrap chunk index {} is out of order; expected {}",
                batch.chunk_index, self.next_chunk_index
            ));
        }
        if batch.payload.is_empty() {
            return Err("Bootstrap chunk payload is empty".into());
        }
        if batch.payload.len() > MAX_CHUNK_PAYLOAD_SIZE {
            return Err(format!(
                "Bootstrap chunk is {} bytes; limit is {MAX_CHUNK_PAYLOAD_SIZE} bytes",
                batch.payload.len()
            ));
        }

        match self.current_chunk_count {
            Some(chunk_count) if chunk_count != batch.chunk_count => {
                return Err("Bootstrap chunk count changed within a record".into());
            }
            Some(_) => {}
            None => self.current_chunk_count = Some(batch.chunk_count),
        }
        let remaining_chunks = batch.chunk_count - batch.chunk_index;
        let remaining_batches = self.header.batch_count - self.next_batch_index;
        if remaining_chunks > remaining_batches {
            return Err("Bootstrap record cannot fit in the declared batch count".into());
        }

        let next_size = self
            .current_payload
            .len()
            .checked_add(batch.payload.len())
            .ok_or_else(|| "Bootstrap record size overflowed usize".to_string())?;
        if next_size > MAX_CHUNKED_RECORD_SIZE {
            return Err(format!(
                "Bootstrap record exceeds the {MAX_CHUNKED_RECORD_SIZE}-byte limit"
            ));
        }
        self.total_payload_size =
            checked_bootstrap_total_size(self.total_payload_size, batch.payload.len())?;
        self.current_payload
            .try_reserve(batch.payload.len())
            .map_err(|_| "Bootstrap record allocation failed".to_string())?;
        self.current_payload.extend_from_slice(&batch.payload);
        self.next_batch_index = self
            .next_batch_index
            .checked_add(1)
            .ok_or_else(|| "Bootstrap batch index overflowed u32".to_string())?;
        self.next_chunk_index = self
            .next_chunk_index
            .checked_add(1)
            .ok_or_else(|| "Bootstrap chunk index overflowed u32".to_string())?;

        if self.next_chunk_index == batch.chunk_count {
            // A record kind this build does not know is skipped: the rest of the Bootstrap
            // is still complete state (ADR 0028).
            if let Some(record) = decode_bootstrap_record(&self.current_payload)? {
                self.insert_record(record)?;
            }
            self.current_payload.clear();
            self.current_chunk_count = None;
            self.next_chunk_index = 0;
            self.next_record_index = self
                .next_record_index
                .checked_add(1)
                .ok_or_else(|| "Bootstrap record index overflowed u32".to_string())?;
        }
        Ok(())
    }

    pub fn finish(self) -> Result<SessionBootstrap, String> {
        if self.next_batch_index != self.header.batch_count {
            return Err(format!(
                "Bootstrap ended after {} of {} declared batches",
                self.next_batch_index, self.header.batch_count
            ));
        }
        if self.current_chunk_count.is_some() || !self.current_payload.is_empty() {
            return Err("Bootstrap ended in the middle of a record".into());
        }
        Ok(SessionBootstrap {
            server_id: self.header.server_id,
            runtime_epoch: self.header.runtime_epoch,
            session_id: self.header.session_id,
            sequence: self.header.sequence,
            snapshot: self.header.snapshot,
            settings: self.header.settings,
            terminals: self.terminals,
            agents: self.agents,
            workspace_git: self.workspace_git,
            zoomed_panes: self.zoomed_panes,
        })
    }

    fn insert_record(&mut self, record: BootstrapRecord) -> Result<(), String> {
        match record {
            BootstrapRecord::Terminal(terminal) => {
                if !self.terminal_ids.insert(terminal.pane_id) {
                    return Err("Bootstrap contains a duplicate Terminal Pane ID".into());
                }
                self.terminals.push(terminal);
            }
            BootstrapRecord::Agent(agent) => {
                if !self.agent_ids.insert(agent.pane_id) {
                    return Err("Bootstrap contains a duplicate Agent Pane ID".into());
                }
                self.agents.push(agent);
            }
            BootstrapRecord::WorkspaceGit(git) => {
                if !self.workspace_git_ids.insert(git.workspace_id) {
                    return Err("Bootstrap contains a duplicate Workspace Git ID".into());
                }
                self.workspace_git.push(git);
            }
            BootstrapRecord::ZoomedPane(pane_id) => {
                if !self.zoomed_pane_ids.insert(pane_id) {
                    return Err("Bootstrap contains a duplicate zoomed Pane ID".into());
                }
                self.zoomed_panes.push(pane_id);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_aggregate_limit_rejects_overflow_without_allocating_the_payload() {
        assert_eq!(
            checked_bootstrap_total_size(MAX_BOOTSTRAP_TOTAL_SIZE - 1, 1).unwrap(),
            MAX_BOOTSTRAP_TOTAL_SIZE
        );
        assert!(checked_bootstrap_total_size(MAX_BOOTSTRAP_TOTAL_SIZE, 1).is_err());
        assert!(checked_bootstrap_total_size(usize::MAX, 1).is_err());
    }
}
