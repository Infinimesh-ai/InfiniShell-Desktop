use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;

use mio::unix::SourceFd;
use parking_lot::{FairMutex, Mutex};

use super::{EventLoop, MAX_LOCKED_READ, PTY_TOKEN, SIGNALS_TOKEN};
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::local_tty::{ChildEvent, EventedPty, EventedReadWrite, mio_channel};
use crate::terminal::writeable_pty::Message;
use crate::terminal::{SizeInfo, TerminalModel};

#[derive(Default)]
struct PtyObservation {
    read_count: usize,
    written: Vec<u8>,
    killed: bool,
}

struct ContinuouslyReadablePty {
    readiness: UnixStream,
    peer: UnixStream,
    control_tx: mio_channel::Sender<Message>,
    message_after_first_read: Option<Message>,
    would_block_on_first_write: bool,
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
        drop(observation);

        if let Some(message) = self.message_after_first_read.take() {
            self.control_tx.send(message).unwrap();
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

#[test]
fn continuous_output_preserves_child_exit_without_killing_again() {
    let observation = run_with_continuous_output(Message::ChildExited, false);
    let observation = observation.lock();

    assert_eq!(observation.read_count, 1);
    assert!(!observation.killed);
}
