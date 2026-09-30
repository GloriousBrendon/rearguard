//! A minimal async client for the protocol, used by the simulator integration and the
//! tests. A real game client (the Godot binding) will speak the same messages.

use std::io;
use std::net::SocketAddr;

use rearguard_core::protocol::{
    ClientMessage, LENGTH_PREFIX, ServerMessage, decode_payload, encode_frame, payload_length,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Largest server message a client accepts.
const MAX_REPLY: usize = 1 << 20;

/// One connection to a server.
#[derive(Debug)]
pub struct Client {
    stream: TcpStream,
}

impl Client {
    /// Connects to `addr`.
    ///
    /// # Errors
    /// Connection errors.
    pub async fn connect(addr: SocketAddr) -> io::Result<Self> {
        Ok(Self {
            stream: TcpStream::connect(addr).await?,
        })
    }

    /// Sends one message and waits for the reply.
    ///
    /// # Errors
    /// I/O errors, or an undecodable reply.
    pub async fn request(&mut self, message: &ClientMessage) -> io::Result<ServerMessage> {
        self.send_raw(&encode_frame(message).map_err(io::Error::other)?)
            .await?;
        self.receive().await
    }

    /// Writes raw bytes (for tests that send malformed frames).
    ///
    /// # Errors
    /// I/O errors.
    pub async fn send_raw(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.stream.write_all(bytes).await
    }

    /// Reads one server message.
    ///
    /// # Errors
    /// I/O errors (including the server closing the connection), or an undecodable
    /// reply.
    pub async fn receive(&mut self) -> io::Result<ServerMessage> {
        let mut prefix = [0u8; LENGTH_PREFIX];
        self.stream.read_exact(&mut prefix).await?;
        let length = payload_length(prefix, MAX_REPLY).map_err(io::Error::other)?;
        let mut payload = vec![0u8; length];
        self.stream.read_exact(&mut payload).await?;
        decode_payload(&payload).map_err(io::Error::other)
    }

    /// Whether the server has closed the connection (reads end of stream).
    pub async fn is_closed(&mut self) -> bool {
        let mut byte = [0u8; 1];
        matches!(self.stream.read(&mut byte).await, Ok(0) | Err(_))
    }
}
