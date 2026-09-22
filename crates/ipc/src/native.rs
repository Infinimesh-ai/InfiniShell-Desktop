//! This module implements IPC transport on top of the `interprocess` crate, which uses Unix Domain
//! Sockets on Unix platforms and named pipes on Windows under the hood.
#[cfg(unix)]
use std::path::Path;

use async_compat::CompatExt as _;
use futures::{AsyncRead, AsyncWrite};
#[cfg(unix)]
use interprocess::local_socket::GenericFilePath;
#[cfg(windows)]
use interprocess::local_socket::GenericNamespaced;
use interprocess::local_socket::{ListenerOptions, Name, tokio::prelude::*};

use crate::ConnectionAddress;

fn connection_name(connection_address: &ConnectionAddress) -> std::io::Result<Name<'_>> {
    #[cfg(unix)]
    {
        Path::new(&connection_address.0).to_fs_name::<GenericFilePath>()
    }
    #[cfg(windows)]
    {
        connection_address
            .0
            .as_str()
            .to_ns_name::<GenericNamespaced>()
    }
}

pub(crate) mod client {
    use super::*;
    use crate::client::{ClientError, InitializationError, Result};

    /// Returns a tuple containing structs for reading and writing to a local socket, which is the
    /// underlying IPC transport for native (non-wasm) platforms.
    pub async fn connect_client(
        connection_address: ConnectionAddress,
    ) -> Result<(impl AsyncRead + Unpin, impl AsyncWrite + Unpin)> {
        let name = connection_name(&connection_address)
            .map_err(|e| ClientError::Initialization(InitializationError::Io(e)))?;
        let stream = LocalSocketStream::connect(name)
            .compat()
            .await
            .map_err(|e| ClientError::Initialization(InitializationError::Io(e)))?;
        let (reader, writer) = stream.split();
        Ok((reader.compat(), writer.compat()))
    }
}

pub(crate) mod server {
    use super::*;
    use crate::server::{InitializationError, Result, ServerError};

    pub struct ConnectionImpl {
        stream: LocalSocketStream,
    }

    impl ConnectionImpl {
        pub fn into_split(self) -> (impl AsyncRead + Unpin, impl AsyncWrite + Unpin) {
            let (reader, writer) = self.stream.split();
            (reader.compat(), writer.compat())
        }
    }

    pub struct ConnectionListenerImpl {
        listener: LocalSocketListener,
    }

    impl ConnectionListenerImpl {
        pub fn new(connection_address: ConnectionAddress) -> Result<Self> {
            let name = connection_name(&connection_address)
                .map_err(|e| ServerError::Initialization(InitializationError::Io(e)))?;
            let listener = warpui_core::r#async::block_on(
                async move { ListenerOptions::new().name(name).create_tokio() }.compat(),
            )
            .map_err(|e| ServerError::Initialization(InitializationError::Io(e)))?;
            Ok(Self { listener })
        }

        pub async fn accept_connection(&self) -> Result<ConnectionImpl> {
            self.listener
                .accept()
                .compat()
                .await
                .map(|stream| ConnectionImpl { stream })
                .map_err(ServerError::AcceptConnection)
        }
    }
}
