//! 固定 Framework 异常与映像分类夹具；只证明原停点读取及 reader 回收，不代表 G09 通过。

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::os::windows::io::{AsHandle as _, AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use windows::Win32::Foundation::{
    DBG_CONTINUE, DBG_EXCEPTION_NOT_HANDLED, ERROR_SEM_TIMEOUT, EXCEPTION_BREAKPOINT, HANDLE,
    NTSTATUS, WAIT_FAILED, WAIT_OBJECT_0,
};
use windows::Win32::System::Diagnostics::Debug::{
    CREATE_PROCESS_DEBUG_EVENT, CREATE_THREAD_DEBUG_EVENT, ContinueDebugEvent, DEBUG_EVENT,
    EXCEPTION_DEBUG_EVENT, EXIT_PROCESS_DEBUG_EVENT, EXIT_THREAD_DEBUG_EVENT, LOAD_DLL_DEBUG_EVENT,
    OUTPUT_DEBUG_STRING_EVENT, RIP_EVENT, UNLOAD_DLL_DEBUG_EVENT, WaitForDebugEvent,
};
use windows::Win32::System::Threading::{
    DEBUG_PROCESS, GetCurrentProcess, GetProcessId, GetProcessIdOfThread, GetThreadId,
    PROCESS_QUERY_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, THREAD_GET_CONTEXT,
    THREAD_QUERY_INFORMATION, THREAD_SYNCHRONIZE, TerminateProcess, WaitForSingleObject,
};
use windows::core::HRESULT;

use super::clr_reader::{
    BoundFile, ClrExceptionStop, ClrNativeReturnStop, ClrReader, ClrReaderImage, ClrRuntimeBinding,
    Job, birth, duplicate, empty_environment, file_identity, hex, image_path, information, put64,
    raw, remaining, request, require,
};
use super::{ShellClassificationObservation, ShellClassificationWitness};
use crate::blocking::Command;

const TIMEOUT: Duration = Duration::from_secs(30);
const CLEANUP_RESERVE: Duration = Duration::from_secs(5);
const CLR_EXCEPTION: u32 = 0xe0434352;
const MAX_DESCENDANTS: usize = 16;

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
    node_image: BoundFile,
    descendants: HashMap<u32, OwnedHandle>,
    descendant_creates: HashMap<u32, Value>,
    descendant_exits: HashMap<u32, u32>,
    node_created: Option<Value>,
    shell32: Option<BoundFile>,
    shell: Option<ShellClassificationWitness>,
    shell_skips: Vec<Value>,
    shell_entries: Vec<Value>,
    shell_observation: Option<Value>,
    pre_node_shell_entries: Vec<Value>,
    pre_node_shell_observation: Option<Value>,
    initial_breakpoints: HashSet<u32>,
    handling_stage: &'static str,
    continuation_stage: &'static str,
}

impl Fixture {
    fn start(
        image: &mut BoundFile,
        mut node_image: BoundFile,
        root: &Path,
        cleanup_deadline: Instant,
    ) -> io::Result<Self> {
        image.verify()?;
        node_image.verify()?;
        let mut command = Command::new_with_managed_process_group(&image.path);
        empty_environment(&mut command, root)?;
        // CREATE 调试事件在任何用户态执行前发生；只在该停点入 Job 后 Continue。
        // 不叠加 CREATE_SUSPENDED，避免等待尚未获调度的主线程发出创建事件。
        command
            .creation_flags(DEBUG_PROCESS.0)
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
            node_image,
            descendants: HashMap::new(),
            descendant_creates: HashMap::new(),
            descendant_exits: HashMap::new(),
            node_created: None,
            shell32: None,
            shell: None,
            shell_skips: vec![],
            shell_entries: vec![],
            shell_observation: None,
            pre_node_shell_entries: vec![],
            pre_node_shell_observation: None,
            initial_breakpoints: HashSet::new(),
            handling_stage: "not_started",
            continuation_stage: "not_started",
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
            (THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION | THREAD_SYNCHRONIZE).0,
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

    fn capture_native_return(
        &mut self,
        event: &DEBUG_EVENT,
        receipt: Value,
        pre_node: bool,
        image: &mut ClrReaderImage,
        root: &Path,
        deadline: Instant,
    ) -> io::Result<()> {
        let observation = if pre_node {
            &mut self.pre_node_shell_observation
        } else {
            &mut self.shell_observation
        };
        require(observation.is_none(), "固定分类每阶段只能返回一次")?;
        require(self.active_reader.is_none(), "上次 reader 尚未释放")?;
        *observation = Some(json!({"classification":receipt,"main_tid":self.main_thread}));
        let shell = self
            .shell
            .as_ref()
            .ok_or_else(|| io::Error::other("分类观察器缺失"))?;
        require(
            !shell.requires_restoration(),
            "分类返回后仍有未恢复调试寄存器",
        )?;
        observation.as_mut().expect("刚记录的分类返回必须存在")["live_threads_restored_before_reader"] =
            json!(true);
        let clr = self
            .clr
            .as_mut()
            .ok_or_else(|| io::Error::other("分类返回缺少原 CLR 映像"))?;
        self.active_reader = Some(ClrReader::start_native_return(
            image,
            root,
            self.sequence,
            deadline,
        )?);
        let reader = self
            .active_reader
            .as_mut()
            .expect("刚创建的 reader 必须存在");
        let exception = unsafe { event.u.Exception };
        reader.bind_native_return_and_send(
            image,
            clr,
            ClrNativeReturnStop {
                process: shell.process_handle(),
                thread: shell.thread_handle(event.dwThreadId)?,
                process_id: event.dwProcessId,
                thread_id: event.dwThreadId,
                event_sequence: self.sequence,
                exception_code: exception.ExceptionRecord.ExceptionCode.0 as u32,
                first_chance: exception.dwFirstChance,
                hresult: 0,
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
            .expect("成功读取必须保留原返回停点绑定")
            .clone();
        observation["main_tid"] = json!(self.main_thread);
        observation["classification"] = receipt;
        observation["live_threads_restored_before_reader"] = json!(true);
        observation["reader"] = result;
        observation["reader_reaped"] = json!(reader.is_reaped());
        observation["reader_job_empty"] = json!(reader.job_empty_fixture()?);
        require(
            reader.is_reaped() && reader.job_empty_fixture()?,
            "分类读取器尚未完全回收",
        )?;
        if pre_node {
            self.pre_node_shell_observation = Some(observation);
        } else {
            self.shell_observation = Some(observation);
        }
        self.active_reader.take();
        if pre_node {
            remaining(deadline)?;
            self.shell
                .as_mut()
                .expect("原分类观察器必须保留")
                .resume_after_pre_node_return(event, self.sequence)?;
            self.pre_node_shell_observation
                .as_mut()
                .expect("启动前收据必须保留")["rearmed_after_reader_reaped"] = json!(true);
        }
        Ok(())
    }

    fn handle_descendant(
        &mut self,
        event: &DEBUG_EVENT,
        file: Option<&mut File>,
        cleanup: bool,
    ) -> io::Result<()> {
        self.handling_stage = "descendant_event";
        match event.dwDebugEventCode {
            CREATE_PROCESS_DEBUG_EVENT => {
                self.handling_stage = "descendant_create_binding";
                let info = unsafe { event.u.CreateProcessInfo };
                require(
                    !self.descendants.contains_key(&event.dwProcessId),
                    "重复子进程 CREATE",
                )?;
                let handle = duplicate(
                    info.hProcess,
                    unsafe { GetCurrentProcess() },
                    (PROCESS_QUERY_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE).0,
                )?;
                self.descendants.insert(event.dwProcessId, unsafe {
                    OwnedHandle::from_raw_handle(handle.0)
                });
                self.descendant_creates.insert(event.dwProcessId, json!({
                    "pid":event.dwProcessId,"tid":event.dwThreadId,"sequence":self.sequence,
                    "birth":null,"process_id_matched":unsafe { GetProcessId(info.hProcess) } == event.dwProcessId,
                    "thread_process_id_matched":unsafe { GetProcessIdOfThread(info.hThread) } == event.dwProcessId,
                    "thread_id_matched":unsafe { GetThreadId(info.hThread) } == event.dwThreadId,
                    "image":null,"image_present":file.is_some(),"path_matched":null,
                    "file_identity_matched":null,"sha256_matched":null,"role":"unclassified",
                    "cleanup_create":cleanup,"exit_confirmed":false
                }));
                if cleanup {
                    // 收尾时新出现的原 CREATE 也保留句柄并精确终止，不依赖它已进入根 Job。
                    let _ = unsafe { TerminateProcess(handle, 1) };
                }
                let binding = self
                    .descendant_creates
                    .get_mut(&event.dwProcessId)
                    .expect("刚保留的原 CREATE 收据必须存在");
                let created = birth(handle, false)?;
                binding["birth"] = json!(created);
                validate_descendant_create(binding, false)?;
                // 正常执行限制 16 个后代；失败收尾仍保留新句柄，并受原 4096 事件总上限约束。
                require(
                    cleanup || self.descendants.len() <= MAX_DESCENDANTS,
                    "固定夹具后代数量超过资源上限",
                )?;
                if cleanup {
                    return Ok(());
                }
                self.node_image.verify()?;
                if let Some(file) = file {
                    let image = (|| -> io::Result<()> {
                        let identity = file_identity(&information(file)?);
                        binding["file_identity_matched"] =
                            json!(identity == self.node_image.identity);
                        binding["image"] = json!({"file_identity":identity,"sha256":null});
                        binding["path_matched"] = json!(image_path(file)? == self.node_image.path);
                        let digest = BoundFile::digest(file)?;
                        binding["sha256_matched"] = json!(digest == self.node_image.sha);
                        binding["image"]["sha256"] = json!(hex(&digest));
                        Ok(())
                    })();
                    if let Err(error) = image {
                        binding["image_error"] = json!({"kind":format!("{:?}",error.kind()),"os_code":error.raw_os_error()});
                    }
                }
                if !validate_descendant_create(binding, self.node_created.is_some())? {
                    return Ok(());
                }
                self.handling_stage = "node_create_binding";
                self.shell
                    .as_mut()
                    .ok_or_else(|| io::Error::other("子 CREATE 先于原根绑定"))?
                    .set_node_created(event, self.sequence, created)?;
                binding["role"] = json!("fixed_node");
                self.node_created = Some(
                    json!({"pid":event.dwProcessId,"birth":created,"sequence":self.sequence,
                    "image":self.node_image.receipt(),"original_create_bound":true}),
                );
                Ok(())
            }
            EXIT_PROCESS_DEBUG_EVENT => {
                require(
                    self.descendants.contains_key(&event.dwProcessId),
                    "子退出缺少原 CREATE",
                )?;
                require(
                    self.descendant_exits
                        .insert(event.dwProcessId, unsafe { event.u.ExitProcess.dwExitCode })
                        .is_none(),
                    "重复子退出",
                )
            }
            CREATE_THREAD_DEBUG_EVENT
            | EXIT_THREAD_DEBUG_EVENT
            | LOAD_DLL_DEBUG_EVENT
            | UNLOAD_DLL_DEBUG_EVENT
            | EXCEPTION_DEBUG_EVENT
            | OUTPUT_DEBUG_STRING_EVENT => require(
                self.descendants.contains_key(&event.dwProcessId),
                "子事件缺少原 CREATE",
            ),
            RIP_EVENT => Err(io::Error::other("子模式发生原生调试错误")),
            _code => Err(io::Error::other("子模式出现未知调试事件")),
        }
    }

    fn terminate_original(&mut self) -> io::Result<()> {
        if self.termination_requested {
            return Ok(());
        }
        let _ = self.job.terminate();
        for child in self.descendants.values() {
            if unsafe { WaitForSingleObject(raw(child), 0) } != WAIT_OBJECT_0 {
                let _ = unsafe { TerminateProcess(raw(child), 1) };
            }
        }
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
        let result = (|| {
            self.continuation_stage = "reader_release_check";
            require(
                self.active_reader.is_none(),
                "reader 未确认退出，不能继续原停点",
            )?;
            if let Some((event, status)) = self.pending {
                self.continuation_stage = "continue_debug_event";
                unsafe { ContinueDebugEvent(event.dwProcessId, event.dwThreadId, status) }
                    .map_err(io::Error::from)?;
                self.pending.take();
                if let Some(last) = self.events.last_mut() {
                    last["continued"] = json!(true);
                    last["continue_status"] = json!(status.0 as u32);
                }
                if event.dwProcessId == self.child.id() {
                    if event.dwDebugEventCode == EXIT_THREAD_DEBUG_EVENT {
                        if let Some(thread) = self.threads.get(&event.dwThreadId) {
                            self.continuation_stage = "root_thread_exit_wait";
                            let waited = unsafe {
                                WaitForSingleObject(raw(thread), remaining(self.cleanup_deadline)?)
                            };
                            let error = (waited == WAIT_FAILED).then(io::Error::last_os_error);
                            if let Some(last) = self.events.last_mut() {
                                last["exit_wait"] = json!({"stage":self.continuation_stage,"result":waited.0,
                                "os_code":error.as_ref().and_then(io::Error::raw_os_error)});
                            }
                            if let Some(error) = error {
                                return Err(error);
                            }
                            require(waited == WAIT_OBJECT_0, "原线程 EXIT 继续后尚未退出")?;
                            if let Some(shell) = self.shell.as_mut() {
                                self.continuation_stage = "shell_thread_exit_confirmation";
                                shell.thread_exited(&event)?;
                            }
                        }
                        self.threads.remove(&event.dwThreadId);
                    } else if event.dwDebugEventCode == EXIT_PROCESS_DEBUG_EVENT {
                        self.continuation_stage = "root_process_exit_wait";
                        let waited = unsafe {
                            WaitForSingleObject(
                                HANDLE(self.child.as_raw_handle()),
                                remaining(self.cleanup_deadline)?,
                            )
                        };
                        let error = (waited == WAIT_FAILED).then(io::Error::last_os_error);
                        if let Some(last) = self.events.last_mut() {
                            last["exit_wait"] = json!({"stage":self.continuation_stage,"result":waited.0,
                            "os_code":error.as_ref().and_then(io::Error::raw_os_error)});
                        }
                        if let Some(error) = error {
                            return Err(error);
                        }
                        require(waited == WAIT_OBJECT_0, "原进程 EXIT 继续后尚未退出")?;
                        if let Some(shell) = self.shell.as_mut() {
                            self.continuation_stage = "shell_process_exit_confirmation";
                            shell.confirm_process_exit()?;
                        }
                        self.threads.clear();
                    }
                } else if event.dwDebugEventCode == EXIT_PROCESS_DEBUG_EVENT {
                    self.continuation_stage = "descendant_process_exit_wait";
                    let child = self
                        .descendants
                        .get(&event.dwProcessId)
                        .ok_or_else(|| io::Error::other("后代退出缺少原句柄"))?;
                    let waited = unsafe {
                        WaitForSingleObject(raw(child), remaining(self.cleanup_deadline)?)
                    };
                    let error = (waited == WAIT_FAILED).then(io::Error::last_os_error);
                    if let Some(last) = self.events.last_mut() {
                        last["exit_wait"] = json!({"stage":self.continuation_stage,"result":waited.0,
                        "os_code":error.as_ref().and_then(io::Error::raw_os_error)});
                    }
                    if let Some(error) = error {
                        return Err(error);
                    }
                    require(waited == WAIT_OBJECT_0, "后代 EXIT 继续后尚未退出")?;
                    self.continuation_stage = "descendant_exit_birth_confirmation";
                    let binding = self
                        .descendant_creates
                        .get_mut(&event.dwProcessId)
                        .ok_or_else(|| io::Error::other("后代退出缺少原出生收据"))?;
                    let exited_birth = birth(raw(child), false)?;
                    binding["exit_birth"] = json!(exited_birth);
                    require(
                        binding["birth"].as_u64() == Some(exited_birth),
                        "后代退出出生身份改变",
                    )?;
                    binding["exit_code"] = json!(unsafe { event.u.ExitProcess.dwExitCode });
                    binding["exit_confirmed"] = json!(true);
                }
            }
            Ok(())
        })();
        if let Err(error) = &result {
            if let Some(last) = self.events.last_mut() {
                last["continuation_error"] = json!({"stage":self.continuation_stage,
                    "kind":format!("{:?}",error.kind()),"os_code":error.raw_os_error()});
            }
        }
        result
    }

    fn handle(
        &mut self,
        event: &DEBUG_EVENT,
        image: &mut ClrReaderImage,
        root: &Path,
        deadline: Instant,
        cleanup: bool,
    ) -> io::Result<()> {
        self.handling_stage = "root_event";
        // 映像句柄由调试器关闭；进程/线程事件原句柄由 Windows 在 EXIT 继续后关闭。
        let file_handle = match event.dwDebugEventCode {
            CREATE_PROCESS_DEBUG_EVENT => Some(unsafe { event.u.CreateProcessInfo.hFile }),
            LOAD_DLL_DEBUG_EVENT => Some(unsafe { event.u.LoadDll.hFile }),
            _code => None,
        };
        let mut file = file_handle
            .filter(|handle| !handle.is_invalid())
            .map(|handle| unsafe { File::from_raw_handle(handle.0) });
        if event.dwProcessId != self.child.id() {
            return self.handle_descendant(event, file.as_mut(), cleanup);
        }
        if cleanup {
            if event.dwDebugEventCode == EXIT_PROCESS_DEBUG_EVENT {
                self.exit_code = Some(unsafe { event.u.ExitProcess.dwExitCode });
                self.exited = true;
            }
            return Ok(());
        }
        match event.dwDebugEventCode {
            CREATE_PROCESS_DEBUG_EVENT => {
                self.handling_stage = "root_create_binding";
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
                let expected = self
                    .node_image
                    .path
                    .to_str()
                    .ok_or_else(|| io::Error::other("固定子模式路径不是 Unicode"))?;
                let expected = expected.strip_prefix("\\\\?\\").unwrap_or(expected);
                require(
                    expected
                        .as_bytes()
                        .first()
                        .is_some_and(u8::is_ascii_alphabetic)
                        && expected.as_bytes().get(1..3) == Some(b":\\"),
                    "固定子模式必须使用已绑定的本地盘符路径",
                )?;
                self.shell = Some(ShellClassificationWitness::new(
                    event,
                    birth(info.hProcess, false)?,
                    *uuid::Uuid::new_v4().as_bytes(),
                    expected.encode_utf16().collect(),
                )?);
                Ok(())
            }
            CREATE_THREAD_DEBUG_EVENT => {
                self.handling_stage = "thread_create_binding_and_arm";
                self.bind_thread(unsafe { event.u.CreateThread.hThread }, event.dwThreadId)?;
                let shell = self
                    .shell
                    .as_mut()
                    .ok_or_else(|| io::Error::other("线程 CREATE 缺少原根"))?;
                shell.retain_thread(event)?;
                if self.shell32.is_some() && self.shell_observation.is_none() {
                    shell.arm_all(event)?;
                }
                Ok(())
            }
            EXIT_THREAD_DEBUG_EVENT => Ok(()),
            LOAD_DLL_DEBUG_EVENT => {
                if let Some(file) = file {
                    let path = image_path(&file)?;
                    if path
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("clr.dll"))
                    {
                        self.handling_stage = "clr_load_binding";
                        require(self.clr.is_none(), "重复 CLR 映像")?;
                        self.clr = Some(ClrRuntimeBinding::from_load(&file, unsafe {
                            event.u.LoadDll.lpBaseOfDll
                        }
                            as u64)?);
                    } else if path.file_name().is_some_and(|name| {
                        name.to_string_lossy().eq_ignore_ascii_case("shell32.dll")
                    }) {
                        self.handling_stage = "shell32_load_binding_and_arm";
                        require(self.shell32.is_none(), "重复 Shell32 映像")?;
                        let system = PathBuf::from(
                            std::env::var_os("SystemRoot")
                                .ok_or_else(|| io::Error::other("缺少系统根"))?,
                        );
                        let bound = BoundFile::open(&system.join("System32/shell32.dll"))?;
                        require(
                            path == bound.path
                                && file_identity(&information(&file)?) == bound.identity,
                            "Shell32 原 LOAD 与固定系统映像不符",
                        )?;
                        let shell = self
                            .shell
                            .as_mut()
                            .ok_or_else(|| io::Error::other("Shell32 LOAD 缺少原根"))?;
                        shell.bind_shell32(event, &file, bound.sha)?;
                        self.shell32 = Some(bound);
                        shell.arm_all(event)?;
                    }
                }
                Ok(())
            }
            EXCEPTION_DEBUG_EVENT => {
                self.handling_stage = "shell_observe";
                self.shell
                    .as_mut()
                    .ok_or_else(|| io::Error::other("异常停点缺少原根"))?
                    .sample_post_node_root_stop(event, self.sequence)?;
                let before = self.shell.as_ref().map(ShellClassificationWitness::summary);
                let observation = self
                    .shell
                    .as_mut()
                    .ok_or_else(|| io::Error::other("异常停点缺少原根"))?
                    .observe(event, self.sequence)?;
                match observation {
                    ShellClassificationObservation::NotOwned => (),
                    ShellClassificationObservation::OwnedSkipped => {
                        self.pending.as_mut().expect("当前停点必须存在").1 = DBG_CONTINUE;
                        let before = before.expect("跳过入口必须有原根");
                        let after = self.shell.as_ref().expect("原根必须保留").summary();
                        let thread = self
                            .threads
                            .get(&event.dwThreadId)
                            .ok_or_else(|| io::Error::other("跳过入口缺少原线程"))?;
                        self.shell_skips.push(json!({
                            "sequence":self.sequence,"pid":event.dwProcessId,"tid":event.dwThreadId,
                            "thread_birth":birth(raw(thread), true)?,
                            "node_created_before_event":self.node_created.is_some(),
                            "resume_flag_writes_before":before.resume_flag_writes,
                            "resume_flag_writes_after":after.resume_flag_writes,
                            "fixed_eflags_api_readbacks_before":before.fixed_eflags_api_readbacks,
                            "fixed_eflags_api_readbacks_after":after.fixed_eflags_api_readbacks,
                        }));
                        return Ok(());
                    }
                    ShellClassificationObservation::OwnedEntry => {
                        self.pending.as_mut().expect("当前停点必须存在").1 = DBG_CONTINUE;
                        self.shell_entries
                            .push(json!({"sequence":self.sequence,"tid":event.dwThreadId}));
                        require(self.shell_entries.len() == 1, "固定分类调用入口不唯一")?;
                        return Ok(());
                    }
                    ShellClassificationObservation::OwnedPreNodeEntry => {
                        self.pending.as_mut().expect("当前停点必须存在").1 = DBG_CONTINUE;
                        self.pre_node_shell_entries
                            .push(json!({"sequence":self.sequence,"tid":event.dwThreadId}));
                        require(
                            self.pre_node_shell_entries.len() == 1,
                            "固定启动前入口不唯一",
                        )?;
                        return Ok(());
                    }
                    ShellClassificationObservation::OwnedPreNodeReturn(receipt) => {
                        self.pending.as_mut().expect("当前停点必须存在").1 = DBG_CONTINUE;
                        self.handling_stage = "pre_node_return_reader";
                        return self.capture_native_return(
                            event,
                            serde_json::to_value(receipt)?,
                            true,
                            image,
                            root,
                            deadline,
                        );
                    }
                    ShellClassificationObservation::OwnedReturn(receipt) => {
                        self.pending.as_mut().expect("当前停点必须存在").1 = DBG_CONTINUE;
                        self.handling_stage = "native_return_reader";
                        return self.capture_native_return(
                            event,
                            serde_json::to_value(receipt)?,
                            false,
                            image,
                            root,
                            deadline,
                        );
                    }
                }
                let info = unsafe { event.u.Exception };
                if !cleanup
                    && info.dwFirstChance == 1
                    && info.ExceptionRecord.ExceptionCode.0 as u32 == CLR_EXCEPTION
                {
                    self.handling_stage = "exception_reader";
                    self.capture(event, image, root, deadline)?;
                }
                Ok(())
            }
            EXIT_PROCESS_DEBUG_EVENT => {
                self.exit_code = Some(unsafe { event.u.ExitProcess.dwExitCode });
                self.exited = true;
                Ok(())
            }
            UNLOAD_DLL_DEBUG_EVENT => {
                if let Some(shell) = self.shell.as_mut() {
                    if shell.bound_base() == Some(unsafe { event.u.UnloadDll.lpBaseOfDll } as u64) {
                        shell.unload(event)?;
                    }
                }
                Ok(())
            }
            OUTPUT_DEBUG_STRING_EVENT => Ok(()),
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
        let mut first_error = None;
        // 所有提前返回都先汇回此处；后续等待或清理错误不能替换已经记录的首错。
        let outcome = (|| {
            self.handling_stage = "reader_release";
            self.release_reader()?;
            if let Err(error) = self.continue_pending() {
                self.handling_stage = self.continuation_stage;
                if !cleanup || self.pending.is_some() {
                    return Err(error);
                }
                first_error = Some((error, self.continuation_stage));
            }
            while !self.exited || self.descendant_exits.len() != self.descendants.len() {
                self.handling_stage = "wait_debug_event";
                require(self.sequence < 4096, "CLR 夹具调试事件超过上限")?;
                let mut event = DEBUG_EVENT::default();
                match unsafe { WaitForDebugEvent(&mut event, remaining(deadline)?.min(100)) } {
                    Ok(()) => {}
                    Err(error) if error.code() == HRESULT::from_win32(ERROR_SEM_TIMEOUT.0) => {
                        continue;
                    }
                    Err(error) => return Err(io::Error::from(error)),
                }
                self.sequence += 1;
                let exception = (event.dwDebugEventCode == EXCEPTION_DEBUG_EVENT)
                    .then(|| unsafe { event.u.Exception });
                let status: NTSTATUS = if let Some(exception) = exception {
                    if exception.ExceptionRecord.ExceptionCode == EXCEPTION_BREAKPOINT
                        && exception.dwFirstChance == 1
                        && (event.dwProcessId == self.child.id()
                            || self.descendants.contains_key(&event.dwProcessId))
                        && self.initial_breakpoints.insert(event.dwProcessId)
                    {
                        DBG_CONTINUE
                    } else {
                        DBG_EXCEPTION_NOT_HANDLED
                    }
                } else {
                    DBG_CONTINUE
                };
                self.pending = Some((event, status));
                let result = self.handle(&event, reader, root, deadline, cleanup);
                let result_stage = self.handling_stage;
                let handling_failed = result.is_err();
                self.events.push(json!({"sequence":self.sequence,"code":event.dwDebugEventCode.0,"pid":event.dwProcessId,"tid":event.dwThreadId,
                "exception_code":exception.map(|value| value.ExceptionRecord.ExceptionCode.0 as u32),
                "first_chance":exception.map(|value| value.dwFirstChance),
                "continue_status":self.pending.as_ref().map(|pending| pending.1.0 as u32),
                "handling_stage":self.handling_stage,"shell_state":self.shell.as_ref().map(|shell| shell.summary()),
                "handled":result.is_ok(),"continued":false}));
                if let Err(error) = &result {
                    if let Some(last) = self.events.last_mut() {
                        last["handling_error"] = json!({"kind":format!("{:?}",error.kind()),"os_code":error.raw_os_error()});
                    }
                }
                if let Err(error) = result {
                    // handle 先于恢复、终止、reader 释放及 Continue，必须先保留它的原始失败。
                    if first_error.is_none() {
                        first_error = Some((error, result_stage));
                    }
                    if event.dwProcessId == self.child.id()
                        && event.dwDebugEventCode != EXIT_PROCESS_DEBUG_EVENT
                    {
                        if let Some(shell) = self
                            .shell
                            .as_mut()
                            .filter(|shell| shell.requires_restoration())
                        {
                            let restored = shell.withdraw_all(&event);
                            if let Some(last) = self.events.last_mut() {
                                last["failure_register_restoration"] = json!(restored.is_ok());
                            }
                        }
                    }
                    // 首 CREATE 绑定失败也先终止原对象，不能让未获准的用户态执行。
                    self.terminate_original()?;
                }
                // 明确确认 reader 退出且 Job 空以后才能 Continue；失败保留 pending。
                self.handling_stage = "reader_release";
                self.release_reader()?;
                if let Err(error) = self.continue_pending() {
                    self.handling_stage = self.continuation_stage;
                    if !cleanup || self.pending.is_some() {
                        return Err(error);
                    }
                    // 已继续的 EXIT 确认失败不能让其他原后代失去 drain；最终仍返回首次失败。
                    if first_error.is_none() {
                        first_error = Some((error, self.continuation_stage));
                    }
                }
                if handling_failed && !cleanup {
                    break;
                }
            }
            Ok(())
        })();
        match first_error {
            Some((error, stage)) => {
                self.handling_stage = stage;
                Err(error)
            }
            None => outcome,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if !self.exited || self.descendant_exits.len() != self.descendants.len() {
            let _ = self.job.terminate();
            let _ = self.child.kill();
        }
    }
}

fn validate_descendant_create(binding: &Value, node_already_bound: bool) -> io::Result<bool> {
    require(
        binding["pid"].as_u64().is_some_and(|value| value != 0)
            && binding["tid"].as_u64().is_some_and(|value| value != 0)
            && binding["birth"].as_u64().is_some_and(|value| value != 0)
            && binding["process_id_matched"] == true
            && binding["thread_process_id_matched"] == true
            && binding["thread_id_matched"] == true,
        "后代原 CREATE 的进程、线程或出生身份不符",
    )?;
    let fixed_node = binding["path_matched"] == true
        && binding["file_identity_matched"] == true
        && binding["sha256_matched"] == true;
    require(
        binding["path_matched"] != true || fixed_node,
        "固定子模式路径的文件身份或摘要不匹配",
    )?;
    require(
        !fixed_node || !node_already_bound,
        "固定无害子模式出现第二个匹配 CREATE",
    )?;
    Ok(fixed_node)
}

fn validate_descendant_receipts(report: &Value) -> io::Result<()> {
    let creates = report["descendant_creates"]
        .as_object()
        .ok_or_else(|| io::Error::other("后代原 CREATE 收据缺失"))?;
    require(
        !creates.is_empty()
            && creates.len() <= MAX_DESCENDANTS
            && report["all_descendants_reaped"] == true,
        "后代数量越界或未全部退出回收",
    )?;
    let mut fixed_count = 0;
    for binding in creates.values() {
        let fixed_node = validate_descendant_create(binding, fixed_count != 0)?;
        require(
            binding["exit_confirmed"] == true
                && binding["exit_birth"] == binding["birth"]
                && binding["cleanup_create"] == false,
            "后代未自然完成原退出确认",
        )?;
        if fixed_node {
            require(
                binding["role"] == "fixed_node"
                    && binding["pid"] == report["node_created"]["pid"]
                    && binding["birth"] == report["node_created"]["birth"]
                    && binding["sequence"] == report["node_created"]["sequence"]
                    && binding["exit_code"] == 0,
                "固定子模式与唯一匹配 CREATE 或退出身份不符",
            )?;
            fixed_count += 1;
        } else {
            require(
                binding["role"] == "unclassified",
                "非目标后代不能记为固定子模式",
            )?;
        }
    }
    require(fixed_count == 1, "缺少唯一固定子模式原 CREATE")
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
                && tokens[..4].contains(&frame["method_token"])
                && frame["il_offsets"].as_array().is_some_and(|offsets| {
                    offsets
                        .iter()
                        .any(|offset| offset.as_u64().is_some_and(|offset| offset < 0xfffffffd))
                })
        });
        // 分类调用及其 Exercise 祖先不能冒充四个抛出方法；缺目标帧仍因四项不完整失败。
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

fn validate_native_return_reader(item: &Value, prepared: &Value) -> io::Result<()> {
    let reader = &item["reader"];
    let receipt = &item["classification"];
    validate_fixture_thread(item)?;
    require(
        item["reader_reaped"] == true && item["reader_job_empty"] == true,
        "分类 reader 未确认回收",
    )?;
    require(
        reader["schema"] == 1
            && reader["operation"] == 3
            && reader["status"] == "observed"
            && reader["target_identity_verified"] == true
            && reader["dac_sha256_verified"] == true
            && reader["dac_loaded"] == true
            && reader["stack_api_hresult"] == 0
            && reader["budget_exhausted"] == false,
        "分类 reader 未完整读取原目标与 DAC",
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
            !item[field].is_null() && item[field] == reader[field],
            "分类 reader 原停点身份不匹配",
        )?;
    }
    require(
        item["event_hresult"] == 0
            && reader["event_hresult"] == 0
            && reader["exception_source"] == "none"
            && reader["exception_api_hresult"] == 0x8000000a_u32
            && reader["chain"] == json!([])
            && reader["object_chain_complete"] == false
            && reader["tracker_complete"] == false
            && reader["exception_state_flags"] == 0,
        "分类返回不能借用旧异常对象",
    )?;
    for (field, identity) in [
        ("pid", "process_id"),
        ("tid", "thread_id"),
        ("process_birth", "process_birth"),
        ("thread_birth", "thread_birth"),
    ] {
        require(
            receipt["identity"][identity] == item[field],
            "分类 entry/return 与 reader 身份不符",
        )?;
    }
    let mvid = prepared["fixture_mvid"]
        .as_str()
        .ok_or_else(|| io::Error::other("缺少夹具 MVID"))?;
    let token = prepared["fixture_shell_method_token"]
        .as_u64()
        .ok_or_else(|| io::Error::other("缺少分类方法 MethodDef"))?;
    let frames = reader["frames"]
        .as_array()
        .ok_or_else(|| io::Error::other("分类 CLR 帧缺失"))?;
    for (index, frame) in frames.iter().enumerate() {
        let offsets = frame["il_offsets"]
            .as_array()
            .ok_or_else(|| io::Error::other("分类 CLR 帧的 IL 列表缺失"))?;
        let metadata_frame = frame["frame_kind"] == "metadata_method"
            && frame["method_token"]
                .as_u64()
                .is_some_and(|token| token > 0x06000000 && token <= 0x06ffffff)
            && frame["context_hresult"] == 0
            && frame["il_status"] == 0
            && !offsets.is_empty()
            && offsets.len() <= 8
            && frame["il_offsets_needed"].as_u64() == Some(offsets.len() as u64)
            && offsets.iter().all(|offset| {
                offset
                    .as_u64()
                    .is_some_and(|offset| offset <= u32::MAX as u64)
            });
        // 只允许已由同一 MethodInstance 完整名称确认的首个 P/Invoke 帧无 metadata。
        // 保留该帧的原始 E_FAIL；后方正确的 caller 不能掩盖未知帧或其它映射失败。
        let runtime_frame = index == 0
            && frame["frame_kind"] == "runtime_pinvoke_stub"
            && matches!(
                frame["runtime_name"].as_str(),
                Some("domain_bound_pinvoke_stub" | "domain_neutral_pinvoke_stub")
            )
            && frame["method_token"] == 0x06000000_u32
            && frame["simple_frame_type"] == 2
            && frame["detailed_frame_type"] == 0
            && frame["name_hresult"] == 0
            && frame["name_complete"] == true
            && frame["name_units_needed"]
                .as_u64()
                .is_some_and(|needed| (1..=512).contains(&needed))
            && frame["context_hresult"] == 0
            && frame["mapping_hresult"] == 0x80004005_u32
            && frame["il_status"] == 0x80004005_u32
            && offsets.is_empty()
            && frame["il_offsets_needed"] == 0;
        require(
            metadata_frame || runtime_frame,
            "分类 CLR 栈含未知帧、不完整映射或伪造的 runtime IL",
        )?;
    }
    require(
        frames.len() <= 32
            && frames.iter().any(|frame| {
                frame["module_mvid"]
                    .as_str()
                    .is_some_and(|value| value.eq_ignore_ascii_case(mvid))
                    && frame["method_token"].as_u64() == Some(token)
                    && frame["il_status"] == 0
                    && frame["il_offsets"].as_array().is_some_and(|offsets| {
                        offsets
                            .iter()
                            .any(|offset| offset.as_u64().is_some_and(|offset| offset < 0xfffffffd))
                    })
            }),
        "原分类工作线程缺少精确夹具托管帧",
    )
}

fn validate_native_return(report: &Value, prepared: &Value) -> io::Result<()> {
    validate_descendant_receipts(report)?;
    let item = &report["shell_observation"];
    let receipt = &item["classification"];
    let node = &report["node_created"];
    let summary = &report["shell_classification"];
    let entries = report["shell_entries"]
        .as_array()
        .ok_or_else(|| io::Error::other("分类入口记录缺失"))?;
    validate_native_return_reader(item, prepared)?;
    let sequence = |value: &Value| {
        value
            .as_u64()
            .filter(|value| *value != 0)
            .ok_or_else(|| io::Error::other("分类事件序号缺失"))
    };
    require(
        entries.len() == 1
            && entries[0]["sequence"] == receipt["entry_sequence"]
            && entries[0]["tid"] == item["tid"]
            && receipt["return_sequence"] == item["event_sequence"]
            && sequence(&node["sequence"])? < sequence(&receipt["entry_sequence"])?
            && sequence(&receipt["entry_sequence"])? < sequence(&receipt["return_sequence"])?
            && receipt["node_create_sequence"] == node["sequence"]
            && receipt["node_process_id"] == node["pid"]
            && receipt["node_process_birth"] == node["birth"]
            && node["original_create_bound"] == true
            && report["node_exit_confirmed"] == true,
        "分类未绑定唯一原子 CREATE 与 entry/return",
    )?;
    let skips = report["shell_skips"]
        .as_array()
        .ok_or_else(|| io::Error::other("缺少固定启动前跳过入口收据"))?;
    require(skips.len() == 1, "固定启动前跳过入口不唯一")?;
    let skipped = &skips[0];
    let first_skipped = &summary["first_skipped_entry"];
    let resume_before = skipped["resume_flag_writes_before"].as_u64();
    let fixed_before = skipped["fixed_eflags_api_readbacks_before"].as_u64();
    let fixed_after = skipped["fixed_eflags_api_readbacks_after"].as_u64();
    require(
        skipped["node_created_before_event"] == false
            && sequence(&skipped["sequence"])? < sequence(&node["sequence"])?
            && first_skipped["sequence"] == skipped["sequence"]
            && first_skipped.get("node_create_sequence") == Some(&Value::Null)
            && first_skipped["generation"] == receipt["generation"]
            && first_skipped["identity"] == receipt["identity"]
            && first_skipped["flags"] == 0x2000
            && first_skipped["path_comparison"] == "matched"
            && skipped["pid"] == item["pid"]
            && skipped["tid"] == item["tid"]
            && skipped["thread_birth"] == item["thread_birth"]
            && skipped["resume_flag_writes_after"].as_u64()
                == resume_before.and_then(|value| value.checked_add(1))
            && resume_before.is_some()
            && fixed_before
                .zip(fixed_after)
                .is_some_and(|(before, after)| {
                    after == before || before.checked_add(1) == Some(after)
                })
            && summary["skipped_entries"] == 1
            && summary["resume_flag_writes"] == skipped["resume_flag_writes_after"]
            && summary["fixed_eflags_api_readbacks"] == skipped["fixed_eflags_api_readbacks_after"],
        "固定启动前入口未证明同一工作线程的自有 RF 写入读回",
    )?;
    require(
        receipt["expected_node_matched"] == true
            && receipt["flags"] == 0x2000
            && receipt["return_region_kind"] == "private"
            && receipt["registers_restored"] == true
            && receipt["execution_context_unchanged"] == true
            && item["live_threads_restored_before_reader"] == true
            && receipt["post_start_clr_stack_required"] == true
            && receipt["raw_return_u64"] == 0x4550
            && report["fixture_shell_return_u64"] == receipt["raw_return_u64"]
            && report["fixture_shell_return_low32"]
                .as_u64()
                .is_some_and(|value| value <= u32::MAX as u64)
            && receipt["raw_return_low32"] == report["fixture_shell_return_low32"],
        "原私有返回映射、寄存器恢复或夹具返回值不符",
    )?;
    let exited_before_restore = summary["threads_exited_before_restore"]
        .as_u64()
        .ok_or_else(|| io::Error::other("缺少原线程提前退出计数"))?;
    let expected_restoration = if exited_before_restore == 0 {
        "readback_verified"
    } else {
        "original_exit_confirmed"
    };
    require(
        summary["selected_calls"] == 1
            && summary["returned_calls"] == 1
            && summary["root_process_id"] == item["pid"]
            && summary["generation"] == receipt["generation"]
            && summary["dirty_threads"] == 0
            && summary["restoration"] == expected_restoration
            && summary["restored_threads"]
                .as_u64()
                .is_some_and(|count| count > 0)
            && summary["original_process_exit_confirmed"] == true
            && summary["stopped"] == true,
        "分类没有停止、恢复或确认原进程退出",
    )?;
    Ok(())
}

fn validate_pre_node_return(report: &Value, prepared: &Value) -> io::Result<()> {
    let item = &report["pre_node_shell_observation"];
    validate_native_return_reader(item, prepared)?;
    let receipt = &item["classification"];
    let summary = &report["shell_classification"];
    let post = &report["shell_observation"]["classification"];
    let sample = &summary["post_node_root_stop"];
    require(
        summary["post_node_root_stop_attempted"] == true
            && sample["generation"] == summary["generation"]
            && sample["identity"] == post["identity"]
            && sample["node_create_sequence"] == report["node_created"]["sequence"]
            && sample["sequence"]
                .as_u64()
                .zip(sample["node_create_sequence"].as_u64())
                .is_some_and(|(sequence, node)| node < sequence)
            && sample["sequence"]
                .as_u64()
                .zip(post["entry_sequence"].as_u64())
                .is_some_and(|(sequence, entry)| sequence <= entry)
            && sample["event_code"] == EXCEPTION_DEBUG_EVENT.0
            && sample["scope"] == "original_root_event_thread_at_this_stop_only"
            && sample["dirty"] == true
            && sample["addresses_equal"] == true
            && sample["configuration_equal"] == true,
        "固定启动后原工作线程停点未核实入口 DR 配置",
    )?;
    let entries = report["pre_node_shell_entries"]
        .as_array()
        .ok_or_else(|| io::Error::other("启动前入口收据缺失"))?;
    let entry = receipt["entry_sequence"].as_u64();
    let returned = receipt["return_sequence"].as_u64();
    let skipped = summary["first_skipped_entry"]["sequence"].as_u64();
    require(
        entries.len() == 1
            && entries[0]["sequence"] == receipt["entry_sequence"]
            && entries[0]["tid"] == item["tid"]
            && receipt["return_sequence"] == item["event_sequence"]
            && entry
                .zip(returned)
                .is_some_and(|(entry, returned)| 0 < entry && entry < returned)
            && returned
                .zip(skipped)
                .is_some_and(|(returned, skipped)| returned < skipped)
            && skipped
                .zip(report["node_created"]["sequence"].as_u64())
                .is_some_and(|(skipped, node)| skipped < node),
        "启动前返回、预算外跳过与原 Node 创建时序不符",
    )?;
    require(
        receipt.get("node_create_sequence").is_none()
            && receipt.get("node_process_id").is_none()
            && receipt.get("node_process_birth").is_none()
            && receipt.get("post_start_clr_stack_required").is_none()
            && receipt["pre_node_clr_stack_required"] == true
            && receipt["identity"] == post["identity"]
            && receipt["generation"] == post["generation"]
            && receipt["generation"] == summary["generation"]
            && receipt["expected_node_matched"] == true
            && receipt["flags"] == 0x2000
            && receipt["return_region_kind"] == "private"
            && receipt["registers_restored"] == true
            && receipt["execution_context_unchanged"] == true
            && item["live_threads_restored_before_reader"] == true
            && item["rearmed_after_reader_reaped"] == true
            && receipt["raw_return_u64"] == 0x4550
            && receipt["raw_return_low32"] == 0x4550
            && report["fixture_pre_node_return_u64"] == receipt["raw_return_u64"]
            && summary["pre_node_selected_calls"] == 1
            && summary["pre_node_returned_calls"] == 1,
        "启动前返回未独立绑定原线程、完整值、恢复或重新布点",
    )
}

fn run(root: &Path, report: &mut Value) -> io::Result<()> {
    report["stage"] = json!("input_binding");
    let prepared: Value = serde_json::from_slice(&fs::read(root.join("preparation.safe.json"))?)?;
    require(prepared["status"] == "prepared", "CLR 夹具未完成准备")?;
    let reader_file = BoundFile::open(&root.join("reader.exe"))?;
    let mut fixture_image = BoundFile::open(&root.join("fixture.exe"))?;
    let node_image = BoundFile::open(&root.join("fixture-node.exe"))?;
    require(
        prepared["reader_sha256"] == hex(&reader_file.sha)
            && prepared["fixture_sha256"] == hex(&fixture_image.sha)
            && prepared["fixture_node_sha256"] == hex(&node_image.sha)
            && node_image.sha == fixture_image.sha
            && node_image.identity != fixture_image.identity,
        "CLR 夹具与准备摘要不符",
    )?;
    let mut reader = ClrReaderImage::bind(&reader_file.path, reader_file.sha)?;
    drop(reader_file);
    report["reader_image"] = reader.receipt();
    report["fixture_image"] = fixture_image.receipt();
    report["node_image"] = node_image.receipt();
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
    let mut fixture = Fixture::start(&mut fixture_image, node_image, root, deadline)?;
    let result = fixture.pump(&mut reader, root, active_deadline, false);
    if let Err(error) = &result {
        report["native_result_error"] = json!({"stage":fixture.handling_stage,"kind":format!("{:?}",error.kind()),"os_code":error.raw_os_error()});
    }
    if result.is_err()
        && (!fixture.exited
            || fixture.pending.is_some()
            || fixture.active_reader.is_some()
            || fixture.descendant_exits.len() != fixture.descendants.len())
    {
        let terminated = fixture.terminate_original();
        let drained = fixture.pump(&mut reader, root, deadline, true);
        let drain_error = drained.as_ref().err().map(|error| {
            json!({"stage":fixture.handling_stage,"kind":format!("{:?}",error.kind()),"os_code":error.raw_os_error()})
        });
        report["failure_cleanup"] = json!({"termination_requested":terminated.is_ok(),
            "debug_events_drained":drained.is_ok(),"drain_error":drain_error});
    }
    report["events"] = json!(fixture.events);
    report["observations"] = json!(fixture.observations);
    report["shell_skips"] = json!(fixture.shell_skips);
    report["shell_entries"] = json!(fixture.shell_entries);
    report["shell_observation"] = json!(fixture.shell_observation);
    report["pre_node_shell_entries"] = json!(fixture.pre_node_shell_entries);
    report["pre_node_shell_observation"] = json!(fixture.pre_node_shell_observation);
    report["node_created"] = json!(fixture.node_created);
    report["descendant_creates"] = json!(fixture.descendant_creates);
    report["descendant_exits"] = json!(fixture.descendant_exits);
    if let Some(shell) = fixture.shell.as_ref() {
        report["shell_classification"] = serde_json::to_value(shell.summary())?;
    }
    if let Some(shell32) = fixture.shell32.as_mut() {
        shell32.verify()?;
        report["shell32_image"] = shell32.receipt();
    }
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
    report["all_descendants_reaped"] = json!(fixture.descendants.iter().all(|(pid, child)| {
        fixture.descendant_exits.contains_key(pid)
            && unsafe { WaitForSingleObject(raw(child), 0) } == WAIT_OBJECT_0
            && fixture.descendant_creates.get(pid).is_some_and(|binding| {
                binding["exit_confirmed"] == true
                    && binding["birth"].as_u64().is_some_and(|expected| {
                        expected != 0
                            && birth(raw(child), false).is_ok_and(|actual| actual == expected)
                    })
            })
    }));
    let node_pid = fixture
        .node_created
        .as_ref()
        .and_then(|node| node["pid"].as_u64())
        .and_then(|pid| u32::try_from(pid).ok());
    report["node_exit_confirmed"] = json!(node_pid.is_some_and(|pid| {
        fixture.descendants.get(&pid).is_some_and(|child| {
            fixture.descendant_exits.get(&pid) == Some(&0)
                && unsafe { WaitForSingleObject(raw(child), 0) } == WAIT_OBJECT_0
                && report["node_created"]["birth"]
                    .as_u64()
                    .is_some_and(|expected| {
                        expected != 0
                            && birth(raw(child), false).is_ok_and(|actual| actual == expected)
                    })
        })
    }));
    result?;
    report["stage"] = json!("receipt_validation");
    require(
        fixture.exit_code == Some(0)
            && report["fixture_reaped"] == true
            && report["fixture_job_empty"] == true
            && report["all_descendants_reaped"] == true
            && fixture.pending.is_none()
            && fixture.active_reader.is_none(),
        "CLR 夹具未自然退出并回收",
    )?;
    validate_observations(&fixture.observations, &prepared)?;
    let output = fs::read(root.join("fixture.stdout"))?;
    require(output.len() <= 44, "固定分类返回输出超过数值边界")?;
    let text =
        std::str::from_utf8(&output).map_err(|_| io::Error::other("固定分类返回不是 UTF-8"))?;
    let values = text
        .lines()
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| io::Error::other("固定分类返回不是完整 64 位整数"))?;
    require(values.len() == 2, "固定启动前和启动后返回数量不符")?;
    let before = values[0];
    let returned = values[1];
    require(
        output == format!("{before}\r\n{returned}\r\n").as_bytes()
            || output == format!("{before}\n{returned}\n").as_bytes(),
        "固定分类返回存在额外输出",
    )?;
    report["fixture_shell_return_u64"] = json!(returned);
    report["fixture_shell_return_low32"] = json!(returned as u32);
    report["fixture_pre_node_return_u64"] = json!(before);
    validate_native_return(report, &prepared)?;
    validate_pre_node_return(report, &prepared)?;
    reader.verify()?;
    fixture_image.verify()?;
    fixture.node_image.verify()?;
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

#[test]
fn classification_or_exercise_frames_cannot_replace_fixed_exception_methods() {
    let (mut observations, prepared) = ordered_fixed_exception_receipts();
    let mut classification = observations[0].clone();
    classification["event_sequence"] = json!(9);
    classification["reader"]["event_sequence"] = json!(9);
    classification["reader"]["frames"] = json!([
        {"module_mvid":"fixture-mvid","method_token":6,"il_status":0,"il_offsets":[2]},
        {"module_mvid":"fixture-mvid","method_token":5,"il_status":0,"il_offsets":[2]}
    ]);
    observations.insert(0, classification);
    assert!(validate_observations(&observations, &prepared).is_ok());
    observations.remove(1);
    assert!(validate_observations(&observations, &prepared).is_err());
}

fn fixed_native_return_receipt() -> (Value, Value) {
    let generation = [1_u8; 16];
    let descendant = json!({"pid":51,"tid":52,"birth":300,"sequence":8,
        "process_id_matched":true,"thread_process_id_matched":true,"thread_id_matched":true,
        "path_matched":true,"file_identity_matched":true,"sha256_matched":true,
        "role":"fixed_node","cleanup_create":false,"exit_confirmed":true,"exit_birth":300,"exit_code":0});
    let reader = json!({"schema":1,"operation":3,"status":"observed","event_sequence":11,"nonce":"return-fixture",
        "pid":41,"tid":42,"process_birth":100,"thread_birth":200,"event_hresult":0,
        "target_identity_verified":true,"dac_sha256_verified":true,"dac_loaded":true,"stack_api_hresult":0,"budget_exhausted":false,
        "exception_source":"none","exception_api_hresult":0x8000000a_u32,"chain":[],
        "object_chain_complete":false,"tracker_complete":false,"exception_state_flags":0,
        "frames":[{"module_mvid":"fixture-mvid","method_token":0x06000006_u32,
            "frame_kind":"metadata_method","context_hresult":0,
            "il_status":0,"il_offsets":[2],"il_offsets_needed":1}]});
    let classification = json!({"generation":generation,"identity":{"process_id":41,"thread_id":42,"process_birth":100,"thread_birth":200},
        "entry_sequence":10,"return_sequence":11,"node_create_sequence":8,"node_process_id":51,"node_process_birth":300,
        "expected_node_matched":true,"flags":0x2000,"raw_return_u64":17744,"raw_return_low32":17744,"return_region_kind":"private",
        "registers_restored":true,"execution_context_unchanged":true,"post_start_clr_stack_required":true});
    let observation = json!({"event_sequence":11,"nonce":"return-fixture","pid":41,"tid":42,"main_tid":40,
        "process_birth":100,"thread_birth":200,"event_hresult":0,"reader_reaped":true,"reader_job_empty":true,
        "live_threads_restored_before_reader":true,"classification":classification,"reader":reader});
    let report = json!({
        "node_created":{"pid":51,"birth":300,"sequence":8,"original_create_bound":true},
        "descendant_creates":{"51":descendant},"all_descendants_reaped":true,
        "node_exit_confirmed":true,"fixture_shell_return_u64":17744,"fixture_shell_return_low32":17744,
        "shell_skips":[{"sequence":7,"pid":41,"tid":42,"thread_birth":200,
            "node_created_before_event":false,"resume_flag_writes_before":0,"resume_flag_writes_after":1,
            "fixed_eflags_api_readbacks_before":0,"fixed_eflags_api_readbacks_after":1}],
        "shell_entries":[{"sequence":10,"tid":42}],
        "shell_classification":{"generation":generation,"root_process_id":41,"selected_calls":1,"returned_calls":1,
            "skipped_entries":1,"resume_flag_writes":1,"fixed_eflags_api_readbacks":1,
            "first_skipped_entry":{"generation":generation,
                "identity":{"process_id":41,"thread_id":42,"process_birth":100,"thread_birth":200},
                "sequence":7,"node_create_sequence":null,"flags":0x2000,"path_comparison":"matched"},
            "dirty_threads":0,"threads_exited_before_restore":0,"restoration":"readback_verified",
            "restored_threads":2,"original_process_exit_confirmed":true,"stopped":true},
        "shell_observation":observation
    });
    (
        report,
        json!({"fixture_mvid":"fixture-mvid","fixture_shell_method_token":0x06000006_u32}),
    )
}

fn fixed_native_return_with_stub_receipt() -> (Value, Value) {
    let (mut report, prepared) = fixed_native_return_receipt();
    report["shell_observation"]["reader"]["frames"]
        .as_array_mut()
        .unwrap()
        .insert(
            0,
            json!({"module_mvid":"fixture-mvid","method_token":0x06000000_u32,
                "frame_kind":"runtime_pinvoke_stub","runtime_name":"domain_bound_pinvoke_stub",
                "simple_frame_type":2,"detailed_frame_type":0,"name_hresult":0,
                "name_complete":true,"name_units_needed":86,"context_hresult":0,
                "mapping_hresult":0x80004005_u32,"il_status":0x80004005_u32,
                "il_offsets":[],"il_offsets_needed":0}),
        );
    (report, prepared)
}

fn fixed_pre_node_return_receipt() -> (Value, Value) {
    let (mut report, prepared) = fixed_native_return_receipt();
    let mut item = report["shell_observation"].clone();
    item["event_sequence"] = json!(5);
    item["reader"]["event_sequence"] = json!(5);
    item["classification"]["entry_sequence"] = json!(4);
    item["classification"]["return_sequence"] = json!(5);
    item["classification"]["pre_node_clr_stack_required"] = json!(true);
    item["rearmed_after_reader_reaped"] = json!(true);
    for field in [
        "node_create_sequence",
        "node_process_id",
        "node_process_birth",
        "post_start_clr_stack_required",
    ] {
        item["classification"]
            .as_object_mut()
            .unwrap()
            .remove(field);
    }
    report["pre_node_shell_observation"] = item;
    report["pre_node_shell_entries"] = json!([{"sequence":4,"tid":42}]);
    report["fixture_pre_node_return_u64"] = json!(17744);
    report["shell_classification"]["pre_node_selected_calls"] = json!(1);
    report["shell_classification"]["pre_node_returned_calls"] = json!(1);
    report["shell_classification"]["post_node_root_stop_attempted"] = json!(true);
    report["shell_classification"]["post_node_root_stop"] = json!({
        "generation":report["shell_classification"]["generation"],
        "identity":report["shell_observation"]["classification"]["identity"],
        "sequence":10,"node_create_sequence":8,"event_code":1,
        "scope":"original_root_event_thread_at_this_stop_only",
        "dirty":true,"addresses_equal":true,"configuration_equal":true,
    });
    (report, prepared)
}

#[test]
fn pre_node_return_requires_an_independent_pair_before_skip_and_node() {
    let (report, prepared) = fixed_pre_node_return_receipt();
    assert!(validate_pre_node_return(&report, &prepared).is_ok());
    for field in ["entry_sequence", "return_sequence"] {
        let mut changed = report.clone();
        changed["pre_node_shell_observation"]["classification"][field] = json!(8);
        assert!(validate_pre_node_return(&changed, &prepared).is_err());
    }
    let mut repeated = report.clone();
    repeated["shell_classification"]["pre_node_returned_calls"] = json!(2);
    assert!(validate_pre_node_return(&repeated, &prepared).is_err());
    let mut second_entry = report;
    second_entry["pre_node_shell_entries"] =
        json!([{"sequence":4,"tid":42},{"sequence":6,"tid":42}]);
    assert!(validate_pre_node_return(&second_entry, &prepared).is_err());
}

#[test]
fn pre_node_return_cannot_borrow_post_node_identity_or_replace_its_gate() {
    let (report, prepared) = fixed_pre_node_return_receipt();
    for (field, value) in [
        ("node_create_sequence", Value::Null),
        ("node_process_id", json!(0)),
        ("node_process_birth", json!(300)),
        ("post_start_clr_stack_required", json!(true)),
        ("pre_node_clr_stack_required", json!(false)),
        ("generation", serde_json::to_value([2_u8; 16]).unwrap()),
        ("raw_return_u64", json!(0x1_0000_4550_u64)),
    ] {
        let mut changed = report.clone();
        changed["pre_node_shell_observation"]["classification"][field] = value;
        assert!(validate_pre_node_return(&changed, &prepared).is_err());
    }
    let mut no_post = report.clone();
    no_post["shell_observation"] = Value::Null;
    assert!(validate_native_return(&no_post, &prepared).is_err());
    assert!(validate_pre_node_return(&no_post, &prepared).is_err());
}

#[test]
fn pre_node_return_requires_reader_reaping_and_same_stop_stack_before_rearm() {
    let (report, prepared) = fixed_pre_node_return_receipt();
    for field in [
        "reader_reaped",
        "reader_job_empty",
        "rearmed_after_reader_reaped",
        "live_threads_restored_before_reader",
    ] {
        let mut changed = report.clone();
        changed["pre_node_shell_observation"][field] = json!(false);
        assert!(validate_pre_node_return(&changed, &prepared).is_err());
    }
    let mut old_thread = report.clone();
    old_thread["pre_node_shell_observation"]["reader"]["thread_birth"] = json!(201);
    assert!(validate_pre_node_return(&old_thread, &prepared).is_err());
    let mut unknown_frame = report;
    unknown_frame["pre_node_shell_observation"]["reader"]["frames"][0]["il_status"] =
        json!(0x80004005_u32);
    assert!(validate_pre_node_return(&unknown_frame, &prepared).is_err());
}

#[test]
fn post_node_sample_requires_the_original_live_worker_stop_and_owned_configuration() {
    let (report, prepared) = fixed_pre_node_return_receipt();
    for (field, value) in [
        ("sequence", json!(8)),
        ("sequence", json!(11)),
        ("event_code", json!(5)),
        ("identity", Value::Null),
        ("scope", json!("all_threads_continuously")),
        ("addresses_equal", json!(false)),
        ("configuration_equal", Value::Null),
        ("dirty", json!(false)),
    ] {
        let mut changed = report.clone();
        changed["shell_classification"]["post_node_root_stop"][field] = value;
        assert!(validate_pre_node_return(&changed, &prepared).is_err());
    }
}

#[test]
fn native_return_accepts_identified_framework_pinvoke_frames() {
    let (mut report, prepared) = fixed_native_return_with_stub_receipt();
    assert!(validate_native_return(&report, &prepared).is_ok());
    report["shell_observation"]["reader"]["frames"][0]["runtime_name"] =
        json!("domain_neutral_pinvoke_stub");
    assert!(validate_native_return(&report, &prepared).is_ok());
}

#[test]
fn native_return_rejects_unknown_nil_frames_despite_a_complete_caller() {
    let (mut report, prepared) = fixed_native_return_with_stub_receipt();
    report["shell_observation"]["reader"]["frames"][0]["runtime_name"] = json!("unknown");
    assert!(validate_native_return(&report, &prepared).is_err());
}

#[test]
fn native_return_requires_complete_bounded_stub_names() {
    let (report, prepared) = fixed_native_return_with_stub_receipt();
    let mut truncated = report.clone();
    truncated["shell_observation"]["reader"]["frames"][0]["name_complete"] = json!(false);
    assert!(validate_native_return(&truncated, &prepared).is_err());
    let mut empty = report.clone();
    empty["shell_observation"]["reader"]["frames"][0]["name_units_needed"] = json!(0);
    assert!(validate_native_return(&empty, &prepared).is_err());
    let mut oversized = report;
    oversized["shell_observation"]["reader"]["frames"][0]["name_units_needed"] = json!(513);
    assert!(validate_native_return(&oversized, &prepared).is_err());
}

#[test]
fn native_return_rejects_pinvoke_exemptions_after_the_first_frame() {
    let (mut report, prepared) = fixed_native_return_with_stub_receipt();
    report["shell_observation"]["reader"]["frames"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
    assert!(validate_native_return(&report, &prepared).is_err());
}

#[test]
fn native_return_does_not_replace_failed_metadata_with_an_ancestor() {
    let (mut report, prepared) = fixed_native_return_with_stub_receipt();
    let frames = report["shell_observation"]["reader"]["frames"]
        .as_array_mut()
        .unwrap();
    frames.push(frames[1].clone());
    frames[1]["il_status"] = json!(0x80004005_u32);
    frames[1]["il_offsets"] = json!([]);
    frames[1]["il_offsets_needed"] = json!(0);
    assert!(validate_native_return(&report, &prepared).is_err());
}

#[test]
fn native_return_rejects_fabricated_stub_il() {
    let (mut report, prepared) = fixed_native_return_with_stub_receipt();
    report["shell_observation"]["reader"]["frames"][0]["il_offsets"] = json!([160]);
    report["shell_observation"]["reader"]["frames"][0]["il_offsets_needed"] = json!(1);
    assert!(validate_native_return(&report, &prepared).is_err());
    report["shell_observation"]["reader"]["frames"][0]["il_status"] = json!(0);
    assert!(validate_native_return(&report, &prepared).is_err());
}

#[test]
fn native_return_does_not_use_a_stub_as_the_required_caller() {
    let (mut report, prepared) = fixed_native_return_with_stub_receipt();
    report["shell_observation"]["reader"]["frames"]
        .as_array_mut()
        .unwrap()
        .remove(1);
    assert!(validate_native_return(&report, &prepared).is_err());
}

#[test]
fn descendant_create_roles_require_original_identity_and_one_fixed_target() {
    let (report, prepared) = fixed_native_return_receipt();
    let target = &report["descendant_creates"]["51"];
    assert!(validate_descendant_create(target, false).unwrap());
    assert!(validate_descendant_create(target, true).is_err());
    let mut mismatched = target.clone();
    mismatched["thread_process_id_matched"] = json!(false);
    assert!(validate_descendant_create(&mismatched, false).is_err());
    let mut unknown = target.clone();
    unknown["path_matched"] = Value::Null;
    unknown["file_identity_matched"] = Value::Null;
    unknown["sha256_matched"] = Value::Null;
    unknown["role"] = json!("unclassified");
    assert!(!validate_descendant_create(&unknown, false).unwrap());
    let mut no_target = report;
    no_target["descendant_creates"]["51"] = unknown;
    assert!(validate_native_return(&no_target, &prepared).is_err());
}

#[test]
fn fixed_descendant_path_rejects_mismatched_or_unknown_image_identity() {
    let (report, _) = fixed_native_return_receipt();
    let target = &report["descendant_creates"]["51"];
    let mut different_file = target.clone();
    different_file["file_identity_matched"] = json!(false);
    assert!(validate_descendant_create(&different_file, false).is_err());
    let mut different_bytes = target.clone();
    different_bytes["sha256_matched"] = json!(false);
    assert!(validate_descendant_create(&different_bytes, false).is_err());
    let mut unreadable = target.clone();
    unreadable["sha256_matched"] = Value::Null;
    assert!(validate_descendant_create(&unreadable, false).is_err());
}

#[test]
fn non_target_descendant_before_fixed_node_does_not_replace_its_binding() {
    let (mut report, prepared) = fixed_native_return_receipt();
    let other = json!({"pid":49,"tid":50,"birth":250,"sequence":6,
        "process_id_matched":true,"thread_process_id_matched":true,"thread_id_matched":true,
        "path_matched":false,"file_identity_matched":false,"sha256_matched":false,
        "role":"unclassified","cleanup_create":false,"exit_confirmed":true,"exit_birth":250,"exit_code":1});
    assert!(!validate_descendant_create(&other, false).unwrap());
    report["descendant_creates"]["49"] = other;
    assert!(validate_native_return(&report, &prepared).is_ok());
    assert_eq!(report["node_created"]["pid"], json!(51));
    assert_eq!(report["node_created"]["sequence"], json!(8));
}

#[test]
fn native_return_requires_every_original_descendant_exit_and_birth() {
    let (mut report, prepared) = fixed_native_return_receipt();
    report["descendant_creates"]["49"] = json!({"pid":49,"tid":50,"birth":250,"sequence":6,
        "process_id_matched":true,"thread_process_id_matched":true,"thread_id_matched":true,
        "path_matched":false,"file_identity_matched":false,"sha256_matched":false,
        "role":"unclassified","cleanup_create":false,"exit_confirmed":false,"exit_birth":250,"exit_code":1});
    assert!(validate_native_return(&report, &prepared).is_err());
    report["descendant_creates"]["49"]["exit_confirmed"] = json!(true);
    report["descendant_creates"]["49"]["exit_birth"] = json!(251);
    assert!(validate_native_return(&report, &prepared).is_err());
    report["descendant_creates"]["49"]["exit_birth"] = json!(250);
    report["all_descendants_reaped"] = json!(false);
    assert!(validate_native_return(&report, &prepared).is_err());
}

#[test]
fn native_return_receipts_require_one_pair_after_original_child_create() {
    let (report, prepared) = fixed_native_return_receipt();
    assert!(validate_native_return(&report, &prepared).is_ok());
    let mut missing = report.clone();
    missing["shell_observation"] = Value::Null;
    assert!(validate_native_return(&missing, &prepared).is_err());
    let mut repeated = report.clone();
    let entries = repeated["shell_entries"].as_array_mut().unwrap();
    entries.push(entries[0].clone());
    assert!(validate_native_return(&repeated, &prepared).is_err());
    let mut late_create = report;
    late_create["node_created"]["sequence"] = json!(12);
    late_create["shell_observation"]["classification"]["node_create_sequence"] = json!(12);
    assert!(validate_native_return(&late_create, &prepared).is_err());
}

#[test]
fn native_return_requires_pre_node_skipped_entry_on_the_original_worker() {
    let (report, prepared) = fixed_native_return_receipt();
    for (field, value) in [
        ("sequence", json!(8)),
        ("node_created_before_event", json!(true)),
        ("pid", json!(51)),
        ("tid", json!(40)),
        ("thread_birth", json!(201)),
    ] {
        let mut changed = report.clone();
        changed["shell_skips"][0][field] = value;
        assert!(validate_native_return(&changed, &prepared).is_err());
    }
    let mut missing = report.clone();
    missing["shell_skips"] = json!([]);
    assert!(validate_native_return(&missing, &prepared).is_err());
    let mut repeated = report;
    let skips = repeated["shell_skips"].as_array_mut().unwrap();
    skips.push(skips[0].clone());
    assert!(validate_native_return(&repeated, &prepared).is_err());
}

#[test]
fn native_return_requires_bound_first_skipped_receipt_and_observed_path_match() {
    let (report, prepared) = fixed_native_return_receipt();
    for (field, value) in [
        ("sequence", json!(8)),
        ("node_create_sequence", json!(6)),
        ("generation", serde_json::to_value([2_u8; 16]).unwrap()),
        (
            "identity",
            json!({"process_id":41,"thread_id":40,"process_birth":100,"thread_birth":200}),
        ),
        ("flags", json!(0)),
        ("path_comparison", json!("not_checked")),
        ("path_comparison", json!("mismatched")),
        ("path_comparison", json!("unreadable")),
    ] {
        let mut changed = report.clone();
        changed["shell_classification"]["first_skipped_entry"][field] = value;
        assert!(validate_native_return(&changed, &prepared).is_err());
    }
    let mut missing = report.clone();
    missing["shell_classification"]["first_skipped_entry"] = Value::Null;
    assert!(validate_native_return(&missing, &prepared).is_err());
    let mut missing_node_state = report;
    missing_node_state["shell_classification"]["first_skipped_entry"]
        .as_object_mut()
        .unwrap()
        .remove("node_create_sequence");
    assert!(validate_native_return(&missing_node_state, &prepared).is_err());
}

#[test]
fn native_return_requires_one_successful_control_readback_for_the_skipped_entry() {
    let (report, prepared) = fixed_native_return_receipt();
    for (field, value) in [
        ("resume_flag_writes_before", Value::Null),
        ("resume_flag_writes_after", json!(0)),
        ("resume_flag_writes_after", json!(2)),
        ("fixed_eflags_api_readbacks_before", Value::Null),
        ("fixed_eflags_api_readbacks_after", json!(2)),
    ] {
        let mut changed = report.clone();
        changed["shell_skips"][0][field] = value;
        assert!(validate_native_return(&changed, &prepared).is_err());
    }
    let mut exact = report;
    exact["shell_skips"][0]["fixed_eflags_api_readbacks_after"] = json!(0);
    exact["shell_classification"]["fixed_eflags_api_readbacks"] = json!(0);
    assert!(validate_native_return(&exact, &prepared).is_ok());
}

#[test]
fn native_return_receipts_require_private_mapping_fixture_value_and_restoration() {
    let (report, prepared) = fixed_native_return_receipt();
    for (field, value) in [
        ("return_region_kind", json!("image")),
        ("raw_return_u64", json!(0x1_0000_4550_u64)),
        ("raw_return_low32", json!(0)),
        ("registers_restored", json!(false)),
        ("execution_context_unchanged", json!(false)),
    ] {
        let mut changed = report.clone();
        changed["shell_observation"]["classification"][field] = value;
        assert!(validate_native_return(&changed, &prepared).is_err());
    }
    let mut exited_armed = report.clone();
    exited_armed["shell_classification"]["threads_exited_before_restore"] = json!(1);
    assert!(validate_native_return(&exited_armed, &prepared).is_err());
    exited_armed["shell_classification"]["restoration"] = json!("original_exit_confirmed");
    assert!(validate_native_return(&exited_armed, &prepared).is_ok());
    let mut unverified = report;
    unverified["shell_classification"]["restoration"] = json!("unknown");
    assert!(validate_native_return(&unverified, &prepared).is_err());
}

#[test]
fn native_return_receipts_require_original_working_thread_stack_and_cleanup() {
    let (report, prepared) = fixed_native_return_receipt();
    let mut wrong_frame = report.clone();
    wrong_frame["shell_observation"]["reader"]["frames"][0]["method_token"] = json!(5);
    assert!(validate_native_return(&wrong_frame, &prepared).is_err());
    for field in ["reader_reaped", "reader_job_empty"] {
        let mut unreleased = report.clone();
        unreleased["shell_observation"][field] = json!(false);
        assert!(validate_native_return(&unreleased, &prepared).is_err());
    }
    let mut main_thread = report.clone();
    main_thread["shell_observation"]["main_tid"] = json!(42);
    assert!(validate_native_return(&main_thread, &prepared).is_err());
    let mut old_object = report;
    old_object["shell_observation"]["reader"]["exception_source"] =
        json!("last_thrown_object_candidate");
    assert!(validate_native_return(&old_object, &prepared).is_err());
}
