use std::io::{self, Read, Write};
use std::os::windows::io::FromRawHandle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mio::windows::NamedPipe;
use mio::{Events, Interest, Poll, Token};
use parking_lot::FairMutex;

use super::pipes;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::local_tty::event_loop::{EventLoop, PTY_TOKEN, SIGNALS_TOKEN};
use crate::terminal::local_tty::{ChildEvent, EventedPty, EventedReadWrite, mio_channel};
use crate::terminal::model::test_utils::block_size;
use crate::terminal::writeable_pty::Message;
use crate::terminal::{SizeInfo, SizeUpdate, TerminalModel};

struct PipePty {
    pipe: NamedPipe,
    control_tx: mio_channel::Sender<Message>,
    exit_after_first_read: bool,
    dropped_tx: mpsc::Sender<()>,
    killed: Arc<AtomicBool>,
    observed_eof: Arc<AtomicBool>,
}

impl Read for PipePty {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let bytes_read = self.pipe.read(buf)?;
        if bytes_read == 0 {
            self.observed_eof.store(true, Ordering::SeqCst);
        }
        if bytes_read > 0 && std::mem::take(&mut self.exit_after_first_read) {
            // 用真实 NamedPipe 首次完成的读取触发退出，余下数据必须继续经 IOCP 接收。
            self.control_tx.send(Message::ChildExited).unwrap();
        }
        Ok(bytes_read)
    }
}

impl EventedReadWrite for PipePty {
    type Reader = Self;
    type Writer = NamedPipe;

    fn register(&mut self, poll: &Poll, interest: Interest) -> io::Result<()> {
        poll.registry()
            .register(&mut self.pipe, PTY_TOKEN, interest)
    }

    fn reregister(&mut self, poll: &Poll, interest: Interest) -> io::Result<()> {
        poll.registry()
            .reregister(&mut self.pipe, PTY_TOKEN, interest)
    }

    fn deregister(&mut self, poll: &Poll) -> io::Result<()> {
        poll.registry().deregister(&mut self.pipe)
    }

    fn reader(&mut self) -> &mut Self::Reader {
        self
    }

    fn read_token(&self) -> Token {
        PTY_TOKEN
    }

    fn writer(&mut self) -> &mut Self::Writer {
        &mut self.pipe
    }

    fn write_token(&self) -> Token {
        PTY_TOKEN
    }
}

impl EventedPty for PipePty {
    fn child_event_token(&self) -> Token {
        SIGNALS_TOKEN
    }

    fn next_child_event(&mut self) -> Option<ChildEvent> {
        None
    }

    fn on_resize(&mut self, _: &SizeInfo) {}

    fn kill(self) -> anyhow::Result<()> {
        self.killed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

impl Drop for PipePty {
    fn drop(&mut self) {
        let _ = self.dropped_tx.send(());
    }
}

struct RunningPipe {
    thread: JoinHandle<()>,
    control_tx: mio_channel::Sender<Message>,
    dropped_rx: mpsc::Receiver<()>,
    terminal: Arc<FairMutex<TerminalModel>>,
    killed: Arc<AtomicBool>,
    observed_eof: Arc<AtomicBool>,
}

fn spawn_pipe_reader() -> (RunningPipe, NamedPipe) {
    let pipes::DuplexPipe { client, server } = pipes::create_async_anonymous_pipe().unwrap();
    // 测试分别拥有管道两端，不启动 ConPTY 或外部进程。
    let client = unsafe { NamedPipe::from_raw_handle(client.0) };
    let server = unsafe { NamedPipe::from_raw_handle(server.0) };
    let (control_tx, control_rx) = mio_channel::channel();
    let (dropped_tx, dropped_rx) = mpsc::channel();
    let killed = Arc::new(AtomicBool::new(false));
    let observed_eof = Arc::new(AtomicBool::new(false));
    let pty = PipePty {
        pipe: server,
        control_tx: control_tx.clone(),
        exit_after_first_read: true,
        dropped_tx,
        killed: killed.clone(),
        observed_eof: observed_eof.clone(),
    };
    let listener = ChannelEventListener::new_for_test();
    let mut terminal = TerminalModel::mock(None, Some(listener.clone()));
    terminal.resize(SizeUpdate::from_cell_dimensions(block_size().size, 24, 80));
    terminal.simulate_long_running_block("pipe-output-test", "");
    let terminal = Arc::new(FairMutex::new(terminal));
    let thread = EventLoop::new(terminal.clone(), listener, pty, control_rx).spawn();

    (
        RunningPipe {
            thread,
            control_tx,
            dropped_rx,
            terminal,
            killed,
            observed_eof,
        },
        client,
    )
}

fn wait_for_writable(poll: &mut Poll) {
    let mut events = Events::with_capacity(8);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        poll.poll(
            &mut events,
            Some(deadline.saturating_duration_since(Instant::now())),
        )
        .unwrap();
        if events.iter().any(|event| event.is_writable()) {
            break;
        }
        assert!(Instant::now() < deadline, "管道写入应在总时限内完成");
    }
}

fn write_pipe_output(mut pipe: NamedPipe, output: &[u8]) -> (NamedPipe, Poll) {
    let mut poll = Poll::new().unwrap();
    poll.registry()
        .register(&mut pipe, Token(4), Interest::READABLE | Interest::WRITABLE)
        .unwrap();
    // 先消费注册时的可写通知，后一次可写通知才能证明异步写入已经完成。
    wait_for_writable(&mut poll);
    assert_eq!(pipe.write(output).unwrap(), output.len());
    wait_for_writable(&mut poll);
    // 保留注册和 Poll，让 drop 后的取消完成产生可读通知并释放写端句柄。
    (pipe, poll)
}

fn close_pipe_writer(pipe: NamedPipe, mut poll: Poll) {
    drop(pipe);
    let mut events = Events::with_capacity(8);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        poll.poll(
            &mut events,
            Some(deadline.saturating_duration_since(Instant::now())),
        )
        .unwrap();
        // 读端不会向写端发送数据；可读通知表示 pending read 已经取消或因对端关闭结束。
        // read_done 已释放该读取持有的 Arc，无需等待读端测试线程再次处理 EOF。
        if events.iter().any(|event| event.is_readable()) {
            break;
        }
        assert!(Instant::now() < deadline, "写端的异步读取必须完成取消收尾");
    }
}

fn finish_pipe_reader(running: RunningPipe) -> Arc<FairMutex<TerminalModel>> {
    let finished = running
        .dropped_rx
        .recv_timeout(Duration::from_secs(5))
        .is_ok();
    if !finished {
        // 超过总等待上限时仍发 Shutdown，防止失败测试遗留阻塞线程。
        let _ = running.control_tx.send(Message::Shutdown);
    }
    running.thread.join().unwrap();
    assert!(finished, "退出后的管道读取必须在总时限内结束");
    assert!(!running.killed.load(Ordering::SeqCst));
    running.terminal
}

#[test]
fn child_exit_drains_pending_named_pipe_reads_until_eof() {
    let (running, writer) = spawn_pipe_reader();
    let observed_eof = running.observed_eof.clone();
    // 大于 mio 默认的 4 KiB 缓冲，确保尾标记需要后续 IOCP 完成通知。
    let mut output = vec![0; 64 * 1024];
    output.extend_from_slice(b"iocp-exit-tail-preserved");
    let writer = thread::spawn(move || {
        let (writer, poll) = write_pipe_output(writer, &output);
        close_pipe_writer(writer, poll);
    });

    let writer_result = writer.join();
    let terminal = finish_pipe_reader(running);
    writer_result.unwrap();

    assert!(
        observed_eof.load(Ordering::SeqCst),
        "读端必须收到真实 EOF，不能只依靠总时限结束"
    );
    assert_eq!(
        terminal
            .lock()
            .block_list()
            .active_block()
            .output_to_string(),
        "iocp-exit-tail-preserved"
    );
}

#[test]
fn child_exit_without_pipe_eof_has_bounded_drain() {
    let (running, writer) = spawn_pipe_reader();
    let observed_eof = running.observed_eof.clone();
    let (writer, poll) = write_pipe_output(writer, b"pipe-remains-open");

    // 写端一直存活，不会产生 EOF；退出必须依靠总时限兜底。
    let terminal = finish_pipe_reader(running);
    let saw_eof_before_closing_writer = observed_eof.load(Ordering::SeqCst);
    close_pipe_writer(writer, poll);
    assert!(!saw_eof_before_closing_writer, "写端仍存活时不应收到 EOF");

    assert_eq!(
        terminal
            .lock()
            .block_list()
            .active_block()
            .output_to_string(),
        "pipe-remains-open"
    );
}
