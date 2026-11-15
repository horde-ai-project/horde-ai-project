use std::sync::Arc;

use horde_node::PhysicalServer;
use horde_protocol::{JobRecord, ProtocolError, ProtocolResult};

/// Pick the first member that can accept the job under caps + fingerprint.
pub async fn pick_server(
    members: &[Arc<PhysicalServer>],
    job: &JobRecord,
) -> ProtocolResult<Arc<PhysicalServer>> {
    let mut last_err = ProtocolError::Other("no members in collective".into());
    for server in members {
        match server.can_accept(job).await {
            Ok(()) => return Ok(Arc::clone(server)),
            Err(err) => last_err = err,
        }
    }
    Err(last_err)
}
