//! Loopback tests: invite gate, reject without invite, cross-node attested job.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use horde_mesh::{MeshHost, MeshService};
use horde_node::{EncryptedKeystore, GpuExecutor, IdentityKeys, PhysicalServer, StubExecutor};
use horde_protocol::{
    CapabilityAdvert, InvitePurpose, InviteRecord, JobCreateRequest, JobRecord, JobResult,
    JobRouting, ModelFingerprint, ProtocolError, ProtocolResult, WorkKind,
};
use tokio::sync::RwLock;

struct TestHost {
    invites: RwLock<HashMap<String, InviteRecord>>,
    keys: IdentityKeys,
    server: Arc<PhysicalServer>,
}

impl TestHost {
    fn new(label: &str) -> Self {
        let keys = IdentityKeys::generate();
        let server = Arc::new(PhysicalServer::new(
            format!("srv-{label}"),
            label.to_string(),
            keys.clone(),
        ));
        Self {
            invites: RwLock::new(HashMap::new()),
            keys,
            server,
        }
    }

    async fn mint_mesh_invite(&self) -> String {
        let token = format!("horde_inv_test_{}", label_token());
        let (_plain, record) =
            InviteRecord::mint(InvitePurpose::MeshJoin, 600, token.clone(), None);
        self.invites
            .write()
            .await
            .insert(InviteRecord::hash_token(&token), record);
        token
    }
}

#[async_trait]
impl MeshHost for TestHost {
    async fn validate_mesh_invite(&self, token: &str) -> ProtocolResult<()> {
        let mut invites = self.invites.write().await;
        let record = invites
            .get_mut(&InviteRecord::hash_token(token))
            .ok_or(ProtocolError::InviteInvalid)?;
        record.verify_plaintext(token)?;
        record.redeemed = true;
        Ok(())
    }

    async fn local_capability(&self) -> CapabilityAdvert {
        self.server.snapshot().await.capability
    }

    async fn execute_mesh_job(&self, request: JobCreateRequest) -> ProtocolResult<JobResult> {
        let job = JobRecord::new(request).map_err(|e| e)?;
        if !self
            .server
            .snapshot()
            .await
            .capability
            .supports_fingerprint(&job.fingerprint)
        {
            return Err(ProtocolError::CapabilityMismatch("fingerprint".into()));
        }
        StubExecutor
            .execute(&job, &self.keys)
            .await
            .map_err(|e| ProtocolError::Other(e.to_string()))
    }

    async fn note_outbound_work(&self, _peer_id: &str) {}

    async fn should_decline_consumer(&self, _peer_id: &str) -> bool {
        false
    }

    async fn rank_mesh_peers(
        &self,
        peer_ids: &[String],
        _job_priority_hint: Option<u8>,
    ) -> Vec<String> {
        peer_ids.to_vec()
    }
}

fn label_token() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_string()
}

fn ephemeral_addr() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

async fn bind_mesh(host: Arc<TestHost>) -> Arc<MeshService> {
    // Bind :0 then rebuild is awkward; pick a free port via TcpListener.
    let listener = tokio::net::TcpListener::bind(ephemeral_addr())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let mesh = MeshService::new(host.keys.clone(), addr, host).unwrap();
    mesh.start().await.unwrap();
    // tiny settle for accept loop
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    mesh
}

#[tokio::test]
async fn reject_connection_without_invite() {
    let host_a = Arc::new(TestHost::new("a"));
    let host_b = Arc::new(TestHost::new("b"));
    let mesh_a = bind_mesh(Arc::clone(&host_a)).await;
    let mesh_b = MeshService::new(
        host_b.keys.clone(),
        {
            let l = tokio::net::TcpListener::bind(ephemeral_addr()).await.unwrap();
            let a = l.local_addr().unwrap();
            drop(l);
            a
        },
        host_b,
    )
    .unwrap();
    mesh_b.start().await.unwrap();

    let err = mesh_b
        .dial_with_invite(&mesh_a.bind_addr().to_string(), None)
        .await
        .expect_err("must require invite");
    assert!(matches!(
        err,
        ProtocolError::MeshInviteRequired | ProtocolError::Other(_)
    ));
}

#[tokio::test]
async fn reject_bad_invite_token() {
    let host_a = Arc::new(TestHost::new("a-bad"));
    let host_b = Arc::new(TestHost::new("b-bad"));
    let mesh_a = bind_mesh(Arc::clone(&host_a)).await;
    let mesh_b = bind_mesh(Arc::clone(&host_b)).await;

    let err = mesh_b
        .dial_with_invite(&mesh_a.bind_addr().to_string(), Some("horde_inv_bogus"))
        .await
        .expect_err("bad invite");
    assert!(
        matches!(err, ProtocolError::Other(_))
            || matches!(err, ProtocolError::InviteInvalid)
            || err.to_string().contains("invite")
    );
}

#[tokio::test]
async fn invite_join_and_cross_node_job() {
    let host_a = Arc::new(TestHost::new("a-job"));
    let host_b = Arc::new(TestHost::new("b-job"));
    let mesh_a = bind_mesh(Arc::clone(&host_a)).await;
    let mesh_b = bind_mesh(Arc::clone(&host_b)).await;

    let token = host_a.mint_mesh_invite().await;
    let peer = mesh_b
        .dial_with_invite(&mesh_a.bind_addr().to_string(), Some(&token))
        .await
        .expect("invite join");
    assert_eq!(peer.node_id, mesh_a.node_fingerprint());

    // Allow peer sessions to settle on both sides
    for _ in 0..50 {
        if !mesh_a.list_peers().await.is_empty() && !mesh_b.list_peers().await.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        !mesh_b.list_peers().await.is_empty(),
        "B should see A as peer"
    );

    let req = JobCreateRequest {
        fingerprint: ModelFingerprint {
            model_id: "stub/echo".into(),
            revision_hash: "deadbeefcafebabe".into(),
            tokenizer_revision: None,
            precision: "fp16".into(),
        },
        kind: WorkKind::StubEcho,
        payload: "cross-node-ping".into(),
        requested_vram_mb: None,
        routing: JobRouting::Mesh,
        priority_hint: Some(60),
    };
    let peer_id = mesh_b.find_capable_peer(&req).await.unwrap();
    let job_id = horde_protocol::JobId::new();
    let result = mesh_b
        .offer_job(&peer_id, job_id, req)
        .await
        .expect("cross-node job");
    assert!(result.output.contains("cross-node-ping"));
    result
        .attestation
        .verify(&result.output)
        .expect("attestation verifies");
    assert_eq!(
        result.attestation.model_fingerprint.model_id,
        "stub/echo"
    );
}

#[tokio::test]
async fn keystore_smoke_unused_import_guard() {
    // Keep EncryptedKeystore linked for CI feature parity with node crate.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("horde-mesh-ks-{nanos}.json"));
    let _ = EncryptedKeystore::create(&path, "x");
    let _ = std::fs::remove_file(&path);
}
