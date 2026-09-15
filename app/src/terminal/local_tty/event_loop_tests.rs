use std::io::{self, Cursor, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use futures_lite::future;
use mio::unix::SourceFd;
use parking_lot::{FairMutex, Mutex};

use super::{EventLoop, MAX_LOCKED_READ, PTY_TOKEN, SIGNALS_TOKEN};
use crate::terminal::event::Event;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::local_tty::{ChildEvent, EventedPty, EventedReadWrite, mio_channel};
use crate::terminal::model::ansi::Mode;
use crate::terminal::model::terminal_model::HandlerEvent;
use crate::terminal::model::test_utils::block_size;
use crate::terminal::writeable_pty::Message;
use crate::terminal::{SizeInfo, SizeUpdate, TerminalModel};

#[derive(Default)]
struct PtyObservation {
    read_count: usize,
    written: Vec<u8>,
    killed: bool,
}

type PtyRead =
    Box<dyn FnMut(&mut [u8], usize, &mio_channel::Sender<Message>) -> io::Result<usize> + Send>;

struct ContinuouslyReadablePty {
    readiness: UnixStream,
    peer: UnixStream,
    control_tx: mio_channel::Sender<Message>,
    message_after_first_read: Option<Message>,
    would_block_on_first_write: bool,
    read: Option<PtyRead>,
    observation: Arc<Mutex<PtyObservation>>,
}

impl Read for ContinuouslyReadablePty {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut observation = self.observation.lock();
        observation.read_count += 1;
        // 回归时受控退出，避免持续可读的测试线程无限运行。
        if observation.read_count > 8 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "控制消息未在有限读批次内处理",
            ));
        }
        let read_count = observation.read_count;
        drop(observation);

        if let Some(message) = self.message_after_first_read.take() {
            self.control_tx.send(message).unwrap();
        }

        if let Some(read) = &mut self.read {
            return read(buf, read_count, &self.control_tx);
        }

        let bytes_read = buf.len().min(MAX_LOCKED_READ);
        buf[..bytes_read].fill(0);
        Ok(bytes_read)
    }
}

impl Write for ContinuouslyReadablePty {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if std::mem::take(&mut self.would_block_on_first_write) {
            // 先填满再排空真实套接字缓冲区，产生下一次可写通知。
            let fill_error = io::copy(&mut io::repeat(0), &mut self.readiness).unwrap_err();
            assert_eq!(fill_error.kind(), io::ErrorKind::WouldBlock);
            let drain_error = io::copy(&mut self.peer, &mut io::sink()).unwrap_err();
            assert_eq!(drain_error.kind(), io::ErrorKind::WouldBlock);
            return Err(io::ErrorKind::WouldBlock.into());
        }

        self.observation.lock().written.extend_from_slice(buf);
        self.control_tx.send(Message::Shutdown).unwrap();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl EventedReadWrite for ContinuouslyReadablePty {
    type Reader = Self;
    type Writer = Self;

    fn register(&mut self, poll: &mio::Poll, interest: mio::Interest) -> io::Result<()> {
        poll.registry().register(
            &mut SourceFd(&self.readiness.as_raw_fd()),
            PTY_TOKEN,
            interest,
        )?;
        self.peer.write_all(b"ready")
    }

    fn reregister(&mut self, poll: &mio::Poll, interest: mio::Interest) -> io::Result<()> {
        poll.registry().reregister(
            &mut SourceFd(&self.readiness.as_raw_fd()),
            PTY_TOKEN,
            interest,
        )
    }

    fn deregister(&mut self, poll: &mio::Poll) -> io::Result<()> {
        poll.registry()
            .deregister(&mut SourceFd(&self.readiness.as_raw_fd()))
    }

    fn reader(&mut self) -> &mut Self::Reader {
        self
    }

    fn read_token(&self) -> mio::Token {
        PTY_TOKEN
    }

    fn writer(&mut self) -> &mut Self::Writer {
        self
    }

    fn write_token(&self) -> mio::Token {
        PTY_TOKEN
    }
}

impl EventedPty for ContinuouslyReadablePty {
    fn child_event_token(&self) -> mio::Token {
        SIGNALS_TOKEN
    }

    fn next_child_event(&mut self) -> Option<ChildEvent> {
        None
    }

    fn on_resize(&mut self, _: &SizeInfo) {}

    fn kill(self) -> anyhow::Result<()> {
        self.observation.lock().killed = true;
        Ok(())
    }
}

fn run_with_continuous_output(
    message_after_first_read: Message,
    would_block_on_first_write: bool,
) -> Arc<Mutex<PtyObservation>> {
    let (control_tx, control_rx) = mio_channel::channel();
    let (readiness, peer) = UnixStream::pair().unwrap();
    readiness.set_nonblocking(true).unwrap();
    peer.set_nonblocking(true).unwrap();
    let observation = Arc::new(Mutex::new(PtyObservation::default()));
    let pty = ContinuouslyReadablePty {
        readiness,
        peer,
        control_tx,
        message_after_first_read: Some(message_after_first_read),
        would_block_on_first_write,
        read: None,
        observation: Arc::clone(&observation),
    };
    let listener = ChannelEventListener::new_for_test();
    let terminal = Arc::new(FairMutex::new(TerminalModel::mock(
        None,
        Some(listener.clone()),
    )));

    EventLoop::new(terminal, listener, pty, control_rx)
        .spawn()
        .join()
        .unwrap();

    observation
}

#[test]
fn continuous_output_does_not_starve_interrupt_input() {
    let observation = run_with_continuous_output(Message::Input(vec![0x03].into()), false);

    assert_eq!(observation.lock().written, [0x03]);
}

#[test]
fn continuous_output_does_not_starve_write_readiness_after_would_block() {
    let observation = run_with_continuous_output(Message::Input(vec![0x03].into()), true);

    assert_eq!(observation.lock().written, [0x03]);
}

#[test]
fn continuous_output_does_not_starve_shutdown() {
    let observation = run_with_continuous_output(Message::Shutdown, false);
    let observation = observation.lock();

    assert_eq!(observation.read_count, 1);
    assert!(observation.killed);
}

struct RunningPty {
    thread: JoinHandle<()>,
    control_tx: mio_channel::Sender<Message>,
    terminal: Arc<FairMutex<TerminalModel>>,
    observation: Arc<Mutex<PtyObservation>>,
}

fn spawn_with_reader(
    read: PtyRead,
    message_after_first_read: Option<Message>,
    listener: ChannelEventListener,
) -> RunningPty {
    let (control_tx, control_rx) = mio_channel::channel();
    let (readiness, peer) = UnixStream::pair().unwrap();
    readiness.set_nonblocking(true).unwrap();
    peer.set_nonblocking(true).unwrap();
    let observation = Arc::new(Mutex::new(PtyObservation::default()));
    let pty = ContinuouslyReadablePty {
        readiness,
        peer,
        control_tx: control_tx.clone(),
        message_after_first_read,
        would_block_on_first_write: false,
        read: Some(read),
        observation: observation.clone(),
    };
    let mut terminal = TerminalModel::mock(None, Some(listener.clone()));
    // 默认测试网格只有七列且停留在输入阶段；这里模拟正常命令输出，避免尾标记被换行或早期输出缓存遮蔽。
    terminal.resize(SizeUpdate::from_cell_dimensions(block_size().size, 24, 80));
    terminal.simulate_long_running_block("pty-output-test", "");
    let terminal = Arc::new(FairMutex::new(terminal));
    let thread = EventLoop::new(terminal.clone(), listener, pty, control_rx).spawn();

    RunningPty {
        thread,
        control_tx,
        terminal,
        observation,
    }
}

#[test]
fn child_exit_preserves_output_after_read_batch() {
    // 首批恰好达到公平调度上限，尾标记只能在下一批进入模型。
    let mut output = vec![0; MAX_LOCKED_READ];
    output.extend_from_slice(b"exit-tail-preserved");
    let mut output = Cursor::new(output);
    let running = spawn_with_reader(
        Box::new(move |buf, _, _| {
            let limit = buf.len().min(MAX_LOCKED_READ);
            output.read(&mut buf[..limit])
        }),
        Some(Message::ChildExited),
        ChannelEventListener::new_for_test(),
    );

    running.thread.join().unwrap();

    assert!(
        running
            .terminal
            .lock()
            .block_list()
            .active_block()
            .contents_to_string()
            .contains("exit-tail-preserved")
    );
    assert!(!running.observation.lock().killed);
}

#[test]
fn pty_hangup_preserves_bytes_buffered_while_model_is_locked() {
    let (start_tx, start_rx) = mpsc::channel();
    let (hangup_tx, hangup_rx) = mpsc::channel();
    let running = spawn_with_reader(
        Box::new(move |buf, read_count, _| {
            if read_count == 1 {
                start_rx.recv().unwrap();
                buf[..11].copy_from_slice(b"locked-tail");
                return Ok(11);
            }
            if read_count == 2 {
                hangup_tx.send(()).unwrap();
                return Err(io::Error::from_raw_os_error(libc::EIO));
            }
            Ok(0)
        }),
        Some(Message::ChildExited),
        ChannelEventListener::new_for_test(),
    );

    // 锁保持到第二次读取：首批小于缓冲上限，必然经过 try_lock 失败后继续读取的路径。
    let terminal = running.terminal.lock();
    start_tx.send(()).unwrap();
    let reached_hangup = hangup_rx.recv_timeout(Duration::from_secs(5)).is_ok();
    drop(terminal);
    running.thread.join().unwrap();

    assert!(reached_hangup, "测试必须先读入尾部，再在持锁期间遇到 EIO");
    assert_eq!(
        running
            .terminal
            .lock()
            .block_list()
            .active_block()
            .output_to_string(),
        "locked-tail"
    );
}

#[test]
fn pty_eof_preserves_buffered_bytes_without_reading_again() {
    let (start_tx, start_rx) = mpsc::channel();
    let (eof_tx, eof_rx) = mpsc::channel();
    let running = spawn_with_reader(
        Box::new(move |buf, read_count, _| {
            if read_count == 1 {
                start_rx.recv().unwrap();
                buf[..11].copy_from_slice(b"locked-tail");
                return Ok(11);
            }
            if read_count == 2 {
                eof_tx.send(()).unwrap();
            }
            Ok(0)
        }),
        Some(Message::ChildExited),
        ChannelEventListener::new_for_test(),
    );

    let terminal = running.terminal.lock();
    start_tx.send(()).unwrap();
    let reached_eof = eof_rx.recv_timeout(Duration::from_secs(5)).is_ok();
    drop(terminal);
    running.thread.join().unwrap();

    assert!(reached_eof, "测试必须先读入尾部，再在持锁期间遇到 EOF");
    assert_eq!(running.observation.lock().read_count, 2);
    assert_eq!(
        running
            .terminal
            .lock()
            .block_list()
            .active_block()
            .output_to_string(),
        "locked-tail"
    );
}

#[test]
fn child_exit_output_drain_does_not_starve_shutdown() {
    let running = spawn_with_reader(
        Box::new(|buf, read_count, control_tx| {
            if read_count == 2 {
                control_tx.send(Message::Shutdown).unwrap();
            }
            buf[..MAX_LOCKED_READ].fill(0);
            Ok(MAX_LOCKED_READ)
        }),
        Some(Message::ChildExited),
        ChannelEventListener::new_for_test(),
    );

    running.thread.join().unwrap();

    assert_eq!(running.observation.lock().read_count, 2);
    assert!(!running.observation.lock().killed);
}

#[test]
fn child_exit_and_shutdown_in_one_wakeup_do_not_start_output_drain() {
    let running = spawn_with_reader(
        Box::new(|buf, read_count, control_tx| {
            if read_count == 1 {
                control_tx.send(Message::Shutdown).unwrap();
            }
            buf[..MAX_LOCKED_READ].fill(0);
            Ok(MAX_LOCKED_READ)
        }),
        Some(Message::ChildExited),
        ChannelEventListener::new_for_test(),
    );

    running.thread.join().unwrap();

    assert_eq!(running.observation.lock().read_count, 1);
    assert!(!running.observation.lock().killed);
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
#[test]
fn child_exit_stops_output_drain_at_pty_hangup() {
    let mut output = vec![0; MAX_LOCKED_READ];
    output.extend_from_slice(b"hangup-tail-preserved");
    let mut output = Cursor::new(output);
    let running = spawn_with_reader(
        Box::new(move |buf, _, _| {
            let limit = buf.len().min(MAX_LOCKED_READ);
            let bytes_read = output.read(&mut buf[..limit])?;
            if bytes_read == 0 {
                return Err(io::Error::from_raw_os_error(libc::EIO));
            }
            Ok(bytes_read)
        }),
        Some(Message::ChildExited),
        ChannelEventListener::new_for_test(),
    );

    running.thread.join().unwrap();

    assert_eq!(running.observation.lock().read_count, 3);
    assert!(
        running
            .terminal
            .lock()
            .block_list()
            .active_block()
            .contents_to_string()
            .contains("hangup-tail-preserved")
    );
}

fn synchronized_read_batch() -> Vec<u8> {
    // 把开始标记放在首批末尾，下一次零超时 poll 紧接同步输出开始。
    let frame = b"\x1b[?2026hsync-frame";
    let mut batch = vec![0; MAX_LOCKED_READ - frame.len()];
    batch.extend_from_slice(frame);
    batch
}

#[test]
fn child_exit_flushes_unfinished_synchronized_output() {
    let mut output = synchronized_read_batch();
    output.extend_from_slice(b"-exit-tail");
    let mut output = Cursor::new(output);
    let running = spawn_with_reader(
        Box::new(move |buf, _, _| {
            let limit = buf.len().min(MAX_LOCKED_READ);
            output.read(&mut buf[..limit])
        }),
        Some(Message::ChildExited),
        ChannelEventListener::new_for_test(),
    );

    running.thread.join().unwrap();

    assert!(
        running
            .terminal
            .lock()
            .block_list()
            .active_block()
            .contents_to_string()
            .contains("sync-frame-exit-tail")
    );
}

fn is_sync_output_finished(event: &Event) -> bool {
    matches!(
        event,
        Event::Handler(HandlerEvent::UnsetMode {
            mode: Mode::SyncOutput
        })
    )
}

#[test]
fn zero_timeout_poll_preserves_unexpired_synchronized_output() {
    let (events_tx, events_rx) = async_channel::unbounded();
    let listener = ChannelEventListener::builder_for_test()
        .with_terminal_events_tx(events_tx)
        .build();
    let mut output = Cursor::new(synchronized_read_batch());
    let running = spawn_with_reader(
        Box::new(move |buf, read_count, control_tx| {
            if read_count == 2 {
                assert!(
                    !std::iter::from_fn(|| events_rx.try_recv().ok())
                        .any(|event| is_sync_output_finished(&event))
                );
                control_tx.send(Message::Shutdown).unwrap();
            }
            output.read(buf)
        }),
        None,
        listener,
    );

    running.thread.join().unwrap();

    assert_eq!(running.observation.lock().read_count, 2);
    assert!(
        !running
            .terminal
            .lock()
            .block_list()
            .active_block()
            .contents_to_string()
            .contains("sync-frame")
    );
}

#[test]
fn synchronized_output_timeout_flushes_once() {
    let (events_tx, events_rx) = async_channel::unbounded();
    let listener = ChannelEventListener::builder_for_test()
        .with_terminal_events_tx(events_tx)
        .build();
    let mut output = Cursor::new(synchronized_read_batch());
    let running = spawn_with_reader(Box::new(move |buf, _, _| output.read(buf)), None, listener);

    // 等待真正的超时刷新事件；回归时五秒上限负责结束测试并清理线程。
    let flushed = future::block_on(future::race(
        async {
            while let Ok(event) = events_rx.recv().await {
                if is_sync_output_finished(&event) {
                    return true;
                }
            }
            false
        },
        async {
            async_io::Timer::after(Duration::from_secs(5)).await;
            false
        },
    ));
    // 写入会再次进入事件循环，测试用 PTY 随后发送 Shutdown。
    running
        .control_tx
        .send(Message::Input(vec![0x03].into()))
        .unwrap();
    running.thread.join().unwrap();

    assert!(flushed, "同步输出应在截止时间后刷新");
    assert!(
        running
            .terminal
            .lock()
            .block_list()
            .active_block()
            .contents_to_string()
            .contains("sync-frame")
    );
    assert_eq!(
        std::iter::from_fn(|| events_rx.try_recv().ok())
            .filter(is_sync_output_finished)
            .count(),
        0,
        "后续轮询不应重复刷新已经结束的同步帧"
    );
}
