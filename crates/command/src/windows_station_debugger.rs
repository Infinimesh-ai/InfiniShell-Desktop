//! 第二段原创建线程拥有调试端口；调用方只经已绑定管道领取原句柄副本。

use std::collections::HashMap;
use std::ffi::c_void;
use std::io;
use std::mem::{size_of, size_of_val};
use std::os::windows::io::{IntoRawHandle as _, OwnedHandle};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{
    DBG_CONTINUE, DBG_EXCEPTION_NOT_HANDLED, DUPLICATE_SAME_ACCESS, DuplicateHandle,
    ERROR_SEM_TIMEOUT, GetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, HLOCAL, LocalFree,
    NTSTATUS,
};
use windows::Win32::Security::Authorization::ConvertStringSidToSidW;
use windows::Win32::Security::{PSID, SECURITY_CAPABILITIES};
use windows::Win32::System::Diagnostics::Debug::{
    CREATE_PROCESS_DEBUG_EVENT, CREATE_THREAD_DEBUG_EVENT, ContinueDebugEvent, DEBUG_EVENT,
    DebugSetProcessKillOnExit, EXIT_PROCESS_DEBUG_EVENT, LOAD_DLL_DEBUG_EVENT, WaitForDebugEvent,
};
use windows::Win32::System::Threading::{
    CREATE_NEW_CONSOLE, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    CreateProcessW, DEBUG_PROCESS, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, GetProcessId,
    GetProcessIdOfThread, GetThreadId, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION, STARTF_USESHOWWINDOW,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{HRESULT, PCWSTR, PWSTR};

use super::super::appcontainer::Attributes;
use super::{
    CLEANUP_TIMEOUT, Closed, FileIdentity, PathLease, Pipe, Request, START_TIMEOUT, Snapshot,
    alive, invalid, owned, raw, require, resume, wait_step, wide,
};

#[path = "windows_station_debug_event.rs"]
mod event;
use event::Event;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::windows) struct Spawn {
    program: PathBuf,
    program_identity: FileIdentity,
    command: Vec<u16>,
    environment: Vec<u16>,
    cwd: PathBuf,
    streams: [u64; 3],
    hidden_console: bool,
}

impl Spawn {
    pub(in crate::windows) fn new(
        program: &Path,
        command: Vec<u16>,
        environment: Vec<u16>,
        cwd: &Path,
        streams: [HANDLE; 3],
        hidden_console: bool,
    ) -> io::Result<(Self, PathLease)> {
        let lease = PathLease::capture(program)?;
        let request = Self {
            program: lease.path().to_owned(),
            program_identity: lease.entries[0].2.clone(),
            command,
            environment,
            cwd: cwd.to_owned(),
            streams: streams.map(|value| value.0 as usize as u64),
            hidden_console,
        };
        Ok((request, lease))
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Message {
    nonce: String,
    sequence: u64,
    operation: Operation,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
enum Operation {
    Spawn(Spawn),
    Wait,
    Continue {
        event: u64,
        pid: u32,
        tid: u32,
        code: u32,
        status: i32,
    },
    Resume,
    Close,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    nonce: String,
    sequence: u64,
    response: Response,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
enum Response {
    Spawned { process: u64, thread: u64, pid: u32 },
    Event(Option<Event>),
    Done,
    Closed(Closed),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pending {
    sequence: u64,
    pid: u32,
    tid: u32,
    code: u32,
}
impl Pending {
    fn matches(&self, sequence: u64, pid: u32, tid: u32, code: u32) -> bool {
        *self
            == Self {
                sequence,
                pid,
                tid,
                code,
            }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Inflight {
    Wait,
    Continue(Pending, i32),
    Other,
}
struct Client {
    pipe: Pipe,
    peer: OwnedHandle,
    identity: Snapshot,
    nonce: String,
    sequence: u64,
    inflight: Option<Inflight>,
    pending: Option<Pending>,
    delivery: Option<Event>,
    event_handles: Vec<OwnedHandle>,
    resumed: bool,
}

/// 克隆不延长原生句柄寿命；PrivateStation 回收时会清空共享槽。
#[derive(Clone)]
pub struct StationDebugger(Arc<Mutex<Option<Client>>>);
impl std::fmt::Debug for StationDebugger {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StationDebugger")
            .finish_non_exhaustive()
    }
}

fn duplicate(peer: HANDLE, source: HANDLE) -> io::Result<OwnedHandle> {
    require(!source.is_invalid(), "调试远端句柄无效")?;
    let mut result = HANDLE::default();
    unsafe {
        DuplicateHandle(
            peer,
            source,
            GetCurrentProcess(),
            &mut result,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
    }
    .map_err(io::Error::other)?;
    Ok(owned(result))
}
fn remote(value: u64) -> io::Result<HANDLE> {
    require(value > 0 && value < isize::MAX as u64, "调试远端句柄越界")?;
    Ok(HANDLE(value as usize as *mut c_void))
}
impl Client {
    fn send(&mut self, operation: Operation, inflight: Inflight) -> io::Result<()> {
        require(self.inflight.is_none(), "调试请求仍在进行")?;
        require(
            Snapshot::capture(raw(&self.peer))? == self.identity && alive(raw(&self.peer))?,
            "调试原创建者身份变化",
        )?;
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| invalid("调试请求序号溢出"))?;
        // 发送结果未知也不能重放产生副作用的请求。
        self.inflight = Some(inflight);
        self.pipe.send(&Message {
            nonce: self.nonce.clone(),
            sequence: self.sequence,
            operation,
        })
    }
    fn receive(&mut self, deadline: Instant) -> io::Result<Option<Response>> {
        loop {
            if Instant::now() >= deadline {
                return Ok(None);
            }
            if let Some(reply) = self.pipe.try_receive::<Reply>()? {
                require(
                    reply.nonce == self.nonce
                        && reply.sequence == self.sequence
                        && self.inflight.is_some(),
                    "调试回复绑定不匹配",
                )?;
                self.inflight = None;
                return Ok(Some(reply.response));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            wait_step(deadline, raw(&self.peer)).or_else(|error| {
                if error.kind() == io::ErrorKind::TimedOut {
                    Ok(())
                } else {
                    Err(error)
                }
            })?;
        }
    }
    fn complete(&mut self, deadline: Instant) -> io::Result<Response> {
        self.receive(deadline)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "调试控制回复尚未收到"))
    }
}
impl StationDebugger {
    pub(super) fn new(
        pipe: Pipe,
        peer: HANDLE,
        identity: Snapshot,
        nonce: String,
    ) -> io::Result<Self> {
        let peer = duplicate(unsafe { GetCurrentProcess() }, peer)?;
        Ok(Self(Arc::new(Mutex::new(Some(Client {
            pipe,
            peer,
            identity,
            nonce,
            sequence: 0,
            inflight: None,
            pending: None,
            delivery: None,
            event_handles: Vec::new(),
            resumed: false,
        })))))
    }
    fn lock(&self) -> io::Result<MutexGuard<'_, Option<Client>>> {
        self.0.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => {
                io::Error::new(io::ErrorKind::WouldBlock, "调试控制已有在途调用")
            }
            TryLockError::Poisoned(_) => invalid("调试控制状态不可用"),
        })
    }
    pub(super) fn invalidate(&self) -> io::Result<()> {
        match self.0.try_lock() {
            Ok(mut client) => {
                client.take();
                Ok(())
            }
            Err(TryLockError::WouldBlock) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "调试控制已有在途调用",
            )),
            Err(TryLockError::Poisoned(failure)) => {
                // 即使状态已中毒也先释放句柄，不把共享克隆留作 LUID 的隐藏持有者。
                failure.into_inner().take();
                Err(invalid("调试控制状态不可用"))
            }
        }
    }
    pub(super) fn spawn(&self, request: Spawn) -> io::Result<PROCESS_INFORMATION> {
        let mut guard = self.lock()?;
        let client = guard.as_mut().ok_or_else(|| invalid("调试控制已关闭"))?;
        client.send(Operation::Spawn(request), Inflight::Other)?;
        match client.complete(Instant::now() + START_TIMEOUT)? {
            Response::Spawned {
                process,
                thread,
                pid,
            } => {
                let process = duplicate(raw(&client.peer), remote(process)?)?;
                let thread = duplicate(raw(&client.peer), remote(thread)?)?;
                require(
                    unsafe {
                        GetProcessId(raw(&process)) == pid
                            && GetProcessIdOfThread(raw(&thread)) == pid
                    },
                    "调试根进程原句柄不匹配",
                )?;
                let tid = unsafe { GetThreadId(raw(&thread)) };
                Ok(PROCESS_INFORMATION {
                    hProcess: HANDLE(process.into_raw_handle()),
                    hThread: HANDLE(thread.into_raw_handle()),
                    dwProcessId: pid,
                    dwThreadId: tid,
                })
            }
            Response::Event(_) | Response::Done | Response::Closed(_) => {
                Err(invalid("调试创建回复类型无效"))
            }
        }
    }
    pub(super) fn resume(&self) -> io::Result<()> {
        let mut guard = self.lock()?;
        let client = guard.as_mut().ok_or_else(|| invalid("调试控制已关闭"))?;
        require(!client.resumed, "调试根进程恢复不可重放")?;
        client.resumed = true;
        client.send(Operation::Resume, Inflight::Other)?;
        match client.complete(Instant::now() + START_TIMEOUT)? {
            Response::Done => Ok(()),
            Response::Spawned { .. } | Response::Event(_) | Response::Closed(_) => {
                Err(invalid("调试恢复回复类型无效"))
            }
        }
    }
    /// 超时后保留在途 Wait；下次只领取同一回复，不重复领取事件。
    pub fn wait_event(
        &self,
        timeout_ms: u32,
        deadline: Instant,
    ) -> io::Result<Option<DEBUG_EVENT>> {
        require(timeout_ms > 0 && timeout_ms <= 100, "调试领取时间片无效")?;
        let deadline =
            deadline.min(Instant::now() + std::time::Duration::from_millis(timeout_ms.into()));
        let mut guard = self.lock()?;
        let client = guard.as_mut().ok_or_else(|| invalid("调试控制已关闭"))?;
        require(client.pending.is_none(), "调试事件尚未继续")?;
        if Instant::now() >= deadline {
            return Ok(None);
        }
        if client.delivery.is_none() {
            match client.inflight {
                None => client.send(Operation::Wait, Inflight::Wait)?,
                Some(Inflight::Wait) => {}
                Some(Inflight::Continue(_, _) | Inflight::Other) => {
                    return Err(invalid("调试控制回复仍待领取"));
                }
            }
            let Some(response) = client.receive(deadline)? else {
                return Ok(None);
            };
            client.delivery = match response {
                Response::Event(event) => event,
                Response::Spawned { .. } | Response::Done | Response::Closed(_) => {
                    return Err(invalid("调试事件回复类型无效"));
                }
            };
        }
        let Some(event) = client.delivery.as_ref() else {
            return Ok(None);
        };
        let mut native = event.restore()?;
        let mut file = None;
        let mut handles = Vec::new();
        unsafe {
            match native.dwDebugEventCode {
                CREATE_PROCESS_DEBUG_EVENT => {
                    let value = &mut native.u.CreateProcessInfo;
                    if !value.hFile.is_invalid() {
                        file = Some(duplicate(raw(&client.peer), value.hFile)?);
                    }
                    handles.push(duplicate(raw(&client.peer), value.hProcess)?);
                    value.hProcess = raw(&handles[0]);
                    handles.push(duplicate(raw(&client.peer), value.hThread)?);
                    value.hThread = raw(&handles[1]);
                    require(
                        GetProcessId(value.hProcess) == event.pid
                            && GetThreadId(value.hThread) == event.tid
                            && GetProcessIdOfThread(value.hThread) == event.pid,
                        "调试创建事件原句柄身份不匹配",
                    )?;
                    value.hFile = file.as_ref().map_or(HANDLE::default(), raw);
                }
                CREATE_THREAD_DEBUG_EVENT => {
                    handles.push(duplicate(raw(&client.peer), native.u.CreateThread.hThread)?);
                    native.u.CreateThread.hThread = raw(&handles[0]);
                    require(
                        GetThreadId(native.u.CreateThread.hThread) == event.tid
                            && GetProcessIdOfThread(native.u.CreateThread.hThread) == event.pid,
                        "调试线程事件原句柄身份不匹配",
                    )?;
                }
                LOAD_DLL_DEBUG_EVENT => {
                    if !native.u.LoadDll.hFile.is_invalid() {
                        file = Some(duplicate(raw(&client.peer), native.u.LoadDll.hFile)?);
                    }
                    native.u.LoadDll.hFile = file.as_ref().map_or(HANDLE::default(), raw);
                }
                _ => {}
            }
        }
        client.pending = Some(Pending {
            sequence: event.sequence,
            pid: event.pid,
            tid: event.tid,
            code: event.code,
        });
        client.delivery = None;
        client.event_handles = handles;
        if let Some(file) = file {
            let _ = file.into_raw_handle();
        }
        Ok(Some(native))
    }
    /// 回复丢失时再次调用只续收原请求；绝不再次调用原生 ContinueDebugEvent。
    pub fn continue_event(
        &self,
        pid: u32,
        tid: u32,
        status: NTSTATUS,
        deadline: Instant,
    ) -> io::Result<NTSTATUS> {
        require(
            status == DBG_CONTINUE || status == DBG_EXCEPTION_NOT_HANDLED,
            "调试继续状态无效",
        )?;
        let mut guard = self.lock()?;
        let client = guard.as_mut().ok_or_else(|| invalid("调试控制已关闭"))?;
        let pending = client.pending.ok_or_else(|| invalid("调试事件缺失"))?;
        require(
            pending.pid == pid && pending.tid == tid,
            "调试继续身份不匹配",
        )?;
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "调试继续期限已结束",
            ));
        }
        let actual_status = match client.inflight {
            None => {
                client.send(
                    Operation::Continue {
                        event: pending.sequence,
                        pid,
                        tid,
                        code: pending.code,
                        status: status.0,
                    },
                    Inflight::Continue(pending, status.0),
                )?;
                status
            }
            Some(Inflight::Continue(value, original_status)) if value == pending => {
                NTSTATUS(original_status)
            }
            Some(Inflight::Continue(_, _) | Inflight::Wait | Inflight::Other) => {
                return Err(invalid("调试继续请求不匹配"));
            }
        };
        match client.complete(deadline)? {
            Response::Done => {
                client.pending = None;
                client.event_handles.clear();
                Ok(actual_status)
            }
            Response::Spawned { .. } | Response::Event(_) | Response::Closed(_) => {
                Err(invalid("调试继续回复类型无效"))
            }
        }
    }
    pub(super) fn close(&self, deadline: Instant) -> io::Result<Closed> {
        let mut guard = self.lock()?;
        let client = guard.as_mut().ok_or_else(|| invalid("调试控制已关闭"))?;
        require(client.pending.is_none(), "调试事件尚未排空")?;
        client.send(Operation::Close, Inflight::Other)?;
        match client.complete(deadline)? {
            Response::Closed(closed) => Ok(closed),
            Response::Spawned { .. } | Response::Event(_) | Response::Done => {
                Err(invalid("调试关闭回复类型无效"))
            }
        }
    }
}

struct Package {
    process: OwnedHandle,
    thread: Option<OwnedHandle>,
    pid: u32,
    image: PathLease,
    processes: HashMap<u32, OwnedHandle>,
    pending: Option<(Pending, Option<OwnedHandle>, Option<HANDLE>)>,
    event_sequence: u64,
    root_exit: bool,
}
impl Package {
    fn spawn(request: Spawn, binding: &Request) -> io::Result<Self> {
        let image = PathLease::capture(&request.program)?;
        require(
            image.entries[0].2 == request.program_identity,
            "调试创建映像身份不匹配",
        )?;
        let cwd = PathLease::capture(&request.cwd)?;
        require(
            cwd.path() == binding.device_target.path
                && cwd.entries[0].2 == binding.device_target.identity,
            "调试创建目录身份不匹配",
        )?;
        require(
            request.command.last() == Some(&0)
                && !request.command[..request.command.len().saturating_sub(1)].contains(&0)
                && request.environment.ends_with(&[0, 0]),
            "调试创建参数终止符无效",
        )?;
        let program = image.application_path()?;
        let mut command = request.command;
        let mut handles = request
            .streams
            .map(remote)
            .into_iter()
            .collect::<io::Result<Vec<_>>>()?;
        require(
            handles.len() == 3
                && handles[0] != handles[1]
                && handles[0] != handles[2]
                && handles[1] != handles[2],
            "调试标准流白名单无效",
        )?;
        for handle in &handles {
            let mut flags = 0;
            unsafe { GetHandleInformation(*handle, &mut flags) }.map_err(io::Error::other)?;
            require(flags & HANDLE_FLAG_INHERIT.0 != 0, "调试标准流不可继承")?;
        }
        let sid_text = wide(binding.container_sid.as_ref())?;
        let mut sid = PSID::default();
        unsafe { ConvertStringSidToSidW(PCWSTR(sid_text.as_ptr()), &mut sid) }
            .map_err(io::Error::other)?;
        let created: io::Result<PROCESS_INFORMATION> = (|| {
            let mut capabilities = SECURITY_CAPABILITIES {
                AppContainerSid: sid,
                Capabilities: std::ptr::null_mut(),
                CapabilityCount: 0,
                Reserved: 0,
            };
            let attributes = Attributes::new(2)?;
            unsafe {
                UpdateProcThreadAttribute(
                    attributes.list,
                    0,
                    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                    Some((&mut capabilities as *mut SECURITY_CAPABILITIES).cast()),
                    size_of_val(&capabilities),
                    None,
                    None,
                )
            }
            .map_err(io::Error::other)?;
            unsafe {
                UpdateProcThreadAttribute(
                    attributes.list,
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    Some(handles.as_mut_ptr().cast()),
                    handles.len() * size_of::<HANDLE>(),
                    None,
                    None,
                )
            }
            .map_err(io::Error::other)?;
            let mut desktop = wide(
                format!(
                    "{}\\{}",
                    Snapshot::capture(unsafe { GetCurrentProcess() })?.station_name(),
                    binding.profile
                )
                .as_ref(),
            )?;
            let mut startup = STARTUPINFOEXW::default();
            startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            startup.StartupInfo.hStdInput = handles[0];
            startup.StartupInfo.hStdOutput = handles[1];
            startup.StartupInfo.hStdError = handles[2];
            startup.StartupInfo.lpDesktop = PWSTR(desktop.as_mut_ptr());
            startup.lpAttributeList = attributes.list;
            let console = if request.hidden_console {
                startup.StartupInfo.dwFlags |= STARTF_USESHOWWINDOW;
                startup.StartupInfo.wShowWindow = SW_HIDE.0 as u16;
                CREATE_NEW_CONSOLE
            } else {
                CREATE_NO_WINDOW
            };
            let directory = wide(request.cwd.as_os_str())?;
            image.verify()?;
            cwd.verify()?;
            let mut process = PROCESS_INFORMATION::default();
            unsafe {
                CreateProcessW(
                    PCWSTR(program.as_ptr()),
                    Some(PWSTR(command.as_mut_ptr())),
                    None,
                    None,
                    true,
                    CREATE_SUSPENDED
                        | CREATE_UNICODE_ENVIRONMENT
                        | EXTENDED_STARTUPINFO_PRESENT
                        | DEBUG_PROCESS
                        | console,
                    Some(request.environment.as_ptr().cast()),
                    PCWSTR(directory.as_ptr()),
                    &startup.StartupInfo,
                    &mut process,
                )
            }
            .map_err(io::Error::other)?;
            Ok(process)
        })();
        unsafe { LocalFree(Some(HLOCAL(sid.0))) };
        let process: PROCESS_INFORMATION = created?;
        Ok(Self {
            process: owned(process.hProcess),
            thread: Some(owned(process.hThread)),
            pid: process.dwProcessId,
            image,
            processes: HashMap::new(),
            pending: None,
            event_sequence: 0,
            root_exit: false,
        })
    }
    fn wait(&mut self) -> io::Result<Option<Event>> {
        require(self.pending.is_none(), "原调试事件尚未继续")?;
        let mut event = DEBUG_EVENT::default();
        match unsafe { WaitForDebugEvent(&mut event, 0) } {
            Ok(()) => {}
            Err(error) if error.code() == HRESULT::from_win32(ERROR_SEM_TIMEOUT.0) => {
                return Ok(None);
            }
            Err(error) => return Err(io::Error::other(error)),
        }
        self.event_sequence = self
            .event_sequence
            .checked_add(1)
            .ok_or_else(|| invalid("调试事件序号溢出"))?;
        let (file, process) = unsafe {
            match event.dwDebugEventCode {
                CREATE_PROCESS_DEBUG_EVENT => (
                    event.u.CreateProcessInfo.hFile,
                    Some(event.u.CreateProcessInfo.hProcess),
                ),
                LOAD_DLL_DEBUG_EVENT => (event.u.LoadDll.hFile, None),
                _ => (HANDLE::default(), None),
            }
        };
        // 先记住原事件再做可失败的复制；错误路径仍能终止原 CREATE 对象并继续它。
        self.pending = Some((
            Pending {
                sequence: self.event_sequence,
                pid: event.dwProcessId,
                tid: event.dwThreadId,
                code: event.dwDebugEventCode.0,
            },
            (!file.is_invalid()).then(|| owned(file)),
            process,
        ));
        if let Some(process) = process {
            require(
                !self.processes.contains_key(&event.dwProcessId),
                "原调试进程 ID 尚未退出",
            )?;
            self.processes.insert(
                event.dwProcessId,
                duplicate(unsafe { GetCurrentProcess() }, process)?,
            );
        }
        Event::capture(self.event_sequence, &event).map(Some)
    }
    fn continue_event(&mut self, expected: Pending, status: NTSTATUS) -> io::Result<()> {
        require(
            status == DBG_CONTINUE || status == DBG_EXCEPTION_NOT_HANDLED,
            "原调试继续状态无效",
        )?;
        let pending = self
            .pending
            .as_ref()
            .ok_or_else(|| invalid("原调试事件缺失"))?
            .0;
        require(
            pending.matches(expected.sequence, expected.pid, expected.tid, expected.code),
            "原调试继续绑定不匹配",
        )?;
        unsafe { ContinueDebugEvent(pending.pid, pending.tid, status) }
            .map_err(io::Error::other)?;
        self.pending.take();
        if pending.code == EXIT_PROCESS_DEBUG_EVENT.0 {
            self.processes.remove(&pending.pid);
            if pending.pid == self.pid {
                self.root_exit = true;
            }
        }
        Ok(())
    }
    fn drained(&self) -> bool {
        self.root_exit && self.processes.is_empty() && self.pending.is_none()
    }
    fn terminate_and_drain(&mut self) -> io::Result<()> {
        let deadline = Instant::now() + CLEANUP_TIMEOUT;
        if alive(raw(&self.process))? {
            unsafe { TerminateProcess(raw(&self.process), 1) }.map_err(io::Error::other)?;
        }
        while !self.drained() {
            for process in self.processes.values() {
                if alive(raw(process))? {
                    unsafe { TerminateProcess(raw(process), 1) }.map_err(io::Error::other)?;
                }
            }
            if let Some((pending, _, process)) = &self.pending {
                if let Some(process) = process {
                    if alive(*process)? {
                        unsafe { TerminateProcess(*process, 1) }.map_err(io::Error::other)?;
                    }
                }
                self.continue_event(*pending, DBG_CONTINUE)?;
            } else {
                self.wait()?;
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "原调试终止未在期限内排空",
                ));
            }
        }
        Ok(())
    }
}
impl Drop for Package {
    fn drop(&mut self) {
        if !self.drained() {
            let _ = self.terminate_and_drain();
        }
    }
}

pub(super) struct Server {
    sequence: u64,
    package: Option<Package>,
}
fn validate_message(nonce: &str, sequence: u64, message: &Message) -> io::Result<()> {
    require(
        message.nonce == nonce && sequence.checked_add(1) == Some(message.sequence),
        "原调试请求绑定不匹配",
    )
}
impl Server {
    pub(super) fn new() -> io::Result<Self> {
        Ok(Self {
            sequence: 0,
            package: None,
        })
    }
    // 返回关闭请求的序号；桌面/盘符真实清理完成后才发 Closed。
    pub(super) fn poll(&mut self, pipe: &Pipe, binding: &Request) -> io::Result<Option<u64>> {
        let Some(message) = pipe.try_receive::<Message>()? else {
            return Ok(None);
        };
        validate_message(&binding.nonce, self.sequence, &message)?;
        self.sequence = message.sequence;
        let response = match message.operation {
            Operation::Spawn(request) => {
                require(self.package.is_none(), "原调试创建不可重放")?;
                self.package = Some(Package::spawn(request, binding)?);
                unsafe { DebugSetProcessKillOnExit(true) }.map_err(io::Error::other)?;
                let package = self.package.as_ref().unwrap();
                Response::Spawned {
                    process: raw(&package.process).0 as usize as u64,
                    thread: raw(package.thread.as_ref().unwrap()).0 as usize as u64,
                    pid: package.pid,
                }
            }
            Operation::Wait => Response::Event(
                self.package
                    .as_mut()
                    .ok_or_else(|| invalid("原调试根缺失"))?
                    .wait()?,
            ),
            Operation::Continue {
                event,
                pid,
                tid,
                code,
                status,
            } => {
                self.package
                    .as_mut()
                    .ok_or_else(|| invalid("原调试根缺失"))?
                    .continue_event(
                        Pending {
                            sequence: event,
                            pid,
                            tid,
                            code,
                        },
                        NTSTATUS(status),
                    )?;
                Response::Done
            }
            Operation::Resume => {
                let package = self
                    .package
                    .as_mut()
                    .ok_or_else(|| invalid("原调试根缺失"))?;
                package.image.verify()?;
                let thread = package
                    .thread
                    .take()
                    .ok_or_else(|| invalid("原调试恢复不可重放"))?;
                resume(raw(&thread))?;
                Response::Done
            }
            Operation::Close => {
                require(
                    self.package.as_ref().is_none_or(Package::drained),
                    "原调试树尚未排空",
                )?;
                self.package.take();
                return Ok(Some(message.sequence));
            }
        };
        pipe.send(&Reply {
            nonce: binding.nonce.clone(),
            sequence: message.sequence,
            response,
        })?;
        Ok(None)
    }
    pub(super) fn closed(pipe: &Pipe, sequence: u64, closed: Closed) -> io::Result<()> {
        pipe.send(&Reply {
            nonce: closed.nonce.clone(),
            sequence,
            response: Response::Closed(closed),
        })
    }
}

#[cfg(test)]
#[path = "windows_station_debugger_tests.rs"]
mod tests;
