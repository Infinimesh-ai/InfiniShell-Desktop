use std::fmt::{Debug, Display};
use std::marker::Unpin;

use futures::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use futures::{AsyncRead, AsyncWrite};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::service::ServiceId;

/// The size of a usize, in bytes.
const USIZE_SIZE: usize = std::mem::size_of::<usize>();

/// Unique "address" for a server/client connection.
///
/// In the case of this local socket implementation, this is a socket address (path on the
/// filesystem). Conceptually, this somewhat similar to an IP address + port.
#[derive(Debug, Clone, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct ConnectionAddress(pub(super) String);

impl ConnectionAddress {
    /// Returns a `ConnectionAddress` containing a path for a socket address.
    pub(super) fn new() -> Self {
        Self(format!("/tmp/warp-ipc-{}.sock", rand::random::<i64>()))
    }
}

impl Display for ConnectionAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for ConnectionAddress {
    fn from(value: String) -> Self {
        ConnectionAddress(value)
    }
}

/// A unique ID for each request message.
///
/// The corresponding response for the request should contain the same ID.
pub(super) type RequestId = Uuid;

/// Trait for arbitrary messages that may be sent across the 'wire' (the socket).
pub trait Message: 'static + Send + Sync + Debug + Clone + DeserializeOwned + Serialize {}
impl<T> Message for T where T: 'static + Send + Sync + Debug + Clone + DeserializeOwned + Serialize {}

/// Request message sent by clients and received by servers.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub(super) struct Request {
    /// A unique ID for the request.
    pub(super) id: RequestId,

    /// The ID of the service to which this request belongs.
    pub(super) service_id: ServiceId,

    /// The actual request payload.
    pub(super) bytes: Vec<u8>,
}

impl Request {
    /// Constructs a `Request`, generating a unique request ID in the process.
    pub(super) fn new(service_id: ServiceId, bytes: Vec<u8>) -> Self {
        Self {
            id: Uuid::new_v4(),
            service_id,
            bytes,
        }
    }

    pub(super) fn id(&self) -> &RequestId {
        &self.id
    }
}

/// Response message sent by servers and received by clients.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub(super) enum Response {
    /// For responses produced "successfully". "Successful" only pertains to the frameworks ability
    /// to successfully execute the `Service` handler and produce a response. `Service`s may
    /// internally implement their own error types/response schemas.
    Success {
        /// The ID of the request for which this is a response.
        request_id: RequestId,

        /// The ID of the service to which this response belongs.
        service_id: ServiceId,

        /// The actual response payload.
        bytes: Vec<u8>,
    },

    /// For responses that failed due to a framework-level issue. For example, the client attempted
    /// to call a service that wasn't registered in the server.
    Failure {
        /// The ID of the request for which this is a response.
        request_id: RequestId,

        error_message: String,
    },
}

impl Response {
    /// Constructs a "success" response for the request with the given `request_id`.
    pub(super) fn success(request_id: RequestId, service_id: ServiceId, bytes: Vec<u8>) -> Self {
        Self::Success {
            request_id,
            service_id,
            bytes,
        }
    }

    /// Constructs a "failure" response for the request with the given `request_id`.
    pub(super) fn failure(request_id: RequestId, error_message: String) -> Self {
        Self::Failure {
            request_id,
            error_message,
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ProtocolError {
    /// An error occurred when serializing the request or response.
    #[error(transparent)]
    Serialization(#[from] bincode::Error),

    /// The connection was dropped.
    #[error(transparent)]
    Disconnected(#[from] std::io::Error),

    /// 对端声明的 frame 超过当前 IPC 使用方选择的上限。
    #[error("IPC frame is {frame_bytes} bytes, exceeding the {max_frame_bytes}-byte limit")]
    FrameTooLarge {
        frame_bytes: usize,
        max_frame_bytes: usize,
    },

    /// frame 长度未超过配置上限，但无法预留接收缓冲区。
    #[error("Failed to reserve {frame_bytes} bytes for an IPC frame: {source}")]
    FrameAllocationFailed {
        frame_bytes: usize,
        #[source]
        source: std::collections::TryReserveError,
    },

    #[error("Unknown error occurred: {0}")]
    Other(String),
}

/// Writes the given message to the given `writer`.
pub(super) async fn send_message<M, W>(
    writer: &mut W,
    message: M,
    max_frame_bytes: Option<usize>,
) -> Result<(), ProtocolError>
where
    M: Message,
    W: AsyncWrite + Unpin,
{
    if let Some(max_frame_bytes) = max_frame_bytes {
        let serialized_size = bincode::serialized_size(&message)?;
        let frame_bytes = usize::try_from(serialized_size).unwrap_or(usize::MAX);
        validate_frame_size(frame_bytes, Some(max_frame_bytes))?;
    }
    let serialized_msg = bincode::serialize(&message)?;

    validate_frame_size(serialized_msg.len(), max_frame_bytes)?;

    // First, add a message "header" - a usize representing the length of the
    // serialized payload, in bytes.
    writer
        .write_all(&serialized_msg.len().to_be_bytes())
        .await?;

    // Next, add the serialized payload itself.
    Ok(writer.write_all(&serialized_msg).await?)
}

/// Reads the next message from the given `reader`.
pub(super) async fn receive_message<M, R>(
    reader: &mut BufReader<R>,
    max_frame_bytes: Option<usize>,
) -> Result<M, ProtocolError>
where
    M: Message,
    R: AsyncRead + Unpin,
{
    // Start by allocating a buffer that is only large enough to receive the
    // message header, to ensure we don't accidentally receive multiple messages
    // in a single read.
    let mut header_buf = [0; USIZE_SIZE];

    // Read the message "header" from the socket.
    reader.read_exact(&mut header_buf[..]).await?;

    // Parse the message header - we convert the bytes back into a usize, which
    // tells us the size of the serialized message, in bytes.  We add the size
    // of a usize to get the total number of bytes we expect to read off the
    // wire.
    let payload_len = usize::from_be_bytes(header_buf);

    // header 由不可信对端控制；必须在预留缓冲区和反序列化前拒绝超限 frame。
    validate_frame_size(payload_len, max_frame_bytes)?;

    // Grow the initial buffer to a sufficient size and read the rest of the
    // message from the socket.
    let mut payload_buf = Vec::new();
    payload_buf
        .try_reserve_exact(payload_len)
        .map_err(|source| ProtocolError::FrameAllocationFailed {
            frame_bytes: payload_len,
            source,
        })?;
    payload_buf.resize(payload_len, 0);
    reader.read_exact(&mut payload_buf).await?;

    // Deserialize the message.
    let message: M = bincode::deserialize(&payload_buf[..])?;
    Ok(message)
}

fn validate_frame_size(
    frame_bytes: usize,
    max_frame_bytes: Option<usize>,
) -> Result<(), ProtocolError> {
    if let Some(max_frame_bytes) = max_frame_bytes
        && frame_bytes > max_frame_bytes
    {
        return Err(ProtocolError::FrameTooLarge {
            frame_bytes,
            max_frame_bytes,
        });
    }
    Ok(())
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
