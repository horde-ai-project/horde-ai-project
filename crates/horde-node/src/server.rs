use std::sync::Arc;

use chrono::{Local, Timelike};
use horde_protocol::{
    CapabilityAdvert, JobRecord, JobResult, JobStatus, ProtocolError, ProtocolResult, ServerLimits,
};
use serde::Serialize;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::executor::{GpuExecutor, StubExecutor};
use crate::keystore::IdentityKeys;

/// Snapshot returned to the collective admin API.
#[derive(Clone, Debug, Serialize)]
pub struct ServerSnapshot {
    pub server_id: String,
    pub hostname: String,
    pub limits: ServerLimits,
    pub capability: CapabilityAdvert,
    pub node_id: String,
}

/// One physical OS host under a collective.
pub struct PhysicalServer {
    pub server_id: String,
    pub hostname: String,
    identity: IdentityKeys,
    limits: RwLock<ServerLimits>,
    capability: RwLock<CapabilityAdvert>,
    executor: StubExecutor,
}

impl PhysicalServer {
    pub fn new(server_id: impl Into<String>, hostname: impl Into<String>, identity: IdentityKeys) -> Self {
        let server_id = server_id.into();
        let hostname = hostname.into();
        let capability = CapabilityAdvert::stub_local(&server_id, &hostname);
        Self {
            server_id,
            hostname,
            identity,
            limits: RwLock::new(ServerLimits::default()),
            capability: RwLock::new(capability),
            executor: StubExecutor,
        }
    }

    pub fn node_fingerprint(&self) -> String {
        self.identity.node_id.fingerprint()
    }

    pub async fn snapshot(&self) -> ServerSnapshot {
        ServerSnapshot {
            server_id: self.server_id.clone(),
            hostname: self.hostname.clone(),
            limits: self.limits.read().await.clone(),
            capability: self.capability.read().await.clone(),
            node_id: self.identity.node_id.fingerprint(),
        }
    }

    pub async fn set_limits(&self, limits: ServerLimits) -> ProtocolResult<()> {
        limits.validate()?;
        *self.limits.write().await = limits;
        info!(server_id = %self.server_id, "updated server limits");
        Ok(())
    }

    pub async fn limits(&self) -> ServerLimits {
        self.limits.read().await.clone()
    }

    /// Whether this server may accept a job right now under hard caps + Phase C gates.
    pub async fn can_accept(&self, job: &JobRecord) -> ProtocolResult<()> {
        let limits = self.limits.read().await;
        if !limits.enabled {
            return Err(ProtocolError::LimitRejected("server disabled".into()));
        }
        let hour = Local::now().hour() as u8;
        if !limits.time_window.contains_hour(hour) {
            return Err(ProtocolError::LimitRejected(
                "outside configured time window".into(),
            ));
        }
        let cap = self.capability.read().await;
        if !cap.supports_fingerprint(&job.fingerprint) {
            return Err(ProtocolError::CapabilityMismatch(format!(
                "server lacks model {}",
                job.fingerprint.compact()
            )));
        }
        horde_protocol::check_resource_gate(
            &job.kind,
            &job.fingerprint,
            &cap,
            job.requested_vram_mb,
        )?;
        if let Some(need) = job.requested_vram_mb {
            if need > cap.vram_free_mb && cap.backend != horde_protocol::BackendKind::Stub {
                return Err(ProtocolError::CapabilityMismatch(format!(
                    "need {need} MiB VRAM, free {}",
                    cap.vram_free_mb
                )));
            }
        }
        Ok(())
    }

    pub async fn run_job(self: &Arc<Self>, mut job: JobRecord) -> JobRecord {
        job.status = JobStatus::Running;
        job.assigned_server_id = Some(self.server_id.clone());
        job.updated_at = chrono::Utc::now();

        match self.executor.execute(&job, &self.identity).await {
            Ok(result) => {
                job.status = JobStatus::Succeeded;
                job.result = Some(result);
                job.error = None;
            }
            Err(err) => {
                warn!(job_id = %job.id, error = %err, "job failed");
                job.status = JobStatus::Failed;
                job.error = Some(err.to_string());
                job.result = None::<JobResult>;
            }
        }
        job.updated_at = chrono::Utc::now();
        job
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horde_protocol::{JobCreateRequest, ModelFingerprint, WorkKind};

    #[tokio::test]
    async fn stub_runs_echo_job() {
        let keys = IdentityKeys::generate();
        let server = Arc::new(PhysicalServer::new("srv-1", "localhost", keys));
        let job = JobRecord::new(JobCreateRequest {
            fingerprint: ModelFingerprint {
                model_id: "stub/echo".into(),
                revision_hash: "deadbeefcafebabe".into(),
                tokenizer_revision: None,
                precision: "fp16".into(),
            },
            kind: WorkKind::StubEcho,
            payload: "hi".into(),
            requested_vram_mb: None,
            routing: Default::default(),
        })
        .unwrap();
        assert!(server.can_accept(&job).await.is_ok());
        let done = server.run_job(job).await;
        assert_eq!(done.status, JobStatus::Succeeded);
        assert!(done.result.unwrap().output.contains("echo=hi"));
    }
}
