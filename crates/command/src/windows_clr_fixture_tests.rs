//! 固定 Framework 异常夹具；只证明原调试停点读取及 reader 回收，不代表 G09 通过。

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::os::windows::io::{AsHandle as _, AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use windows::Win32::Foundation::{
    DBG_CONTINUE, DBG_EXCEPTION_NOT_HANDLED, ERROR_SEM_TIMEOUT, EXCEPTION_BREAKPOINT, HANDLE,
    NTSTATUS, WAIT_OBJECT_0,
};
use windows::Win32::System::Diagnostics::Debug::{
    CREATE_PROCESS_DEBUG_EVENT, CREATE_THREAD_DEBUG_EVENT, ContinueDebugEvent, DEBUG_EVENT,
    EXCEPTION_DEBUG_EVENT, EXIT_PROCESS_DEBUG_EVENT, EXIT_THREAD_DEBUG_EVENT, LOAD_DLL_DEBUG_EVENT,
    OUTPUT_DEBUG_STRING_EVENT, RIP_EVENT, UNLOAD_DLL_DEBUG_EVENT, WaitForDebugEvent,
};
use windows::Win32::System::Threading::{
    DEBUG_ONLY_THIS_PROCESS, GetCurrentProcess, GetProcessId, GetProcessIdOfThread, GetThreadId,
    THREAD_GET_CONTEXT, THREAD_QUERY_INFORMATION, WaitForSingleObject,
};
use windows::core::HRESULT;

use super::clr_reader::{
    BoundFile, ClrExceptionStop, ClrReader, ClrReaderImage, ClrRuntimeBinding, Job, birth,
    duplicate, empty_environment, file_identity, hex, image_path, information, put64, raw,
    remaining, request, require,
};
use crate::blocking::Command;

const TIMEOUT: Duration = Duration::from_secs(30);
const CLEANUP_RESERVE: Duration = Duration::from_secs(5);
const CLR_EXCEPTION: u32 = 0xe0434352;

fn save(path: &Path, value: &Value) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()
}

struct Fixture {
    child: Child,
    job: Job,
    threads: HashMap<u32, OwnedHandle>,
    main_thread: Option<u32>,
    clr: Option<ClrRuntimeBinding>,
    exited: bool,
    exit_code: Option<u32>,
    sequence: u64,
    observations: Vec<Value>,
    events: Vec<Value>,
    image_identity: (u32, u32, u32, u32, u32),
    cleanup_deadline: Instant,
    active_reader: Option<ClrReader>,
    pending: Option<(DEBUG_EVENT, NTSTATUS)>,
    termination_requested: bool,
}

impl Fixture {
    fn start(image: &mut BoundFile, root: &Path, cleanup_deadline: Instant) -> io::Result<Self> {
        image.verify()?;
        let mut command = Command::new_with_managed_process_group(&image.path);
        empty_environment(&mut command, root)?;
        // CREATE 调试事件在任何用户态执行前发生；只在该停点入 Job 后 Continue。
        // 不叠加 CREATE_SUSPENDED，避免等待尚未获调度的主线程发出创建事件。
        command
            .creation_flags(DEBUG_ONLY_THIS_PROCESS.0)
            .stdin(Stdio::null())
            .stdout(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(root.join("fixture.stdout"))?,
            )
            .stderr(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(root.join("fixture.stderr"))?,
            );
        let job = Job::new()?;
        let child = command.spawn()?;
        Ok(Self {
            child,
            job,
            threads: HashMap::new(),
            main_thread: None,
            clr: None,
            exited: false,
            exit_code: None,
            sequence: 0,
            observations: vec![],
            events: vec![],
            image_identity: image.identity,
            cleanup_deadline,
            active_reader: None,
            pending: None,
            termination_requested: false,
        })
    }

    fn bind_thread(&mut self, thread: HANDLE, id: u32) -> io::Result<()> {
        require(
            unsafe { GetThreadId(thread) } == id
                && unsafe { GetProcessIdOfThread(thread) } == self.child.id(),
            "原调试线程身份不匹配",
        )?;
        require(!self.threads.contains_key(&id), "重复调试线程")?;
        let handle = duplicate(
            thread,
            unsafe { GetCurrentProcess() },
            (THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION).0,
        )?;
        self.threads
            .insert(id, unsafe { OwnedHandle::from_raw_handle(handle.0) });
        Ok(())
    }

    fn capture(
        &mut self,
        event: &DEBUG_EVENT,
        image: &mut ClrReaderImage,
        root: &Path,
        deadline: Instant,
    ) -> io::Result<()> {
        require(self.observations.len() < 16, "CLR 夹具异常停点超过上限")?;
        require(self.active_reader.is_none(), "上次 reader 尚未释放")?;
        let thread = self
            .threads
            .get(&event.dwThreadId)
            .ok_or_else(|| io::Error::other("CLR 异常线程缺少原 CREATE 句柄"))?;
        let clr = self
            .clr
            .as_mut()
            .ok_or_else(|| io::Error::other("CLR 异常先于原映像绑定"))?;
        let exception = unsafe { event.u.Exception };
        require(
            exception.ExceptionRecord.NumberParameters >= 1,
            "CLR 原事件缺少 HRESULT",
        )?;
        let hresult = exception.ExceptionRecord.ExceptionInformation[0] as u32;
        self.active_reader = Some(ClrReader::start(image, root, self.sequence, deadline)?);
        let reader = self
            .active_reader
            .as_mut()
            .expect("刚创建的 reader 必须存在");
        reader.bind_and_send(
            image,
            clr,
            ClrExceptionStop {
                process: self.child.as_handle(),
                thread: thread.as_handle(),
                process_id: self.child.id(),
                thread_id: event.dwThreadId,
                event_sequence: self.sequence,
                exception_code: exception.ExceptionRecord.ExceptionCode.0 as u32,
                first_chance: exception.dwFirstChance,
                hresult,
                read_budget_bytes: 16 * 1024 * 1024,
            },
        )?;
        let result = loop {
            if let Some(result) = reader.poll(|| false)? {
                break result;
            }
        };
        clr.verify()?;
        let mut observation = reader
            .binding()
            .expect("成功读取必须保留原停点绑定")
            .clone();
        observation["main_tid"] = json!(self.main_thread);
        observation["reader"] = result;
        observation["reader_reaped"] = json!(reader.is_reaped());
        observation["reader_job_empty"] = json!(reader.job_empty_fixture()?);
        self.observations.push(observation);
        self.active_reader.take();
        Ok(())
    }

    fn release_reader(&mut self) -> io::Result<()> {
        if let Some(reader) = self.active_reader.as_mut() {
            reader.abort_and_reap(self.cleanup_deadline)?;
        }
        self.active_reader.take();
        Ok(())
    }

    fn terminate_original(&mut self) -> io::Result<()> {
        if self.termination_requested {
            return Ok(());
        }
        let _ = self.job.terminate();
        if let Err(error) = self.child.kill() {
            if unsafe { WaitForSingleObject(HANDLE(self.child.as_raw_handle()), 0) }
                != WAIT_OBJECT_0
            {
                return Err(error);
            }
        }
        self.termination_requested = true;
        Ok(())
    }

    fn continue_pending(&mut self) -> io::Result<()> {
        require(
            self.active_reader.is_none(),
            "reader 未确认退出，不能继续原停点",
        )?;
        if let Some((event, status)) = self.pending.as_ref() {
            unsafe { ContinueDebugEvent(event.dwProcessId, event.dwThreadId, *status) }
                .map_err(io::Error::from)?;
            self.pending.take();
            if let Some(last) = self.events.last_mut() {
                last["continued"] = json!(true);
            }
        }
        Ok(())
    }

    fn handle(
        &mut self,
        event: &DEBUG_EVENT,
        image: &mut ClrReaderImage,
        root: &Path,
        deadline: Instant,
        cleanup: bool,
    ) -> io::Result<()> {
        // 映像句柄由调试器关闭；进程/线程事件原句柄由 Windows 在 EXIT 继续后关闭。
        let file_handle = match event.dwDebugEventCode {
            CREATE_PROCESS_DEBUG_EVENT => Some(unsafe { event.u.CreateProcessInfo.hFile }),
            LOAD_DLL_DEBUG_EVENT => Some(unsafe { event.u.LoadDll.hFile }),
            _code => None,
        };
        let file = file_handle
            .filter(|handle| !handle.is_invalid())
            .map(|handle| unsafe { File::from_raw_handle(handle.0) });
        require(
            event.dwProcessId == self.child.id(),
            "夹具收到其他进程的调试事件",
        )?;
        if cleanup {
            if event.dwDebugEventCode == EXIT_PROCESS_DEBUG_EVENT {
                self.exit_code = Some(unsafe { event.u.ExitProcess.dwExitCode });
                self.exited = true;
                self.threads.clear();
            }
            return Ok(());
        }
        match event.dwDebugEventCode {
            CREATE_PROCESS_DEBUG_EVENT => {
                let info = unsafe { event.u.CreateProcessInfo };
                require(
                    unsafe { GetProcessId(info.hProcess) } == self.child.id(),
                    "CREATE 原进程身份不匹配",
                )?;
                require(
                    birth(info.hProcess, false)?
                        == birth(HANDLE(self.child.as_raw_handle()), false)?,
                    "CREATE 原进程出生身份不匹配",
                )?;
                require(
                    file_identity(&information(
                        file.as_ref()
                            .ok_or_else(|| io::Error::other("CREATE 缺少原映像"))?,
                    )?) == self.image_identity,
                    "CREATE 原映像与固定夹具不同",
                )?;
                self.job.assign(HANDLE(self.child.as_raw_handle()))?;
                self.bind_thread(info.hThread, event.dwThreadId)?;
                self.main_thread = Some(event.dwThreadId);
                Ok(())
            }
            CREATE_THREAD_DEBUG_EVENT => {
                self.bind_thread(unsafe { event.u.CreateThread.hThread }, event.dwThreadId)
            }
            EXIT_THREAD_DEBUG_EVENT => {
                self.threads.remove(&event.dwThreadId);
                Ok(())
            }
            LOAD_DLL_DEBUG_EVENT => {
                if let Some(file) = file {
                    let path = image_path(&file)?;
                    if path
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("clr.dll"))
                    {
                        require(self.clr.is_none(), "重复 CLR 映像")?;
                        self.clr = Some(ClrRuntimeBinding::from_load(&file, unsafe {
                            event.u.LoadDll.lpBaseOfDll
                        }
                            as u64)?);
                    }
                }
                Ok(())
            }
            EXCEPTION_DEBUG_EVENT => {
                let info = unsafe { event.u.Exception };
                if !cleanup
                    && info.dwFirstChance == 1
                    && info.ExceptionRecord.ExceptionCode.0 as u32 == CLR_EXCEPTION
                {
                    self.capture(event, image, root, deadline)?;
                }
                Ok(())
            }
            EXIT_PROCESS_DEBUG_EVENT => {
                self.exit_code = Some(unsafe { event.u.ExitProcess.dwExitCode });
                self.exited = true;
                self.threads.clear();
                Ok(())
            }
            UNLOAD_DLL_DEBUG_EVENT | OUTPUT_DEBUG_STRING_EVENT => Ok(()),
            RIP_EVENT => Err(io::Error::other("CLR 夹具发生原生调试错误")),
            _code => Err(io::Error::other("CLR 夹具出现未知调试事件")),
        }
    }

    fn pump(
        &mut self,
        reader: &mut ClrReaderImage,
        root: &Path,
        deadline: Instant,
        cleanup: bool,
    ) -> io::Result<()> {
        self.release_reader()?;
        self.continue_pending()?;
        while !self.exited {
            require(self.sequence < 4096, "CLR 夹具调试事件超过上限")?;
            let mut event = DEBUG_EVENT::default();
            match unsafe { WaitForDebugEvent(&mut event, remaining(deadline)?.min(100)) } {
                Ok(()) => {}
                Err(error) if error.code() == HRESULT::from_win32(ERROR_SEM_TIMEOUT.0) => continue,
                Err(error) => return Err(io::Error::from(error)),
            }
            self.sequence += 1;
            let status: NTSTATUS = if event.dwDebugEventCode == EXCEPTION_DEBUG_EVENT
                && unsafe { event.u.Exception.ExceptionRecord.ExceptionCode }
                    != EXCEPTION_BREAKPOINT
            {
                DBG_EXCEPTION_NOT_HANDLED
            } else {
                DBG_CONTINUE
            };
            self.pending = Some((event, status));
            let result = self.handle(&event, reader, root, deadline, cleanup);
            self.events.push(json!({"sequence":self.sequence,"code":event.dwDebugEventCode.0,"pid":event.dwProcessId,"tid":event.dwThreadId,"handled":result.is_ok(),"continued":false}));
            if let Err(error) = &result {
                if let Some(last) = self.events.last_mut() {
                    last["handling_error"] =
                        json!({"kind":format!("{:?}",error.kind()),"os_code":error.raw_os_error()});
                }
            }
            if result.is_err() {
                // 首 CREATE 绑定失败也先终止原对象，不能让未获准的用户态执行。
                self.terminate_original()?;
            }
            // 明确确认 reader 退出且 Job 空以后才能 Continue；失败保留 pending。
            self.release_reader()?;
            self.continue_pending()?;
            result?;
        }
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if !self.exited {
            let _ = self.job.terminate();
            let _ = self.child.kill();
        }
    }
}

fn validate_binding(item: &Value) -> io::Result<()> {
    let reader = &item["reader"];
    require(
        item["tid"].as_u64().is_some() && item["main_tid"].as_u64().is_some(),
        "原异常线程身份缺失",
    )?;
    require(
        item["reader_reaped"] == true && item["reader_job_empty"] == true,
        "reader 未确认回收",
    )?;
    require(
        reader["schema"] == 1 && reader["operation"] == 1,
        "reader 协议不匹配",
    )?;
    for field in [
        "event_sequence",
        "nonce",
        "pid",
        "tid",
        "process_birth",
        "thread_birth",
    ] {
        require(
            !item[field].is_null() && reader[field] == item[field],
            "reader 停点或原身份绑定不匹配",
        )?;
    }
    require(
        reader["target_identity_verified"] == true
            && reader["dac_sha256_verified"] == true
            && reader["dac_loaded"] == true,
        "reader 缺少原目标或 DAC 绑定",
    )?;
    Ok(())
}

fn validate_fixture_thread(item: &Value) -> io::Result<()> {
    require(
        item["tid"].as_u64().is_some()
            && item["main_tid"].as_u64().is_some()
            && item["tid"] != item["main_tid"],
        "固定异常线程不能冒用创建主线程",
    )
}

fn validate_observations(observations: &[Value], prepared: &Value) -> io::Result<()> {
    require(!observations.is_empty(), "没有取得真实 CLR 异常原件")?;
    let mvid = prepared["fixture_mvid"]
        .as_str()
        .ok_or_else(|| io::Error::other("缺少夹具 MVID"))?;
    let tokens = prepared["fixture_method_tokens"]
        .as_array()
        .ok_or_else(|| io::Error::other("缺少夹具 MethodDef"))?;
    require(tokens.len() == 5, "缺少独立的同 HRESULT 抛出方法")?;
    let expected_chains: [&[(&str, u32, Option<i32>)]; 4] = [
        &[(
            "System.ComponentModel.Win32Exception",
            0x80004005,
            Some(1234),
        )],
        &[(
            "System.ComponentModel.Win32Exception",
            0x80004005,
            Some(5678),
        )],
        &[
            ("System.InvalidOperationException", 0x80131509, None),
            (
                "System.ComponentModel.Win32Exception",
                0x80004005,
                Some(5678),
            ),
        ],
        &[
            ("System.ApplicationException", 0x80131600, None),
            ("System.InvalidOperationException", 0x80131509, None),
            (
                "System.ComponentModel.Win32Exception",
                0x80004005,
                Some(5678),
            ),
        ],
    ];
    let mut count = 0;
    let mut previous_sequence = None;
    for observation in observations {
        validate_binding(observation)?;
        let reader = &observation["reader"];
        let Some(frames) = reader["frames"].as_array() else {
            continue;
        };
        let fixture_frame = frames.iter().any(|frame| {
            frame["module_mvid"]
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case(mvid))
                && tokens.contains(&frame["method_token"])
                && frame["il_offsets"].as_array().is_some_and(|offsets| {
                    offsets
                        .iter()
                        .any(|offset| offset.as_u64().is_some_and(|offset| offset < 0xfffffffd))
                })
        });
        // 启动异常不能冒充固定夹具；缺少真实目标帧最终仍因四项顺序不完整失败。
        if !fixture_frame {
            continue;
        }
        validate_fixture_thread(observation)?;
        require(count < expected_chains.len(), "固定异常多于预期四次")?;
        let expected = expected_chains[count];
        require(
            reader["status"] == "observed"
                && reader["exception_source"] == "last_thrown_object_candidate"
                && reader["object_chain_complete"] == true
                && reader["tracker_complete"] == false
                && reader["exception_state_flags"] == 2,
            "当前对象链与缺失的 tracker 状态必须分别核验",
        )?;
        let sequence = observation["event_sequence"]
            .as_u64()
            .ok_or_else(|| io::Error::other("缺少原异常事件序号"))?;
        require(
            previous_sequence.is_none_or(|previous| sequence > previous),
            "固定异常事件必须按原停点严格递增",
        )?;
        previous_sequence = Some(sequence);
        require(
            frames.iter().any(|frame| {
                frame["module_mvid"]
                    .as_str()
                    .is_some_and(|value| value.eq_ignore_ascii_case(mvid))
                    && frame["method_token"] == tokens[count]
                    && frame["il_status"] == 0
                    && frame["il_offsets"].as_array().is_some_and(|offsets| {
                        offsets
                            .iter()
                            .any(|offset| offset.as_u64().is_some_and(|offset| offset < 0xfffffffd))
                    })
            }),
            "异常缺少本次独立抛出方法的真实帧",
        )?;
        let chain = reader["chain"]
            .as_array()
            .ok_or_else(|| io::Error::other("缺少实际异常对象链"))?;
        require(
            chain.len() == expected.len(),
            "实际 InnerException 链长度不符",
        )?;
        require(
            observation["event_hresult"].as_u64() == Some(u64::from(expected[0].1)),
            "原事件 HRESULT 与固定异常不同",
        )?;
        for (level, (value, (kind, code, native_code))) in chain.iter().zip(expected).enumerate() {
            require(
                value["type"] == *kind && value["hresult"].as_u64() == Some(u64::from(*code)),
                "异常对象类型或实际 HResult 不符",
            )?;
            require(
                match native_code {
                    Some(code) => value["native_error_code"].as_i64() == Some(i64::from(*code)),
                    None => value["native_error_code"].is_null(),
                },
                "原生错误码必须属于本次对象，不能用相同 HRESULT 的旧对象替代",
            )?;
            require(
                value["inner_status"]
                    == if level + 1 == expected.len() {
                        "null"
                    } else {
                        "object"
                    },
                "异常内链或真实链尾不符",
            )?;
        }
        require(reader["budget_exhausted"] == false, "固定异常读取耗尽预算")?;
        require(frames.len() <= 32, "实际托管帧超过上限")?;
        count += 1;
    }
    require(count == 4, "四次固定异常没有按顺序各读取一次")
}

fn run(root: &Path, report: &mut Value) -> io::Result<()> {
    report["stage"] = json!("input_binding");
    let prepared: Value = serde_json::from_slice(&fs::read(root.join("preparation.safe.json"))?)?;
    require(prepared["status"] == "prepared", "CLR 夹具未完成准备")?;
    let reader_file = BoundFile::open(&root.join("reader.exe"))?;
    let mut fixture_image = BoundFile::open(&root.join("fixture.exe"))?;
    require(
        prepared["reader_sha256"] == hex(&reader_file.sha)
            && prepared["fixture_sha256"] == hex(&fixture_image.sha),
        "CLR 夹具与准备摘要不符",
    )?;
    let mut reader = ClrReaderImage::bind(&reader_file.path, reader_file.sha)?;
    drop(reader_file);
    report["reader_image"] = reader.receipt();
    report["fixture_image"] = fixture_image.receipt();
    let deadline = Instant::now() + TIMEOUT;
    let active_deadline = deadline - CLEANUP_RESERVE;

    report["stage"] = json!("request_rejection");
    let mut rejected = Vec::new();
    let mut invalid_target = request(2, *uuid::Uuid::new_v4().as_bytes(), active_deadline)?;
    put64(&mut invalid_target, 64, 1);
    let truncated = request(2, *uuid::Uuid::new_v4().as_bytes(), active_deadline)?[..80].to_vec();
    for (name, bytes) in [
        ("truncated-request", truncated),
        ("block-with-target", invalid_target),
    ] {
        let mut child = ClrReader::start_fixture(&mut reader, root, name, active_deadline)?;
        let result = (|| {
            child.send_fixture(&mut reader, &bytes)?;
            require(
                child.wait_fixture()?.code() == Some(2),
                "非法或不完整请求未在入口拒绝",
            )?;
            let result = child.output_fixture()?;
            require(
                result["stage"] == "request"
                    && result["target_identity_verified"] == false
                    && result["dac_loaded"] == false
                    && result["read_bytes"] == 0
                    && result["read_calls"] == 0,
                "非法请求已越过目标读取边界",
            )?;
            Ok::<_, io::Error>(result)
        })();
        let cleanup = child.abort_and_reap(deadline);
        rejected.push(json!({"case":name,"reader":result.as_ref().ok(),"reaped":child.is_reaped(),"job_empty":child.job_empty_fixture()?}));
        report["request_rejections"] = json!(rejected);
        cleanup?;
        result?;
    }
    report["request_rejections"] = json!(rejected);

    report["stage"] = json!("reader_termination");
    let mut blocked =
        ClrReader::start_fixture(&mut reader, root, "blocked-reader", active_deadline)?;
    let result = (|| {
        blocked.send_fixture(
            &mut reader,
            &request(2, *uuid::Uuid::new_v4().as_bytes(), active_deadline)?,
        )?;
        require(blocked.remains_running_fixture(), "阻塞夹具提前退出")
    })();
    let cleanup = blocked.abort_and_reap(deadline);
    report["blocked_reader"] = json!({"reaped":blocked.is_reaped(),"job_empty":blocked.job_empty_fixture()?,"target_handles_sent":false});
    cleanup?;
    result?;
    require(
        !blocked.wait_fixture()?.success(),
        "阻塞 reader 未被精确终止",
    )?;
    drop(blocked);

    report["stage"] = json!("native_exception_capture");
    let mut fixture = Fixture::start(&mut fixture_image, root, deadline)?;
    let result = fixture.pump(&mut reader, root, active_deadline, false);
    if let Err(error) = &result {
        report["native_result_error"] =
            json!({"kind":format!("{:?}",error.kind()),"os_code":error.raw_os_error()});
    }
    if result.is_err()
        && (!fixture.exited || fixture.pending.is_some() || fixture.active_reader.is_some())
    {
        let terminated = fixture.terminate_original();
        let drained = fixture.pump(&mut reader, root, deadline, true);
        report["failure_cleanup"] = json!({"termination_requested":terminated.is_ok(),"debug_events_drained":drained.is_ok()});
    }
    report["events"] = json!(fixture.events);
    report["observations"] = json!(fixture.observations);
    report["fixture_exit"] = json!(fixture.exit_code);
    report["pending_event"] = json!(fixture.pending.is_some());
    report["reader_unreleased"] = json!(fixture.active_reader.is_some());
    report["nonfixture_observations_are_unclassified"] = json!(true);
    if let Some(clr) = fixture.clr.as_mut() {
        report["clr_image"] = clr.image.receipt();
        report["dac_image"] = clr.dac.receipt();
        report["clr_dac_file_version"] = json!(clr.version);
    }
    let wait_ms = deadline
        .saturating_duration_since(Instant::now())
        .as_millis()
        .min(u32::MAX as u128) as u32;
    let exited = unsafe { WaitForSingleObject(HANDLE(fixture.child.as_raw_handle()), wait_ms) }
        == WAIT_OBJECT_0;
    report["fixture_reaped"] = json!(
        exited
            && fixture
                .child
                .try_wait()
                .is_ok_and(|status| status.is_some())
    );
    report["fixture_job_empty"] = json!(fixture.job.empty().is_ok_and(|empty| empty));
    result?;
    report["stage"] = json!("receipt_validation");
    require(
        fixture.exit_code == Some(0)
            && report["fixture_reaped"] == true
            && report["fixture_job_empty"] == true
            && fixture.pending.is_none()
            && fixture.active_reader.is_none(),
        "CLR 夹具未自然退出并回收",
    )?;
    validate_observations(&fixture.observations, &prepared)?;
    reader.verify()?;
    fixture_image.verify()?;
    report["accepted"] = json!(true);
    report["stage"] = json!("complete");
    Ok(())
}

#[test]
#[ignore = "仅固定 Windows Framework 4.8 DAC 能力夹具；不执行真实 CLI"]
fn framework_exception_chain_uses_original_event_thread_and_reaps_reader() {
    let root = PathBuf::from(
        std::env::var_os("INFINISHELL_CLR_FIXTURE_ROOT").expect("缺少专属 CLR 夹具根"),
    );
    let root = root.canonicalize().expect("CLR 夹具根不可达");
    let mut report = json!({"schema":1,"accepted":false,"g09_closed":false,"cleanup_ready":false});
    let result = run(&root, &mut report);
    if let Err(error) = &result {
        report["error"] =
            json!({"kind":format!("{:?}",error.kind()),"os_code":error.raw_os_error()});
    }
    save(&root.join("fixture.safe.json"), &report).expect("必须保留 CLR 夹具原收据");
    result.expect("固定 CLR 夹具失败；原件已保留，不代表产品修复");
}

#[test]
fn blocked_reader_request_has_no_target_handles_or_dac_path() {
    let request = request(2, [1; 16], Instant::now() + TIMEOUT).unwrap();
    assert_eq!(request.len(), 168);
    assert!(request[32..96].iter().all(|byte| *byte == 0));
    assert!(request[104..].iter().all(|byte| *byte == 0));
}

#[test]
fn exception_receipts_reject_main_thread_substitution() {
    let result = validate_fixture_thread(
        &json!({"tid":42,"main_tid":42,"reader_reaped":true,"reader_job_empty":true}),
    );
    assert!(result.is_err());
}

#[test]
fn exception_receipts_reject_missing_thread_identity() {
    assert!(validate_binding(&json!({"reader_reaped":true,"reader_job_empty":true})).is_err());
}

#[test]
fn exception_receipts_reject_a_different_birth_with_matching_ids() {
    let item = json!({"event_sequence":7,"nonce":"0101","pid":41,"tid":42,"main_tid":40,"process_birth":100,"thread_birth":200,"reader_reaped":true,"reader_job_empty":true,
        "reader":{"schema":1,"operation":1,"event_sequence":7,"nonce":"0101","pid":41,"tid":42,"process_birth":101,"thread_birth":200,"target_identity_verified":true,"dac_sha256_verified":true,"dac_loaded":true}});
    assert!(validate_binding(&item).is_err());
}

#[test]
fn exception_receipts_reject_unconfirmed_reader_cleanup() {
    let item = json!({"event_sequence":7,"nonce":"0101","pid":41,"tid":42,"main_tid":40,"process_birth":100,"thread_birth":200,"reader_reaped":true,"reader_job_empty":false,
        "reader":{"schema":1,"operation":1,"event_sequence":7,"nonce":"0101","pid":41,"tid":42,"process_birth":100,"thread_birth":200,"target_identity_verified":true,"dac_sha256_verified":true,"dac_loaded":true}});
    assert!(validate_binding(&item).is_err());
}

#[test]
fn exception_receipts_accept_matching_original_thread_birth() {
    let item = json!({"event_sequence":7,"nonce":"0101","pid":41,"tid":42,"main_tid":40,"process_birth":100,"thread_birth":200,"reader_reaped":true,"reader_job_empty":true,
        "reader":{"schema":1,"operation":1,"event_sequence":7,"nonce":"0101","pid":41,"tid":42,"process_birth":100,"thread_birth":200,"target_identity_verified":true,"dac_sha256_verified":true,"dac_loaded":true}});
    assert!(validate_binding(&item).is_ok());
}

#[test]
fn startup_main_thread_identity_remains_an_observation_not_fixture_evidence() {
    let item = json!({"event_sequence":7,"nonce":"0101","pid":41,"tid":40,"main_tid":40,"process_birth":100,"thread_birth":200,"reader_reaped":true,"reader_job_empty":true,
        "reader":{"schema":1,"operation":1,"event_sequence":7,"nonce":"0101","pid":41,"tid":40,"process_birth":100,"thread_birth":200,"target_identity_verified":true,"dac_sha256_verified":true,"dac_loaded":true}});
    assert!(validate_binding(&item).is_ok());
    assert!(validate_fixture_thread(&item).is_err());
}

fn ordered_fixed_exception_receipts() -> (Vec<Value>, Value) {
    let win32 = |code| {
        json!({"type":"System.ComponentModel.Win32Exception","hresult":0x80004005_u32,
            "native_error_code":code,"inner_status":"null"})
    };
    let operation = json!({"type":"System.InvalidOperationException","hresult":0x80131509_u32,
        "native_error_code":null,"inner_status":"object"});
    let application = json!({"type":"System.ApplicationException","hresult":0x80131600_u32,
        "native_error_code":null,"inner_status":"object"});
    let chains = [
        vec![win32(1234)],
        vec![win32(5678)],
        vec![operation.clone(), win32(5678)],
        vec![application, operation, win32(5678)],
    ];
    let observations = chains
        .into_iter()
        .enumerate()
        .map(|(index, chain)| {
            let sequence = index + 10;
            let hresult = chain[0]["hresult"].clone();
            json!({"event_sequence":sequence,"nonce":"fixture","pid":41,"tid":42,"main_tid":40,
            "process_birth":100,"thread_birth":200,"reader_reaped":true,"reader_job_empty":true,
            "event_hresult":hresult,
            "reader":{"schema":1,"operation":1,"event_sequence":sequence,"nonce":"fixture",
                "pid":41,"tid":42,"process_birth":100,"thread_birth":200,
                "target_identity_verified":true,"dac_sha256_verified":true,"dac_loaded":true,
                "status":"observed","exception_source":"last_thrown_object_candidate",
                "object_chain_complete":true,"tracker_complete":false,"exception_state_flags":2,
                "budget_exhausted":false,"chain":chain,
                "frames":[{"module_mvid":"fixture-mvid","method_token":index+1,"il_status":0,"il_offsets":[2]}]}})
        })
        .collect();
    (
        observations,
        json!({"fixture_mvid":"fixture-mvid","fixture_method_tokens":[1,2,3,4,5]}),
    )
}

#[test]
fn exception_receipts_require_current_object_with_same_hresult() {
    let (mut observations, prepared) = ordered_fixed_exception_receipts();
    assert!(validate_observations(&observations, &prepared).is_ok());
    // 两次 HRESULT 相同，沿用上一次对象的原生码仍必须失败。
    observations[1]["reader"]["chain"][0]["native_error_code"] = json!(1234);
    assert!(validate_observations(&observations, &prepared).is_err());
}

#[test]
fn exception_receipts_reject_reordered_fixed_events() {
    let (mut observations, prepared) = ordered_fixed_exception_receipts();
    observations[1]["event_sequence"] = json!(9);
    observations[1]["reader"]["event_sequence"] = json!(9);
    assert!(validate_observations(&observations, &prepared).is_err());
}

#[test]
fn exception_receipts_do_not_hide_partial_tracker_state() {
    let (mut observations, prepared) = ordered_fixed_exception_receipts();
    observations[0]["reader"]["exception_state_flags"] = json!(0);
    assert!(validate_observations(&observations, &prepared).is_err());
}

#[test]
fn exception_receipts_do_not_substitute_an_ancestor_mapping_for_the_throw_frame() {
    let (mut observations, prepared) = ordered_fixed_exception_receipts();
    let frames = observations[1]["reader"]["frames"].as_array_mut().unwrap();
    frames[0]["il_status"] = json!(0x80004002_u32);
    frames[0]["il_offsets"] = json!([]);
    frames.push(json!({"module_mvid":"fixture-mvid","method_token":5,
        "il_status":0,"il_offsets":[2]}));
    assert!(validate_observations(&observations, &prepared).is_err());
}
