//! Length-prefixed Noise_XX_25519_ChaChaPoly_BLAKE2s over TCP.
//!
//! Chosen over QUIC for a lean Phase B loopback/LAN demo while still satisfying
//! the locked “encrypted authenticated peer channels” requirement.
//!
//! After handshake the TCP stream is split so encrypt+write and read+decrypt
//! can proceed concurrently (Noise send/recv nonces are independent).

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use snow::{Builder, TransportState};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

const NOISE_PARAMS: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
const MAX_FRAME: usize = 65_535;

pub struct NoiseKeypair {
    pub private: Vec<u8>,
    #[allow(dead_code)]
    pub public: Vec<u8>,
}

impl NoiseKeypair {
    pub fn generate() -> Result<Self> {
        let params = NOISE_PARAMS.parse().context("noise params")?;
        let kp = Builder::new(params).generate_keypair()?;
        Ok(Self {
            private: kp.private,
            public: kp.public,
        })
    }
}

/// Mid-handshake channel (still owns the full `TcpStream`).
pub struct NoiseHandshake {
    stream: TcpStream,
    transport: TransportState,
}

impl NoiseHandshake {
    pub async fn handshake_initiator(
        mut stream: TcpStream,
        static_key: &NoiseKeypair,
    ) -> Result<Self> {
        let params = NOISE_PARAMS.parse()?;
        let mut state = Builder::new(params)
            .local_private_key(&static_key.private)
            .build_initiator()?;
        let mut buf = vec![0u8; MAX_FRAME];

        let n = state.write_message(&[], &mut buf)?;
        write_frame_stream(&mut stream, &buf[..n]).await?;

        let msg = read_frame_stream(&mut stream).await?;
        state.read_message(&msg, &mut buf)?;

        let n = state.write_message(&[], &mut buf)?;
        write_frame_stream(&mut stream, &buf[..n]).await?;

        let transport = state
            .into_transport_mode()
            .map_err(|e| anyhow!("noise transport: {e}"))?;
        Ok(Self { stream, transport })
    }

    pub async fn handshake_responder(
        mut stream: TcpStream,
        static_key: &NoiseKeypair,
    ) -> Result<Self> {
        let params = NOISE_PARAMS.parse()?;
        let mut state = Builder::new(params)
            .local_private_key(&static_key.private)
            .build_responder()?;
        let mut buf = vec![0u8; MAX_FRAME];

        let msg = read_frame_stream(&mut stream).await?;
        state.read_message(&msg, &mut buf)?;

        let n = state.write_message(&[], &mut buf)?;
        write_frame_stream(&mut stream, &buf[..n]).await?;

        let msg = read_frame_stream(&mut stream).await?;
        state.read_message(&msg, &mut buf)?;

        let transport = state
            .into_transport_mode()
            .map_err(|e| anyhow!("noise transport: {e}"))?;
        Ok(Self { stream, transport })
    }

    pub async fn send_json<T: serde::Serialize>(&mut self, value: &T) -> Result<()> {
        let plain = serde_json::to_vec(value)?;
        let mut out = vec![0u8; plain.len() + 16 + 32];
        let n = self
            .transport
            .write_message(&plain, &mut out)
            .map_err(|e| anyhow!("encrypt: {e}"))?;
        write_frame_stream(&mut self.stream, &out[..n]).await
    }

    pub async fn recv_json<T: serde::de::DeserializeOwned>(&mut self) -> Result<T> {
        let frame = read_frame_stream(&mut self.stream).await?;
        let mut out = vec![0u8; frame.len()];
        let n = self
            .transport
            .read_message(&frame, &mut out)
            .map_err(|e| anyhow!("decrypt: {e}"))?;
        Ok(serde_json::from_slice(&out[..n])?)
    }

    pub fn into_split(self) -> SharedNoiseChannel {
        let (reader, writer) = self.stream.into_split();
        SharedNoiseChannel {
            reader: Arc::new(tokio::sync::Mutex::new(reader)),
            writer: Arc::new(tokio::sync::Mutex::new(writer)),
            transport: Arc::new(Mutex::new(self.transport)),
        }
    }
}

/// Split send/recv halves so a blocked read does not stall writes (or vice versa).
#[derive(Clone)]
pub struct SharedNoiseChannel {
    reader: Arc<tokio::sync::Mutex<OwnedReadHalf>>,
    writer: Arc<tokio::sync::Mutex<OwnedWriteHalf>>,
    transport: Arc<Mutex<TransportState>>,
}

impl SharedNoiseChannel {
    pub fn clone_handle(&self) -> Self {
        self.clone()
    }

    pub async fn send_json<T: serde::Serialize>(&self, value: &T) -> Result<()> {
        let plain = serde_json::to_vec(value)?;
        let cipher = {
            let mut transport = self
                .transport
                .lock()
                .map_err(|_| anyhow!("transport lock poisoned"))?;
            let mut out = vec![0u8; plain.len() + 16 + 32];
            let n = transport
                .write_message(&plain, &mut out)
                .map_err(|e| anyhow!("encrypt: {e}"))?;
            out.truncate(n);
            out
        };
        let mut writer = self.writer.lock().await;
        write_frame_write(&mut writer, &cipher).await
    }

    pub async fn recv_json<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        let frame = {
            let mut reader = self.reader.lock().await;
            read_frame_read(&mut reader).await?
        };
        let plain = {
            let mut transport = self
                .transport
                .lock()
                .map_err(|_| anyhow!("transport lock poisoned"))?;
            let mut out = vec![0u8; frame.len()];
            let n = transport
                .read_message(&frame, &mut out)
                .map_err(|e| anyhow!("decrypt: {e}"))?;
            out.truncate(n);
            out
        };
        Ok(serde_json::from_slice(&plain)?)
    }
}

async fn write_frame_stream(stream: &mut TcpStream, data: &[u8]) -> Result<()> {
    if data.len() > MAX_FRAME {
        return Err(anyhow!("frame too large"));
    }
    let len = (data.len() as u16).to_be_bytes();
    stream.write_all(&len).await?;
    stream.write_all(data).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_frame_stream(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 2];
    stream.read_exact(&mut len_buf).await?;
    let len = u16::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(anyhow!("frame length exceeds max"));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn write_frame_write(stream: &mut OwnedWriteHalf, data: &[u8]) -> Result<()> {
    if data.len() > MAX_FRAME {
        return Err(anyhow!("frame too large"));
    }
    let len = (data.len() as u16).to_be_bytes();
    stream.write_all(&len).await?;
    stream.write_all(data).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_frame_read(stream: &mut OwnedReadHalf) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 2];
    stream.read_exact(&mut len_buf).await?;
    let len = u16::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(anyhow!("frame length exceeds max"));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}
