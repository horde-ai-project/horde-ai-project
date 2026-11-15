use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use horde_protocol::ProtocolError;
use serde_json::json;

pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl From<ProtocolError> for ApiError {
    fn from(err: ProtocolError) -> Self {
        let status = match &err {
            ProtocolError::AuthFailed => StatusCode::UNAUTHORIZED,
            ProtocolError::JobNotFound
            | ProtocolError::ServerNotFound
            | ProtocolError::PeerNotFound => StatusCode::NOT_FOUND,
            ProtocolError::InviteInvalid
            | ProtocolError::MeshInviteRequired
            | ProtocolError::PeerBlocked
            | ProtocolError::NoMeshPeer
            | ProtocolError::AttestationInvalid(_) => StatusCode::BAD_REQUEST,
            ProtocolError::JobNotCancellable(_)
            | ProtocolError::InvalidFingerprint(_)
            | ProtocolError::InvalidTokenize(_)
            | ProtocolError::LimitRejected(_)
            | ProtocolError::CapabilityMismatch(_)
            | ProtocolError::ResourceGate(_)
            | ProtocolError::DhtDisabled
            | ProtocolError::ResourceProofInvalid(_) => StatusCode::BAD_REQUEST,
            ProtocolError::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            ProtocolError::Keystore(_) | ProtocolError::Other(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::new(status, err.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({ "error": self.message })),
        )
            .into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
