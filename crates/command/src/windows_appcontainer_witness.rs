//! 仅供隔离候选的原生取证：PID/TID 只用于发现，权限以原 Job 和新取得的句柄复核。
//! 不把这些诊断句柄加入调试事件授权表，也不替代调用方的 token 与映像租约校验。

use std::io;
use std::mem::size_of;
use std::os::windows::io::{AsHandle as _, BorrowedHandle, OwnedHandle};

use windows::Win32::Foundation::{ERROR_NO_MORE_FILES, FILETIME, HANDLE, WAIT_TIMEOUT};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows::Win32::System::JobObjects::{
    IsProcessInJob, JobObjectBasicProcessIdList, QueryInformationJobObject,
};
use windows::Win32::System::StationsAndDesktops::HDESK;
use windows::Win32::System::Threading::{
    GetProcessId, GetProcessIdOfThread, GetProcessTimes, GetThreadId, OpenProcess, OpenThread,
    PROCESS_QUERY_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_VM_READ, THREAD_GET_CONTEXT,
    THREAD_QUERY_INFORMATION, THREAD_SUSPEND_RESUME, WaitForSingleObject,
};
use windows::core::{BOOL, HRESULT};

use super::{AppContainerProbe, handle, owned};

const MAX_PROCESSES: usize = 32;
const MAX_THREADS: usize = 32;
const MAX_SYSTEM_THREADS: usize = 65536;

#[repr(C)]
#[derive(Default)]
struct ProcessList {
    assigned: u32,
    listed: u32,
    ids: [usize; MAX_PROCESSES],
}

fn creation_time(process: HANDLE) -> io::Result<u64> {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) }
        .map_err(io::Error::other)?;
    Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

fn verify_member(process: HANDLE, job: HANDLE) -> io::Result<()> {
    let mut member = BOOL::default();
    unsafe { IsProcessInJob(process, Some(job), &mut member) }.map_err(io::Error::other)?;
    if !member.as_bool() || unsafe { WaitForSingleObject(process, 0) } != WAIT_TIMEOUT {
        return Err(io::Error::other("取证进程已退出或不在原严格 Job"));
    }
    Ok(())
}

/// 持有发现时已复核的内核对象；调用方仍须核对原 token 和映像租约。
pub struct NativeWitnessProcess {
    process: OwnedHandle,
    job: OwnedHandle,
    created_filetime: u64,
}

impl NativeWitnessProcess {
    pub fn process(&self) -> BorrowedHandle<'_> {
        self.process.as_handle()
    }

    pub fn created_filetime(&self) -> u64 {
        self.created_filetime
    }

    fn verify(&self) -> io::Result<()> {
        verify_member(handle(&self.process), handle(&self.job))?;
        if creation_time(handle(&self.process))? != self.created_filetime {
            return Err(io::Error::other("取证进程创建身份变化"));
        }
        Ok(())
    }

    /// 各线程的失败独立保留，不能把退出或无权限误记成已获得上下文。
    pub fn threads(&self) -> io::Result<Vec<(u32, io::Result<OwnedHandle>)>> {
        self.verify()?;
        let pid = unsafe { GetProcessId(handle(&self.process)) };
        if pid == 0 {
            return Err(io::Error::last_os_error());
        }
        let snapshot = owned(
            unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }.map_err(io::Error::other)?,
        );
        let mut entry = THREADENTRY32 {
            dwSize: size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        let mut next = unsafe { Thread32First(handle(&snapshot), &mut entry) };
        let mut seen = 0;
        let mut threads = Vec::new();
        loop {
            match next {
                Ok(()) => {}
                Err(error) if error.code() == HRESULT::from_win32(ERROR_NO_MORE_FILES.0) => break,
                Err(error) => return Err(io::Error::other(error)),
            }
            seen += 1;
            if seen > MAX_SYSTEM_THREADS {
                return Err(io::Error::other("取证线程枚举超过固定边界"));
            }
            if entry.dwSize < size_of::<THREADENTRY32>() as u32 {
                return Err(io::Error::other("取证线程记录不完整"));
            }
            if entry.th32OwnerProcessID == pid {
                if threads.len() >= MAX_THREADS {
                    return Err(io::Error::other("取证目标线程数量超过固定边界"));
                }
                let id = entry.th32ThreadID;
                let thread = (|| {
                    self.verify()?;
                    let thread = owned(
                        unsafe {
                            OpenThread(
                                THREAD_GET_CONTEXT
                                    | THREAD_QUERY_INFORMATION
                                    | THREAD_SUSPEND_RESUME,
                                false,
                                id,
                            )
                        }
                        .map_err(io::Error::other)?,
                    );
                    if unsafe { GetThreadId(handle(&thread)) } != id
                        || unsafe { GetProcessIdOfThread(handle(&thread)) } != pid
                    {
                        return Err(io::Error::other("取证线程原句柄归属不匹配"));
                    }
                    self.verify()?;
                    Ok(thread)
                })();
                threads.push((id, thread));
            }
            entry.dwSize = size_of::<THREADENTRY32>() as u32;
            next = unsafe { Thread32Next(handle(&snapshot), &mut entry) };
        }
        self.verify()?;
        Ok(threads)
    }
}

impl AppContainerProbe {
    /// 一次有界发现；未观察到 CREATE 的成员也只取得只读进程权限。
    pub fn native_witness_processes(
        &self,
    ) -> io::Result<Vec<(u32, io::Result<NativeWitnessProcess>)>> {
        if self.cleaned {
            return Err(io::Error::other("取证 Job 已清理"));
        }
        let mut members = ProcessList::default();
        unsafe {
            QueryInformationJobObject(
                Some(handle(&self.job)),
                JobObjectBasicProcessIdList,
                &mut members as *mut _ as *mut _,
                size_of::<ProcessList>() as u32,
                None,
            )
        }
        .map_err(io::Error::other)?;
        if members.listed > MAX_PROCESSES as u32 || members.listed != members.assigned {
            return Err(io::Error::other("取证 Job 成员表不完整"));
        }
        let mut processes = Vec::new();
        for &id in &members.ids[..members.listed as usize] {
            let id = u32::try_from(id).map_err(io::Error::other)?;
            if id == 0 {
                return Err(io::Error::other("取证 Job 包含无效进程 ID"));
            }
            let process = (|| {
                let process = owned(
                    unsafe {
                        OpenProcess(
                            PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_SYNCHRONIZE,
                            false,
                            id,
                        )
                    }
                    .map_err(io::Error::other)?,
                );
                if unsafe { GetProcessId(handle(&process)) } != id {
                    return Err(io::Error::other("取证进程原句柄身份不匹配"));
                }
                verify_member(handle(&process), handle(&self.job))?;
                let result = NativeWitnessProcess {
                    created_filetime: creation_time(handle(&process))?,
                    process,
                    job: self.job.try_clone()?,
                };
                result.verify()?;
                Ok(result)
            })();
            processes.push((id, process));
        }
        Ok(processes)
    }

    /// 借用由原 helper 实际桌面对象复制的句柄；不通过名称或调用方窗口站推定。
    pub fn native_witness_desktop(&self) -> io::Result<Option<HDESK>> {
        match &self.private_station {
            Some(station) => station.native_witness_desktop().map(Some),
            None => Ok(None),
        }
    }
}
