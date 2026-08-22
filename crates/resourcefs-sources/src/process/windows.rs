use std::{
    ffi::c_void,
    io,
    mem::{offset_of, size_of},
    os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle},
        process::CommandExt,
    },
    process::Command,
    ptr,
};

use windows_sys::Win32::{
    Foundation::{
        ERROR_INVALID_PARAMETER, HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    },
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_BASIC_PROCESS_ID_LIST,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicAccountingInformation,
            JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation,
            QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
        },
        Threading::{
            CREATE_SUSPENDED, OpenProcess, OpenThread, PROCESS_SYNCHRONIZE, ResumeThread,
            THREAD_SUSPEND_RESUME, WaitForSingleObject,
        },
    },
};

pub(super) fn configure(command: &mut Command) {
    command.creation_flags(CREATE_SUSPENDED);
}

pub(super) struct ChildTree {
    job: OwnedHandle,
    process_id_buffer: Vec<usize>,
    processes: Vec<TrackedProcess>,
}

struct TrackedProcess {
    id: usize,
    handle: OwnedHandle,
}

impl ChildTree {
    pub(super) fn attach(pid: u32, process: RawHandle) -> io::Result<Self> {
        // SAFETY: Null security/name pointers request an unnamed Job with default security.
        let job = owned_null(unsafe { CreateJobObjectW(ptr::null(), ptr::null()) })?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `job` is live and the pointer/length describe `limits` exactly.
        check(unsafe {
            SetInformationJobObject(
                job_handle(&job),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast::<c_void>(),
                structure_size::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()?,
            )
        })?;
        // SAFETY: Tokio owns the live process handle for this suspended child.
        check(unsafe { AssignProcessToJobObject(job_handle(&job), process.cast()) })?;

        let mut tree = Self {
            job,
            process_id_buffer: vec![0; process_list_header_words() + 16],
            processes: Vec::new(),
        };
        tree.track_processes()?;
        tree.resume_primary_thread(pid)?;
        Ok(tree)
    }

    pub(super) fn request_termination(&mut self) -> io::Result<()> {
        let tracking = self.track_processes();
        self.terminate().and(tracking)
    }

    pub(super) fn force_termination(&mut self) -> io::Result<()> {
        let tracking = self.track_processes();
        self.terminate().and(tracking)
    }

    pub(super) fn has_live_processes(&mut self) -> io::Result<bool> {
        self.track_processes()?;
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        // SAFETY: `self.job` is live and the pointer/length describe `accounting` exactly.
        check(unsafe {
            QueryInformationJobObject(
                job_handle(&self.job),
                JobObjectBasicAccountingInformation,
                (&raw mut accounting).cast::<c_void>(),
                structure_size::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>()?,
                ptr::null_mut(),
            )
        })?;
        for process in &self.processes {
            // SAFETY: Each stored process handle remains live for this zero-time wait.
            match unsafe { WaitForSingleObject(process_handle(&process.handle), 0) } {
                WAIT_OBJECT_0 => {}
                WAIT_TIMEOUT => return Ok(true),
                WAIT_FAILED => return Err(io::Error::last_os_error()),
                _ => {
                    return Err(io::Error::other(
                        "Windows process wait returned invalid state",
                    ));
                }
            }
        }
        Ok(accounting.ActiveProcesses != 0)
    }
    fn track_processes(&mut self) -> io::Result<()> {
        let count = self.query_process_ids()?;
        let first_id = process_list_header_words();
        for index in 0..count {
            let id = self.process_id_buffer[first_id + index];
            if self.processes.iter().any(|process| process.id == id) {
                continue;
            }
            let pid = u32::try_from(id)
                .map_err(|_| io::Error::other("Windows process identity is invalid"))?;
            // SAFETY: The job query returned this process ID and only synchronization access is requested.
            let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
            if raw.is_null() {
                let error = io::Error::last_os_error();
                if error
                    .raw_os_error()
                    .is_some_and(|code| code as u32 == ERROR_INVALID_PARAMETER)
                {
                    continue;
                }
                return Err(error);
            }
            // SAFETY: OpenProcess returned a new non-null owned handle.
            let handle = unsafe { OwnedHandle::from_raw_handle(raw.cast()) };
            self.processes.push(TrackedProcess { id, handle });
        }
        Ok(())
    }

    fn query_process_ids(&mut self) -> io::Result<usize> {
        loop {
            let byte_len = self
                .process_id_buffer
                .len()
                .checked_mul(size_of::<usize>())
                .ok_or_else(|| io::Error::other("Windows process list is too large"))?;
            let byte_len = u32::try_from(byte_len)
                .map_err(|_| io::Error::other("Windows process list is too large"))?;
            // SAFETY: The aligned buffer is writable for `byte_len`; this information class writes
            // the fixed header followed by pointer-sized process IDs.
            let success = unsafe {
                QueryInformationJobObject(
                    job_handle(&self.job),
                    JobObjectBasicProcessIdList,
                    self.process_id_buffer.as_mut_ptr().cast::<c_void>(),
                    byte_len,
                    ptr::null_mut(),
                )
            };
            // SAFETY: The buffer is at least the size of the fixed process-list structure.
            let process_list = unsafe {
                &*self
                    .process_id_buffer
                    .as_ptr()
                    .cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>()
            };
            let assigned = process_list.NumberOfAssignedProcesses as usize;
            let listed = process_list.NumberOfProcessIdsInList as usize;
            let capacity = self.process_id_buffer.len() - process_list_header_words();
            if listed > capacity {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Windows returned an invalid job process list",
                ));
            }
            if success != 0 && listed == assigned {
                return Ok(listed);
            }
            if assigned <= capacity {
                return Err(io::Error::last_os_error());
            }
            let required = process_list_header_words()
                .checked_add(assigned)
                .ok_or_else(|| io::Error::other("Windows process list is too large"))?;
            self.process_id_buffer.resize(required, 0);
        }
    }

    fn resume_primary_thread(&self, pid: u32) -> io::Result<()> {
        // SAFETY: The snapshot API returns a new owned handle or INVALID_HANDLE_VALUE.
        let snapshot = owned_invalid(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) })?;
        let mut entry = THREADENTRY32 {
            dwSize: structure_size::<THREADENTRY32>()?,
            ..THREADENTRY32::default()
        };
        // SAFETY: `snapshot` is live and `entry.dwSize` names the complete writable structure.
        if unsafe { Thread32First(snapshot_handle(&snapshot), &raw mut entry) } == 0 {
            return Err(io::Error::last_os_error());
        }
        loop {
            if entry.th32OwnerProcessID == pid {
                // SAFETY: The enumerated thread ID belongs to the still-suspended child.
                let thread = owned_null(unsafe {
                    OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID)
                })?;
                // SAFETY: `thread` has THREAD_SUSPEND_RESUME access and is live.
                if unsafe { ResumeThread(thread_handle(&thread)) } == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                return Ok(());
            }
            // SAFETY: `snapshot` and `entry` remain live and correctly sized.
            if unsafe { Thread32Next(snapshot_handle(&snapshot), &raw mut entry) } == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "suspended child thread was not found",
                ));
            }
        }
    }

    fn terminate(&self) -> io::Result<()> {
        // SAFETY: `self.job` is live; exit code 1 is intentionally non-success.
        check(unsafe { TerminateJobObject(job_handle(&self.job), 1) })
    }
}

impl Drop for ChildTree {
    fn drop(&mut self) {
        let _ = self.force_termination();
    }
}

fn check(success: i32) -> io::Result<()> {
    if success == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn structure_size<T>() -> io::Result<u32> {
    u32::try_from(size_of::<T>()).map_err(|_| io::Error::other("Windows structure is too large"))
}
const fn process_list_header_words() -> usize {
    offset_of!(JOBOBJECT_BASIC_PROCESS_ID_LIST, ProcessIdList) / size_of::<usize>()
}

fn owned_null(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: The API returned a new non-null owned handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle.cast()) })
}

fn owned_invalid(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: The API returned a new handle distinct from INVALID_HANDLE_VALUE.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle.cast()) })
}

fn job_handle(handle: &OwnedHandle) -> HANDLE {
    handle.as_raw_handle().cast()
}
fn process_handle(handle: &OwnedHandle) -> HANDLE {
    handle.as_raw_handle().cast()
}

fn snapshot_handle(handle: &OwnedHandle) -> HANDLE {
    handle.as_raw_handle().cast()
}

fn thread_handle(handle: &OwnedHandle) -> HANDLE {
    handle.as_raw_handle().cast()
}
