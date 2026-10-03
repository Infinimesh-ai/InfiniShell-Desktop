//! 调试事件逐字段编码；不跨进程复制联合体填充字节或把远端句柄当作本地句柄。

use std::ffi::c_void;
use std::io;

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{HANDLE, NTSTATUS};
use windows::Win32::System::Diagnostics::Debug::{
    CREATE_PROCESS_DEBUG_EVENT, CREATE_THREAD_DEBUG_EVENT, DEBUG_EVENT, DEBUG_EVENT_CODE,
    EXCEPTION_DEBUG_EVENT, EXIT_PROCESS_DEBUG_EVENT, EXIT_THREAD_DEBUG_EVENT, LOAD_DLL_DEBUG_EVENT,
    OUTPUT_DEBUG_STRING_EVENT, RIP_EVENT, RIP_INFO_TYPE, UNLOAD_DLL_DEBUG_EVENT,
};
use windows::Win32::System::Threading::LPTHREAD_START_ROUTINE;
use windows::core::PSTR;

use super::super::{invalid, require};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Event {
    pub(super) sequence: u64,
    pub(super) pid: u32,
    pub(super) tid: u32,
    pub(super) code: u32,
    values: Vec<u64>,
}

fn address(value: *mut c_void) -> u64 {
    value as usize as u64
}
fn start(value: LPTHREAD_START_ROUTINE) -> u64 {
    value.map_or(0, |value| value as usize as u64)
}
fn pointer(value: u64) -> io::Result<*mut c_void> {
    Ok(usize::try_from(value).map_err(|_| invalid("调试事件地址越界"))? as *mut c_void)
}
fn number(value: u64) -> io::Result<u32> {
    u32::try_from(value).map_err(|_| invalid("调试事件数值越界"))
}
fn short(value: u64) -> io::Result<u16> {
    u16::try_from(value).map_err(|_| invalid("调试事件短整数越界"))
}
fn routine(value: u64) -> io::Result<LPTHREAD_START_ROUTINE> {
    let value = pointer(value)?;
    // 这只是远端地址元数据，接收端不会调用这个函数指针。
    Ok(if value.is_null() {
        None
    } else {
        Some(unsafe {
            std::mem::transmute::<*mut c_void, unsafe extern "system" fn(*mut c_void) -> u32>(value)
        })
    })
}

impl Event {
    pub(super) fn capture(sequence: u64, event: &DEBUG_EVENT) -> io::Result<Self> {
        let values = unsafe {
            match event.dwDebugEventCode {
                CREATE_PROCESS_DEBUG_EVENT => {
                    let value = event.u.CreateProcessInfo;
                    vec![
                        address(value.hFile.0),
                        address(value.hProcess.0),
                        address(value.hThread.0),
                        address(value.lpBaseOfImage),
                        value.dwDebugInfoFileOffset.into(),
                        value.nDebugInfoSize.into(),
                        address(value.lpThreadLocalBase),
                        start(value.lpStartAddress),
                        address(value.lpImageName),
                        value.fUnicode.into(),
                    ]
                }
                CREATE_THREAD_DEBUG_EVENT => {
                    let value = event.u.CreateThread;
                    vec![
                        address(value.hThread.0),
                        address(value.lpThreadLocalBase),
                        start(value.lpStartAddress),
                    ]
                }
                LOAD_DLL_DEBUG_EVENT => {
                    let value = event.u.LoadDll;
                    vec![
                        address(value.hFile.0),
                        address(value.lpBaseOfDll),
                        value.dwDebugInfoFileOffset.into(),
                        value.nDebugInfoSize.into(),
                        address(value.lpImageName),
                        value.fUnicode.into(),
                    ]
                }
                EXCEPTION_DEBUG_EVENT => {
                    let value = event.u.Exception;
                    require(
                        value.ExceptionRecord.NumberParameters <= 15,
                        "调试异常参数数量无效",
                    )?;
                    let record = value.ExceptionRecord;
                    let mut fields = vec![
                        record.ExceptionCode.0 as u32 as u64,
                        record.ExceptionFlags.into(),
                        address(record.ExceptionRecord.cast()),
                        address(record.ExceptionAddress),
                        record.NumberParameters.into(),
                        value.dwFirstChance.into(),
                    ];
                    fields.extend(record.ExceptionInformation.map(|value| value as u64));
                    fields
                }
                EXIT_PROCESS_DEBUG_EVENT => vec![event.u.ExitProcess.dwExitCode.into()],
                EXIT_THREAD_DEBUG_EVENT => vec![event.u.ExitThread.dwExitCode.into()],
                UNLOAD_DLL_DEBUG_EVENT => vec![address(event.u.UnloadDll.lpBaseOfDll)],
                OUTPUT_DEBUG_STRING_EVENT => {
                    let value = event.u.DebugString;
                    vec![
                        address(value.lpDebugStringData.0.cast()),
                        value.fUnicode.into(),
                        value.nDebugStringLength.into(),
                    ]
                }
                RIP_EVENT => vec![
                    event.u.RipInfo.dwError.into(),
                    event.u.RipInfo.dwType.0.into(),
                ],
                _ => return Err(invalid("调试事件类型无效")),
            }
        };
        Ok(Self {
            sequence,
            pid: event.dwProcessId,
            tid: event.dwThreadId,
            code: event.dwDebugEventCode.0,
            values,
        })
    }

    pub(super) fn restore(&self) -> io::Result<DEBUG_EVENT> {
        require(
            self.sequence > 0 && self.pid > 0 && self.tid > 0,
            "调试事件身份无效",
        )?;
        let code = DEBUG_EVENT_CODE(self.code);
        let length = match code {
            CREATE_PROCESS_DEBUG_EVENT => 10,
            CREATE_THREAD_DEBUG_EVENT | OUTPUT_DEBUG_STRING_EVENT => 3,
            LOAD_DLL_DEBUG_EVENT => 6,
            EXCEPTION_DEBUG_EVENT => 21,
            EXIT_PROCESS_DEBUG_EVENT | EXIT_THREAD_DEBUG_EVENT | UNLOAD_DLL_DEBUG_EVENT => 1,
            RIP_EVENT => 2,
            _ => return Err(invalid("调试事件类型无效")),
        };
        require(self.values.len() == length, "调试事件字段数量无效")?;
        let v = &self.values;
        let mut event = DEBUG_EVENT {
            dwDebugEventCode: code,
            dwProcessId: self.pid,
            dwThreadId: self.tid,
            ..Default::default()
        };
        unsafe {
            match code {
                CREATE_PROCESS_DEBUG_EVENT => {
                    let value = &mut event.u.CreateProcessInfo;
                    value.hFile = HANDLE(pointer(v[0])?);
                    value.hProcess = HANDLE(pointer(v[1])?);
                    value.hThread = HANDLE(pointer(v[2])?);
                    value.lpBaseOfImage = pointer(v[3])?;
                    value.dwDebugInfoFileOffset = number(v[4])?;
                    value.nDebugInfoSize = number(v[5])?;
                    value.lpThreadLocalBase = pointer(v[6])?;
                    value.lpStartAddress = routine(v[7])?;
                    value.lpImageName = pointer(v[8])?;
                    value.fUnicode = short(v[9])?;
                }
                CREATE_THREAD_DEBUG_EVENT => {
                    let value = &mut event.u.CreateThread;
                    value.hThread = HANDLE(pointer(v[0])?);
                    value.lpThreadLocalBase = pointer(v[1])?;
                    value.lpStartAddress = routine(v[2])?;
                }
                LOAD_DLL_DEBUG_EVENT => {
                    let value = &mut event.u.LoadDll;
                    value.hFile = HANDLE(pointer(v[0])?);
                    value.lpBaseOfDll = pointer(v[1])?;
                    value.dwDebugInfoFileOffset = number(v[2])?;
                    value.nDebugInfoSize = number(v[3])?;
                    value.lpImageName = pointer(v[4])?;
                    value.fUnicode = short(v[5])?;
                }
                EXCEPTION_DEBUG_EVENT => {
                    require(v[4] <= 15, "调试异常参数数量无效")?;
                    let value = &mut event.u.Exception;
                    value.ExceptionRecord.ExceptionCode = NTSTATUS(number(v[0])? as i32);
                    value.ExceptionRecord.ExceptionFlags = number(v[1])?;
                    value.ExceptionRecord.ExceptionRecord = pointer(v[2])?.cast();
                    value.ExceptionRecord.ExceptionAddress = pointer(v[3])?;
                    value.ExceptionRecord.NumberParameters = number(v[4])?;
                    value.dwFirstChance = number(v[5])?;
                    for (field, source) in value
                        .ExceptionRecord
                        .ExceptionInformation
                        .iter_mut()
                        .zip(&v[6..])
                    {
                        *field =
                            usize::try_from(*source).map_err(|_| invalid("调试异常参数越界"))?;
                    }
                }
                EXIT_PROCESS_DEBUG_EVENT => event.u.ExitProcess.dwExitCode = number(v[0])?,
                EXIT_THREAD_DEBUG_EVENT => event.u.ExitThread.dwExitCode = number(v[0])?,
                UNLOAD_DLL_DEBUG_EVENT => event.u.UnloadDll.lpBaseOfDll = pointer(v[0])?,
                OUTPUT_DEBUG_STRING_EVENT => {
                    let value = &mut event.u.DebugString;
                    value.lpDebugStringData = PSTR(pointer(v[0])?.cast());
                    value.fUnicode = short(v[1])?;
                    value.nDebugStringLength = short(v[2])?;
                }
                RIP_EVENT => {
                    event.u.RipInfo.dwError = number(v[0])?;
                    event.u.RipInfo.dwType = RIP_INFO_TYPE(number(v[1])?);
                }
                _ => return Err(invalid("调试事件类型无效")),
            }
        }
        Ok(event)
    }
}

#[cfg(test)]
#[path = "windows_station_debug_event_tests.rs"]
mod tests;
