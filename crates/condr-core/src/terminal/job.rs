//! The Job Object a Windows Terminal's shell runs in (ADR 0030): the Job, not the process
//! table, says which processes belong to the Terminal.
// The Win32 Job API has no safe wrapper that adopts the process portable-pty spawned.
#![allow(unsafe_code)]

use super::*;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_BASIC_PROCESS_ID_LIST, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicAccountingInformation, JobObjectBasicProcessIdList,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject,
};

pub(super) struct Job(OwnedHandle);

impl Job {
    /// Its handle is not inheritable, so no process in it can keep it open. Closing the
    /// last handle, however the Server ends, ends everything still in it; a process may
    /// leave on purpose with `CREATE_BREAKAWAY_FROM_JOB`. No UI restrictions: a Job with
    /// them cannot nest inside one the Server already runs in.
    pub(super) fn new() -> io::Result<Self> {
        // SAFETY: null attributes and name create an anonymous, non-inheritable Job.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `handle` is a fresh Job handle that nothing else owns.
        let job = Self(unsafe { OwnedHandle::from_raw_handle(handle as RawHandle) });
        // SAFETY: all-zero is a valid value of this plain C struct.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
        // SAFETY: the pointer and length describe `limits`, which outlives the call.
        check(unsafe {
            SetInformationJobObject(
                job.handle(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size_of_val(&limits) as u32,
            )
        })?;
        Ok(job)
    }

    /// `process` must stay open for the call.
    pub(super) fn assign(&self, process: RawHandle) -> io::Result<()> {
        // SAFETY: the Job handle is open and the caller keeps `process` open.
        check(unsafe { AssignProcessToJobObject(self.handle(), process as HANDLE) })
    }

    /// Ends every process in the Job; the processes are gone only once
    /// [`Self::active_processes`] reaches zero.
    pub(super) fn terminate(&self) -> io::Result<()> {
        // SAFETY: the Job handle is open.
        check(unsafe { TerminateJobObject(self.handle(), 1) })
    }

    pub(super) fn active_processes(&self) -> io::Result<u32> {
        // SAFETY: all-zero is a valid value of this plain C struct.
        let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: the pointer and length describe `info`, which outlives the call.
        check(unsafe {
            QueryInformationJobObject(
                self.handle(),
                JobObjectBasicAccountingInformation,
                (&raw mut info).cast(),
                size_of_val(&info) as u32,
                std::ptr::null_mut(),
            )
        })?;
        Ok(info.ActiveProcesses)
    }

    /// The PIDs of the processes in the Job now, the shell among them while it runs.
    pub(super) fn process_ids(&self) -> io::Result<Vec<u32>> {
        let header = std::mem::offset_of!(JOBOBJECT_BASIC_PROCESS_ID_LIST, ProcessIdList);
        let mut capacity = 16;
        loop {
            // A `usize` buffer keeps the list and its `usize` entries aligned.
            let mut buffer = vec![0usize; header.div_ceil(size_of::<usize>()) + capacity];
            let list = buffer
                .as_mut_ptr()
                .cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
            // SAFETY: the pointer and length describe `buffer`, which outlives the call.
            let result = check(unsafe {
                QueryInformationJobObject(
                    self.handle(),
                    JobObjectBasicProcessIdList,
                    list.cast(),
                    (buffer.len() * size_of::<usize>()) as u32,
                    std::ptr::null_mut(),
                )
            });
            // SAFETY: the call wrote the header, success or `ERROR_MORE_DATA` alike.
            let (assigned, listed) = unsafe {
                (
                    (*list).NumberOfAssignedProcesses as usize,
                    (*list).NumberOfProcessIdsInList as usize,
                )
            };
            match result {
                Ok(()) if listed >= assigned => {
                    // SAFETY: the call wrote `listed` entries after the header.
                    let ids = unsafe {
                        std::slice::from_raw_parts(
                            (&raw const (*list).ProcessIdList).cast::<usize>(),
                            listed,
                        )
                    };
                    return Ok(ids.iter().map(|&pid| pid as u32).collect());
                }
                Ok(()) => {}
                Err(error) if error.raw_os_error() == Some(ERROR_MORE_DATA as i32) => {}
                Err(error) => return Err(error),
            }
            // Processes may join while the list is read; leave room for a few more.
            capacity = assigned.max(capacity) + 16;
        }
    }

    fn handle(&self) -> HANDLE {
        self.0.as_raw_handle() as HANDLE
    }
}

fn check(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
