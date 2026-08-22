use std::{io, os::unix::process::CommandExt, process::Command};

use rustix::process::{Pid, Signal, kill_process_group, test_kill_process_group};

pub(super) fn configure(command: &mut Command) {
    command.process_group(0);
}

pub(super) struct ChildTree {
    group: Pid,
}

impl ChildTree {
    pub(super) fn attach(pid: u32) -> io::Result<Self> {
        let raw = i32::try_from(pid)
            .map_err(|_| io::Error::other("child process identity is invalid"))?;
        let raw = Pid::from_raw(raw)
            .ok_or_else(|| io::Error::other("child process identity is invalid"))?;
        Ok(Self { group: raw })
    }

    pub(super) fn request_termination(&self) -> io::Result<()> {
        signal_group(self.group, Signal::TERM)
    }

    pub(super) fn force_termination(&self) -> io::Result<()> {
        signal_group(self.group, Signal::KILL)
    }

    pub(super) fn has_live_processes(&self) -> io::Result<bool> {
        match test_kill_process_group(self.group) {
            Ok(()) => Ok(true),
            Err(rustix::io::Errno::SRCH) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
}

impl Drop for ChildTree {
    fn drop(&mut self) {
        let _ = self.force_termination();
    }
}

fn signal_group(group: Pid, signal: Signal) -> io::Result<()> {
    match kill_process_group(group, signal) {
        Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
        Err(error) => Err(error.into()),
    }
}
