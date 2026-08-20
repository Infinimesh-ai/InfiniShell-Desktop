use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::anyhow;
use futures::io::{AsyncReadExt as _, AsyncWriteExt as _};
use remote_server::auth::RemoteServerAuthContext;
use remote_server::client::{ParentConnectionHandle, RemoteServerClient};
use remote_server::manager::RemoteServerExitStatus;
use remote_server::proto::{RegisterSshControl, RegisterSshTransport, SshStreamPurpose};
use remote_server::setup::{
    PreinstallCheckResult, RemoteOs, RemotePlatform, parse_platform_output,
};
use remote_server::transport::{
    Connection, ControlPath, Error, InstallOutcome, InstallSource, RemoteTransport,
};
use warp_core::SessionId;
use warpui::r#async::{FutureExt as _, executor};

#[derive(Debug)]
struct RoutedCommandOutput {
    stdout: Vec<u8>,
    stderr: String,
    exit_code: Option<i32>,
}

/// 通过父级 remote-server 上注册的 SSH transport 访问下一跳。
#[derive(Clone)]
pub struct RemoteSshTransport {
    parent_session_id: SessionId,
    parent_connection: ParentConnectionHandle,
    registration: RemoteSshTransportRegistration,
    routed_control: Arc<Mutex<RoutedControl>>,
    auth_context: Arc<RemoteServerAuthContext>,
    warp_owns_control_master: bool,
    remote_os: RemoteOs,
}

#[derive(Clone)]
pub(crate) enum RemoteSshTransportRegistration {
    ControlMaster(RegisterSshControl),
    CrossPlatform(RegisterSshTransport),
}

impl RemoteSshTransportRegistration {
    pub(crate) async fn register(
        self,
        client: &RemoteServerClient,
    ) -> Result<String, remote_server::client::ClientError> {
        match self {
            Self::ControlMaster(registration) => client.register_ssh_control(registration).await,
            Self::CrossPlatform(registration) => client.register_ssh_transport(registration).await,
        }
    }
}

struct RoutedControl {
    parent_client: Arc<RemoteServerClient>,
    control_id: String,
}

impl std::fmt::Debug for RemoteSshTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RemoteSshTransport")
            .field("parent_session_id", &self.parent_session_id)
            .field("warp_owns_control_master", &self.warp_owns_control_master)
            .finish_non_exhaustive()
    }
}

impl RemoteSshTransport {
    pub fn new(
        parent_session_id: SessionId,
        parent_connection: ParentConnectionHandle,
        parent_client: Arc<RemoteServerClient>,
        control_id: String,
        registration: RemoteSshTransportRegistration,
        auth_context: Arc<RemoteServerAuthContext>,
        warp_owns_control_master: bool,
        remote_os: RemoteOs,
    ) -> Self {
        Self {
            parent_session_id,
            parent_connection,
            registration,
            routed_control: Arc::new(Mutex::new(RoutedControl {
                parent_client,
                control_id,
            })),
            auth_context,
            warp_owns_control_master,
            remote_os,
        }
    }

    async fn run_command(
        parent_client: &RemoteServerClient,
        control_id: &str,
        purpose: SshStreamPurpose,
        timeout: Duration,
    ) -> Result<RoutedCommandOutput, Error> {
        let operation = async {
            let mut stream = parent_client
                .open_ssh_stream(control_id.to_string(), purpose, String::new(), 0)
                .await
                .map_err(|error| Error::Other(error.into()))?;
            stream
                .close()
                .await
                .map_err(|error| Error::Other(error.into()))?;
            let mut stdout = Vec::new();
            stream
                .read_to_end(&mut stdout)
                .await
                .map_err(|error| Error::Other(error.into()))?;
            let exit_code = stream.exit_status().and_then(|exit| exit.exit_code);
            Ok(RoutedCommandOutput {
                stdout,
                stderr: stream.stderr_tail(),
                exit_code,
            })
        };
        operation
            .with_timeout(timeout)
            .await
            .map_err(|_| Error::TimedOut)?
    }

    async fn run(
        &self,
        purpose: SshStreamPurpose,
        timeout: Duration,
    ) -> Result<RoutedCommandOutput, Error> {
        let (parent_client, control_id) = self.current_route().await?;
        Self::run_command(&parent_client, &control_id, purpose, timeout).await
    }

    async fn current_route(&self) -> Result<(Arc<RemoteServerClient>, String), Error> {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let parent_client = loop {
            if let Some(client) = self.parent_connection.client() {
                break client;
            }
            if std::time::Instant::now() >= deadline {
                return Err(Error::Other(anyhow!(
                    "parent SSH route is not connected: session={:?}",
                    self.parent_connection.session_id()
                )));
            }
            async_io::Timer::after(Duration::from_millis(100)).await;
        };
        if let Some(control_id) = {
            let route = self
                .routed_control
                .lock()
                .expect("routed SSH control mutex poisoned");
            Arc::ptr_eq(&route.parent_client, &parent_client).then(|| route.control_id.clone())
        } {
            return Ok((parent_client, control_id));
        }

        let control_id = self
            .registration
            .clone()
            .register(&parent_client)
            .await
            .map_err(|error| Error::Other(error.into()))?;
        let Some(current_parent) = self.parent_connection.client() else {
            return Err(Error::Other(anyhow!(
                "parent SSH route disconnected during registration"
            )));
        };
        if !Arc::ptr_eq(&current_parent, &parent_client) {
            return Err(Error::Other(anyhow!(
                "parent SSH route changed during registration"
            )));
        }
        *self
            .routed_control
            .lock()
            .expect("routed SSH control mutex poisoned") = RoutedControl {
            parent_client: Arc::clone(&parent_client),
            control_id: control_id.clone(),
        };
        Ok((parent_client, control_id))
    }

    async fn install_from_client_tarball(&self) -> Result<(), Error> {
        let platform = self.detect_platform().await?;
        let tarball =
            crate::remote_server::ssh_transport::installation::client_tarball_for_platform(
                &platform,
            )
            .await?;
        async {
            let mut file = async_fs::File::open(&tarball)
                .await
                .map_err(|error| Error::Other(error.into()))?;
            let archive_size = file
                .metadata()
                .await
                .map_err(|error| Error::Other(error.into()))?
                .len();
            let (parent_client, control_id) = self.current_route().await?;
            let mut stream = parent_client
                .open_ssh_stream(
                    control_id,
                    SshStreamPurpose::StageBinary,
                    String::new(),
                    archive_size,
                )
                .await
                .map_err(|error| Error::Other(error.into()))?;
            futures_lite::io::copy(&mut file, &mut stream)
                .await
                .map_err(|error| Error::Other(error.into()))?;
            stream
                .close()
                .await
                .map_err(|error| Error::Other(error.into()))?;
            let mut output = Vec::new();
            stream
                .read_to_end(&mut output)
                .await
                .map_err(|error| Error::Other(error.into()))?;
            if stream.exit_status().and_then(|exit| exit.exit_code) != Some(0) {
                return Err(Error::Other(anyhow!(
                    "remote-server tarball staging failed"
                )));
            }
            Ok(())
        }
        .with_timeout(remote_server::setup::SCP_INSTALL_TIMEOUT)
        .await
        .map_err(|_| Error::TimedOut)??;

        let output = self
            .run(
                SshStreamPurpose::InstallStagedBinary,
                remote_server::setup::SCP_INSTALL_TIMEOUT,
            )
            .await?;
        if output.exit_code == Some(0) {
            Ok(())
        } else {
            Err(Error::ScriptFailed {
                exit_code: output.exit_code.unwrap_or(-1),
                stderr: output.stderr,
            })
        }
    }
}

impl RemoteTransport for RemoteSshTransport {
    fn detect_platform(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<RemotePlatform, Error>> + Send>> {
        let transport = self.clone();
        Box::pin(async move {
            let output = transport
                .run(
                    SshStreamPurpose::DetectPlatform,
                    remote_server::setup::CHECK_TIMEOUT,
                )
                .await?;
            if output.exit_code == Some(0) {
                parse_platform_output(&String::from_utf8_lossy(&output.stdout))
            } else {
                Err(Error::Other(anyhow!(
                    "platform probe exited with {:?}: {}",
                    output.exit_code,
                    output.stderr
                )))
            }
        })
    }

    fn run_preinstall_check(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<PreinstallCheckResult, Error>> + Send>> {
        let transport = self.clone();
        Box::pin(async move {
            if transport.remote_os == RemoteOs::Windows {
                return Ok(PreinstallCheckResult::parse(""));
            }
            let output = transport
                .run(
                    SshStreamPurpose::PreinstallCheck,
                    remote_server::setup::CHECK_TIMEOUT,
                )
                .await?;
            if output.exit_code == Some(0) {
                Ok(PreinstallCheckResult::parse(&String::from_utf8_lossy(
                    &output.stdout,
                )))
            } else {
                Err(Error::ScriptFailed {
                    exit_code: output.exit_code.unwrap_or(-1),
                    stderr: output.stderr,
                })
            }
        })
    }

    fn check_binary(&self) -> Pin<Box<dyn Future<Output = Result<bool, Error>> + Send>> {
        let transport = self.clone();
        Box::pin(async move {
            let output = transport
                .run(
                    SshStreamPurpose::CheckBinary,
                    remote_server::setup::CHECK_TIMEOUT,
                )
                .await?;
            match output.exit_code {
                Some(0) => Ok(true),
                Some(126) | Some(127) => Ok(false),
                Some(exit_code) => Err(Error::Other(anyhow!(
                    "binary check exited with {exit_code}: {}",
                    output.stderr
                ))),
                None => Err(Error::Other(anyhow!("binary check terminated by signal"))),
            }
        })
    }

    fn check_has_old_binary(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send>> {
        let transport = self.clone();
        Box::pin(async move {
            let output = transport
                .run(
                    SshStreamPurpose::CheckOldBinary,
                    remote_server::setup::CHECK_TIMEOUT,
                )
                .await
                .map_err(|error| anyhow!(error))?;
            match output.exit_code {
                Some(0) => Ok(true),
                Some(1) => Ok(false),
                Some(exit_code) => Err(anyhow!(
                    "remote-server directory check exited with {exit_code}: {}",
                    output.stderr
                )),
                None => Err(anyhow!(
                    "remote-server directory check terminated by signal"
                )),
            }
        })
    }

    fn install_binary(&self) -> Pin<Box<dyn Future<Output = InstallOutcome> + Send>> {
        let transport = self.clone();
        Box::pin(async move {
            let output = transport
                .run(
                    SshStreamPurpose::InstallBinary,
                    remote_server::setup::INSTALL_TIMEOUT,
                )
                .await;
            let (source, result) = match output {
                Ok(output) if output.exit_code == Some(0) => (InstallSource::Server, Ok(())),
                Ok(output) if output.exit_code != Some(2) => (
                    InstallSource::Client,
                    transport.install_from_client_tarball().await,
                ),
                Ok(output) => (
                    InstallSource::Server,
                    Err(Error::ScriptFailed {
                        exit_code: output.exit_code.unwrap_or(-1),
                        stderr: output.stderr,
                    }),
                ),
                Err(error) => (InstallSource::Server, Err(error)),
            };
            let result = if result.is_ok() {
                transport.check_binary().await.and_then(|installed| {
                    installed
                        .then_some(())
                        .ok_or_else(|| Error::Other(anyhow!("installed binary is unavailable")))
                })
            } else {
                result
            };
            InstallOutcome {
                source: Some(source),
                result,
            }
        })
    }

    fn connect(
        &self,
        executor: Arc<executor::Background>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Connection>> + Send>> {
        let transport = self.clone();
        Box::pin(async move {
            let (parent_client, control_id) = transport.current_route().await?;
            let stream = parent_client
                .open_ssh_stream(
                    control_id.clone(),
                    SshStreamPurpose::RemoteServerProxy,
                    transport.auth_context.remote_server_identity_key(),
                    0,
                )
                .await?;
            let resource = stream.connection_handle();
            let (reader, writer) = stream.split();
            let (client, event_rx, failure_rx, host_response_rx) =
                RemoteServerClient::new(reader, writer, &executor);
            Ok(Connection {
                client,
                event_rx,
                failure_rx,
                host_response_rx,
                resource: Box::new(resource),
                control_path: ControlPath::Remote {
                    client: parent_client,
                    control_id,
                    warp_managed: transport.warp_owns_control_master,
                },
            })
        })
    }

    fn remove_remote_server_binary(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>> {
        let transport = self.clone();
        Box::pin(async move {
            let output = transport
                .run(
                    SshStreamPurpose::RemoveBinary,
                    remote_server::setup::CHECK_TIMEOUT,
                )
                .await
                .map_err(|error| anyhow!(error))?;
            if output.exit_code == Some(0) {
                Ok(())
            } else {
                Err(anyhow!(
                    "failed to remove remote server binary: {}",
                    output.stderr
                ))
            }
        })
    }

    fn is_reconnectable(&self, exit_status: Option<&RemoteServerExitStatus>) -> bool {
        !matches!(exit_status.and_then(|status| status.code), Some(255))
    }
}
