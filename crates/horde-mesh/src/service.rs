//! Invite-bounded mesh service: listen/dial, capability ads, whole-unit job RPC.
//! Optional public DHT is an explicit opt-in stub (default off).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use horde_node::IdentityKeys;
use horde_protocol::{
    BootstrapPeer, CapabilityAdvert, JobCreateRequest, JobId, JobResult, MeshMessage, MeshPeerInfo,
    NodeId, ProtocolError, ProtocolResult, PublicDhtConfig, PublicDhtStub, PublicKeyBytes,
    ResourceProofChallenge, ResourceProofResponse, PROTOCOL_VERSION,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::blocklist::LocalBlocklist;
use crate::channel::{NoiseHandshake, NoiseKeypair, SharedNoiseChannel};

/// Host hooks the mesh calls for invite checks and local job execution.
#[async_trait]
pub trait MeshHost: Send + Sync {
    async fn validate_mesh_invite(&self, token: &str) -> ProtocolResult<()>;
    async fn local_capability(&self) -> CapabilityAdvert;
    /// Execute an inbound mesh job on this collective (stub GPU OK).
    async fn execute_mesh_job(&self, request: JobCreateRequest) -> ProtocolResult<JobResult>;
    /// Soft reputation: we completed work for this invite-trusted peer (Phase D).
    async fn note_outbound_work(&self, peer_id: &str);
    /// Soft reputation: decline pure consumers after local fair-share threshold.
    async fn should_decline_consumer(&self, peer_id: &str) -> bool;
    /// Rank capable peer ids by local fair-share + priority hints (Phase D).
    async fn rank_mesh_peers(&self, peer_ids: &[String], job_priority_hint: Option<u8>)
        -> Vec<String>;
}

#[derive(Clone)]
struct PeerSession {
    info: MeshPeerInfo,
    channel: SharedNoiseChannel,
}

struct PendingJob {
    tx: oneshot::Sender<ProtocolResult<JobResult>>,
}

/// Invite-bounded P2P mesh for one collective head.
pub struct MeshService {
    identity: IdentityKeys,
    noise_kp: NoiseKeypair,
    /// Configured / actual listen address (updated to `local_addr` after bind, including `:0`).
    bind: std::sync::RwLock<SocketAddr>,
    host: Arc<dyn MeshHost>,
    peers: RwLock<HashMap<String, PeerSession>>,
    /// Node IDs previously admitted via invite (reconnect without new invite).
    trusted: RwLock<HashMap<String, PublicKeyBytes>>,
    pub blocklist: LocalBlocklist,
    bootstrap: RwLock<Vec<BootstrapPeer>>,
    dht: Mutex<PublicDhtStub>,
    pending_jobs: Mutex<HashMap<JobId, PendingJob>>,
    listener: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct MeshStatus {
    pub bind: String,
    pub node_id: String,
    pub peer_count: usize,
    pub peers: Vec<MeshPeerInfo>,
    pub bootstrap: Vec<BootstrapPeer>,
    pub blocked: Vec<String>,
    /// Public DHT remains off unless explicitly opted in.
    pub public_dht: PublicDhtConfig,
}

impl MeshService {
    pub fn new(
        identity: IdentityKeys,
        bind: SocketAddr,
        host: Arc<dyn MeshHost>,
    ) -> anyhow::Result<Arc<Self>> {
        let noise_kp = NoiseKeypair::generate()?;
        Ok(Arc::new(Self {
            identity,
            noise_kp,
            bind: std::sync::RwLock::new(bind),
            host,
            peers: RwLock::new(HashMap::new()),
            trusted: RwLock::new(HashMap::new()),
            blocklist: LocalBlocklist::new(),
            bootstrap: RwLock::new(Vec::new()),
            dht: Mutex::new(PublicDhtStub::new(PublicDhtConfig::from_env())),
            pending_jobs: Mutex::new(HashMap::new()),
            listener: Mutex::new(None),
        }))
    }

    pub fn bind_addr(&self) -> SocketAddr {
        *self
            .bind
            .read()
            .unwrap_or_else(|e| e.into_inner())
    }

    pub fn node_fingerprint(&self) -> String {
        self.identity.node_id.fingerprint()
    }

    pub async fn set_bootstrap(&self, peers: Vec<BootstrapPeer>) {
        *self.bootstrap.write().await = peers;
    }

    pub async fn bootstrap_peers(&self) -> Vec<BootstrapPeer> {
        self.bootstrap.read().await.clone()
    }

    pub async fn status(self: &Arc<Self>) -> MeshStatus {
        let peers: Vec<_> = self
            .peers
            .read()
            .await
            .values()
            .map(|p| p.info.clone())
            .collect();
        let blocked = self.blocklist.snapshot().await.blocked_node_ids;
        let public_dht = self.dht.lock().await.config().clone();
        MeshStatus {
            bind: self.bind_addr().to_string(),
            node_id: self.node_fingerprint(),
            peer_count: peers.len(),
            peers,
            bootstrap: self.bootstrap.read().await.clone(),
            blocked,
            public_dht,
        }
    }

    pub async fn dht_config(&self) -> PublicDhtConfig {
        self.dht.lock().await.config().clone()
    }

    pub async fn set_dht_config(&self, config: PublicDhtConfig) -> ProtocolResult<()> {
        self.dht.lock().await.set_config(config)
    }

    pub async fn dht_challenge(&self) -> ProtocolResult<ResourceProofChallenge> {
        self.dht.lock().await.issue_challenge()
    }

    pub async fn dht_lookup(
        &self,
        key: &str,
        proof: Option<ResourceProofResponse>,
    ) -> ProtocolResult<Vec<String>> {
        self.dht.lock().await.lookup(key, proof.as_ref())
    }

    pub async fn list_peers(&self) -> Vec<MeshPeerInfo> {
        self.peers
            .read()
            .await
            .values()
            .map(|p| p.info.clone())
            .collect()
    }

    /// Start accepting inbound mesh connections on the configured bind address.
    ///
    /// After a successful bind, `bind_addr()` reflects the OS-assigned address
    /// (so `127.0.0.1:0` becomes an ephemeral port).
    pub async fn start(self: &Arc<Self>) -> anyhow::Result<()> {
        let mut slot = self.listener.lock().await;
        if slot.is_some() {
            anyhow::bail!("mesh listener already running");
        }
        let configured = self.bind_addr();
        let listener = TcpListener::bind(configured).await.map_err(|e| {
            anyhow::anyhow!("mesh bind {configured} failed: {e}")
        })?;
        let actual = listener.local_addr()?;
        *self
            .bind
            .write()
            .unwrap_or_else(|e| e.into_inner()) = actual;
        info!(%actual, "mesh listening (Noise XX)");
        let this = Arc::clone(self);
        let handle = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, addr)) => {
                        let this = Arc::clone(&this);
                        tokio::spawn(async move {
                            if let Err(err) = this.handle_inbound(stream, addr).await {
                                warn!(%addr, error = %err, "inbound mesh session failed");
                            }
                        });
                    }
                    Err(err) => {
                        warn!(error = %err, "mesh accept error");
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            }
        });
        *slot = Some(handle);
        Ok(())
    }

    /// Stop the accept loop (existing peer sessions stay up until they drop).
    pub async fn stop_listener(&self) {
        if let Some(handle) = self.listener.lock().await.take() {
            handle.abort();
            // Let the aborted task drop its TcpListener before a rebind.
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Runtime rebind: abort the accept loop and listen on `bind` without rebuilding
    /// peer/trust/blocklist/bootstrap/DHT state. Does not restart the HTTP API process.
    pub async fn rebind(self: &Arc<Self>, bind: SocketAddr) -> anyhow::Result<SocketAddr> {
        self.stop_listener().await;
        *self
            .bind
            .write()
            .unwrap_or_else(|e| e.into_inner()) = bind;
        self.start().await?;
        Ok(self.bind_addr())
    }

    /// Dial a peer and present a mesh invite (required unless already trusted).
    pub async fn dial_with_invite(
        self: &Arc<Self>,
        addr: &str,
        invite_token: Option<&str>,
    ) -> ProtocolResult<MeshPeerInfo> {
        let sock: SocketAddr = addr
            .parse()
            .map_err(|e| ProtocolError::Other(format!("bad peer addr: {e}")))?;
        let stream = TcpStream::connect(sock)
            .await
            .map_err(|e| ProtocolError::Other(format!("connect failed: {e}")))?;
        let mut channel = NoiseHandshake::handshake_initiator(stream, &self.noise_kp)
            .await
            .map_err(|e| ProtocolError::Other(format!("noise handshake: {e}")))?;

        self.send_hello(&mut channel)
            .await
            .map_err(|e| ProtocolError::Other(e.to_string()))?;
        let peer_hello = self
            .expect_hello(&mut channel)
            .await
            .map_err(|e| ProtocolError::Other(e.to_string()))?;
        let peer_id = NodeId::from_public_key(peer_hello).fingerprint();

        if self.blocklist.is_blocked(&peer_id).await {
            return Err(ProtocolError::PeerBlocked);
        }

        let already_trusted = self.trusted.read().await.contains_key(&peer_id);
        if !already_trusted {
            let token = invite_token.ok_or(ProtocolError::MeshInviteRequired)?;
            channel
                .send_json(&MeshMessage::InvitePresent {
                    token: token.to_string(),
                })
                .await
                .map_err(|e| ProtocolError::Other(e.to_string()))?;
            match channel
                .recv_json::<MeshMessage>()
                .await
                .map_err(|e| ProtocolError::Other(e.to_string()))?
            {
                MeshMessage::InviteOk => {}
                MeshMessage::InviteDenied { reason } => {
                    return Err(ProtocolError::Other(format!("invite denied: {reason}")));
                }
                other => {
                    return Err(ProtocolError::Other(format!(
                        "expected InviteOk, got {other:?}"
                    )));
                }
            }
        }

        self.finish_session(channel, peer_hello, sock.to_string())
            .await
    }

    /// Offer a whole-unit job to a connected peer; wait for attested result.
    pub async fn offer_job(
        self: &Arc<Self>,
        peer_id: &str,
        job_id: JobId,
        request: JobCreateRequest,
    ) -> ProtocolResult<JobResult> {
        if self.blocklist.is_blocked(peer_id).await {
            return Err(ProtocolError::PeerBlocked);
        }
        let session = self
            .peers
            .read()
            .await
            .get(peer_id)
            .cloned()
            .ok_or(ProtocolError::PeerNotFound)?;

        if let Some(cap) = &session.info.capability {
            if !cap.supports_fingerprint(&request.fingerprint) {
                return Err(ProtocolError::CapabilityMismatch(format!(
                    "peer lacks {}",
                    request.fingerprint.compact()
                )));
            }
        }

        let (tx, rx) = oneshot::channel();
        self.pending_jobs
            .lock()
            .await
            .insert(job_id, PendingJob { tx });

        if let Err(e) = session
            .channel
            .send_json(&MeshMessage::JobOffer {
                job_id,
                request: request.clone(),
            })
            .await
        {
            self.pending_jobs.lock().await.remove(&job_id);
            return Err(ProtocolError::Other(format!("send job offer: {e}")));
        }

        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(ProtocolError::Other("job channel closed".into())),
            Err(_) => {
                self.pending_jobs.lock().await.remove(&job_id);
                Err(ProtocolError::Other("mesh job timed out".into()))
            }
        }
    }

    /// List connected peers that advertise the fingerprint (blocklist filtered).
    pub async fn list_capable_peers(&self, request: &JobCreateRequest) -> Vec<String> {
        let peers = self.peers.read().await;
        let mut capable = Vec::new();
        for (id, session) in peers.iter() {
            if self.blocklist.is_blocked(id).await {
                continue;
            }
            if let Some(cap) = &session.info.capability {
                if cap.supports_fingerprint(&request.fingerprint) {
                    capable.push(id.clone());
                }
            }
        }
        capable
    }

    /// Pick a connected peer by fair-share / priority ranking among capable peers (Phase D).
    pub async fn find_capable_peer(
        self: &Arc<Self>,
        request: &JobCreateRequest,
    ) -> ProtocolResult<String> {
        let capable = self.list_capable_peers(request).await;
        if capable.is_empty() {
            return Err(ProtocolError::NoMeshPeer);
        }
        let ranked = self
            .host
            .rank_mesh_peers(&capable, request.priority_hint)
            .await;
        // Skip pure consumers when alternatives exist.
        for id in &ranked {
            if self.host.should_decline_consumer(id).await && ranked.len() > 1 {
                continue;
            }
            return Ok(id.clone());
        }
        ranked
            .into_iter()
            .next()
            .ok_or(ProtocolError::NoMeshPeer)
    }

    pub async fn block_peer(self: &Arc<Self>, node_id: &str) {
        self.blocklist.block(node_id).await;
        self.peers.write().await.remove(node_id);
        self.trusted.write().await.remove(node_id);
    }

    async fn handle_inbound(
        self: Arc<Self>,
        stream: TcpStream,
        addr: SocketAddr,
    ) -> anyhow::Result<()> {
        let mut channel = NoiseHandshake::handshake_responder(stream, &self.noise_kp).await?;
        self.send_hello(&mut channel).await?;
        let peer_hello = self.expect_hello(&mut channel).await?;
        let peer_id = NodeId::from_public_key(peer_hello).fingerprint();

        if self.blocklist.is_blocked(&peer_id).await {
            channel
                .send_json(&MeshMessage::InviteDenied {
                    reason: "blocked".into(),
                })
                .await
                .ok();
            return Err(anyhow::anyhow!("peer blocked"));
        }

        let already_trusted = self.trusted.read().await.contains_key(&peer_id);
        if !already_trusted {
            match channel.recv_json::<MeshMessage>().await? {
                MeshMessage::InvitePresent { token } => {
                    match self.host.validate_mesh_invite(&token).await {
                        Ok(()) => {
                            channel.send_json(&MeshMessage::InviteOk).await?;
                        }
                        Err(err) => {
                            channel
                                .send_json(&MeshMessage::InviteDenied {
                                    reason: err.to_string(),
                                })
                                .await?;
                            return Err(anyhow::anyhow!("invite denied"));
                        }
                    }
                }
                _ => {
                    channel
                        .send_json(&MeshMessage::InviteDenied {
                            reason: "mesh invite required".into(),
                        })
                        .await?;
                    return Err(anyhow::anyhow!("missing invite"));
                }
            }
        }

        let _ = self
            .finish_session(channel, peer_hello, addr.to_string())
            .await?;
        Ok(())
    }

    async fn finish_session(
        self: &Arc<Self>,
        mut channel: NoiseHandshake,
        peer_key: PublicKeyBytes,
        addr: String,
    ) -> ProtocolResult<MeshPeerInfo> {
        let peer_id = NodeId::from_public_key(peer_key).fingerprint();

        // Capability exchange still on the unsplit handshake socket (serialized).
        let local_cap = self.host.local_capability().await;
        channel
            .send_json(&MeshMessage::Capability {
                advert: local_cap,
            })
            .await
            .map_err(|e| ProtocolError::Other(e.to_string()))?;

        let peer_cap = match channel
            .recv_json::<MeshMessage>()
            .await
            .map_err(|e| ProtocolError::Other(e.to_string()))?
        {
            MeshMessage::Capability { advert } => Some(advert),
            other => {
                return Err(ProtocolError::Other(format!(
                    "expected capability, got {other:?}"
                )));
            }
        };

        let shared = channel.into_split();

        let info = MeshPeerInfo {
            node_id: peer_id.clone(),
            public_key: peer_key,
            addr: addr.clone(),
            capability: peer_cap,
            connected: true,
        };

        self.trusted
            .write()
            .await
            .insert(peer_id.clone(), peer_key);
        self.peers.write().await.insert(
            peer_id.clone(),
            PeerSession {
                info: info.clone(),
                channel: shared.clone_handle(),
            },
        );

        info!(peer = %peer_id, %addr, "mesh peer session established");

        let this = Arc::clone(self);
        let reader = shared.clone_handle();
        let peer_for_task = peer_id.clone();
        tokio::spawn(async move {
            let cleanup = Arc::clone(&this);
            if let Err(err) = this.read_loop(peer_for_task.clone(), reader).await {
                warn!(peer = %peer_for_task, error = %err, "mesh read loop ended");
            }
            cleanup.peers.write().await.remove(&peer_for_task);
        });

        Ok(info)
    }

    async fn read_loop(
        self: Arc<Self>,
        peer_id: String,
        channel: SharedNoiseChannel,
    ) -> anyhow::Result<()> {
        loop {
            let msg: MeshMessage = channel.recv_json().await?;
            match msg {
                MeshMessage::Ping => {
                    channel.send_json(&MeshMessage::Pong).await?;
                }
                MeshMessage::Capability { advert } => {
                    if let Some(session) = self.peers.write().await.get_mut(&peer_id) {
                        session.info.capability = Some(advert);
                    }
                }
                MeshMessage::CapabilityQuery => {
                    let advert = self.host.local_capability().await;
                    channel
                        .send_json(&MeshMessage::Capability { advert })
                        .await?;
                }
                MeshMessage::JobOffer { job_id, request } => {
                    if self.blocklist.is_blocked(&peer_id).await {
                        channel
                            .send_json(&MeshMessage::JobDecline {
                                job_id,
                                reason: "blocked".into(),
                            })
                            .await?;
                        continue;
                    }
                    if self.host.should_decline_consumer(&peer_id).await {
                        channel
                            .send_json(&MeshMessage::JobDecline {
                                job_id,
                                reason: "fair-share: peer ignored as pure consumer".into(),
                            })
                            .await?;
                        continue;
                    }
                    match self.host.execute_mesh_job(request).await {
                        Ok(result) => {
                            if let Err(err) = result.attestation.verify(&result.output) {
                                channel
                                    .send_json(&MeshMessage::JobFailed {
                                        job_id,
                                        error: format!("local attestation invalid: {err}"),
                                    })
                                    .await?;
                            } else {
                                self.host.note_outbound_work(&peer_id).await;
                                channel
                                    .send_json(&MeshMessage::JobComplete { job_id, result })
                                    .await?;
                            }
                        }
                        Err(err) => {
                            channel
                                .send_json(&MeshMessage::JobFailed {
                                    job_id,
                                    error: err.to_string(),
                                })
                                .await?;
                        }
                    }
                }
                MeshMessage::JobComplete { job_id, result } => {
                    let outcome = match result.attestation.verify(&result.output) {
                        Ok(()) => Ok(result),
                        Err(err) => Err(err),
                    };
                    if let Some(pending) = self.pending_jobs.lock().await.remove(&job_id) {
                        let _ = pending.tx.send(outcome);
                    }
                }
                MeshMessage::JobFailed { job_id, error } => {
                    if let Some(pending) = self.pending_jobs.lock().await.remove(&job_id) {
                        let _ = pending.tx.send(Err(ProtocolError::Other(error)));
                    }
                }
                MeshMessage::JobDecline { job_id, reason } => {
                    if let Some(pending) = self.pending_jobs.lock().await.remove(&job_id) {
                        let _ = pending
                            .tx
                            .send(Err(ProtocolError::CapabilityMismatch(reason)));
                    }
                }
                other => {
                    warn!(peer = %peer_id, ?other, "unexpected mesh message");
                }
            }
        }
    }

    async fn send_hello(&self, channel: &mut NoiseHandshake) -> anyhow::Result<()> {
        let pk = self.identity.public_key_bytes();
        let payload =
            MeshMessage::hello_sign_payload(PROTOCOL_VERSION, &self.identity.node_id.fingerprint());
        let sig = self.identity.sign_bytes(&payload);
        channel
            .send_json(&MeshMessage::hello(self.identity.node_id, pk, sig))
            .await
    }

    async fn expect_hello(&self, channel: &mut NoiseHandshake) -> anyhow::Result<PublicKeyBytes> {
        match channel.recv_json::<MeshMessage>().await? {
            MeshMessage::Hello {
                protocol_version,
                node_id,
                public_key,
                identity_sig_hex,
            } => {
                if protocol_version.split('.').next() != PROTOCOL_VERSION.split('.').next() {
                    return Err(anyhow::anyhow!("protocol major mismatch"));
                }
                if node_id.0 != public_key {
                    return Err(anyhow::anyhow!("node_id/public_key mismatch"));
                }
                let vk = VerifyingKey::from_bytes(&public_key.0)?;
                let payload =
                    MeshMessage::hello_sign_payload(&protocol_version, &node_id.fingerprint());
                let sig_bytes = hex::decode(identity_sig_hex)?;
                let sig = Signature::from_slice(&sig_bytes)?;
                vk.verify(&payload, &sig)
                    .map_err(|_| anyhow::anyhow!("bad identity signature"))?;
                Ok(public_key)
            }
            other => Err(anyhow::anyhow!("expected Hello, got {other:?}")),
        }
    }
}
