use async_trait::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;

use crate::error::ApiError;
use crate::AppState;

/// Bearer API key extracted from `Authorization: Bearer <key>`.
#[allow(dead_code)]
pub struct StudioAuth {
    pub api_key: String,
}

#[async_trait]
impl FromRequestParts<AppState> for StudioAuth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "missing Authorization"))?;

        let key = header
            .strip_prefix("Bearer ")
            .or_else(|| header.strip_prefix("bearer "))
            .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "expected Bearer token"))?
            .trim();

        state.head.auth_api_key(key).await.map_err(ApiError::from)?;
        Ok(StudioAuth {
            api_key: key.to_string(),
        })
    }
}

/// Admin calls are localhost-only in Phase A; still require the same API key
/// (or a future admin unlock). Keeps one auth story for studios and admin tools.
#[allow(dead_code)]
pub struct AdminAuth(pub StudioAuth);

#[async_trait]
impl FromRequestParts<AppState> for AdminAuth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(AdminAuth(
            StudioAuth::from_request_parts(parts, state).await?,
        ))
    }
}
