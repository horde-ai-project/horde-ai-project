//! Runtime mesh bind enable + rebind via admin API (no process restart).

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use horde_api::{build_app, resolve_ui_dir, ApiConfig};
use tower::ServiceExt;

fn test_config(data_dir: std::path::PathBuf, mesh_bind: Option<&str>) -> ApiConfig {
    ApiConfig {
        data_dir,
        passphrase: "test-passphrase-not-a-secret".into(),
        hostname: "test-host".into(),
        bind: "127.0.0.1:0".parse().unwrap(),
        mesh_bind: mesh_bind.map(|s| s.parse().unwrap()),
        bootstrap_peers: vec![],
        ui_dir: resolve_ui_dir(),
    }
}

async fn json_body(res: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&to_bytes(res.into_body(), 1024 * 1024).await.unwrap()).unwrap()
}

#[tokio::test]
async fn enable_and_rebind_mesh_via_admin_api() {
    let data_dir = std::env::temp_dir().join(format!("horde-mesh-bind-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&data_dir).expect("mkdir");
    let (app, _state, api_key) = build_app(test_config(data_dir.clone(), None))
        .await
        .expect("build_app");

    let before = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/admin/config")
                .header("Authorization", format!("Bearer {api_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(before.status(), StatusCode::OK);
    let before_cfg = json_body(before).await;
    assert_eq!(before_cfg["mesh_enabled"], false);
    assert!(before_cfg.get("passphrase").is_none());

    let enable = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/admin/mesh/bind")
                .header("Authorization", format!("Bearer {api_key}"))
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"bind":"127.0.0.1:0"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(enable.status(), StatusCode::OK);
    let enabled = json_body(enable).await;
    assert_eq!(enabled["ok"], true);
    assert_eq!(enabled["mesh_enabled"], true);
    assert_eq!(enabled["rebind"], "runtime");
    let first_bind = enabled["mesh_bind"].as_str().expect("mesh_bind").to_string();
    assert!(first_bind.starts_with("127.0.0.1:"));
    assert_ne!(first_bind, "127.0.0.1:0");

    let status = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/admin/mesh/status")
                .header("Authorization", format!("Bearer {api_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(status.status(), StatusCode::OK);
    let st = json_body(status).await;
    assert_eq!(st["bind"], first_bind);

    let config = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/admin/config")
                .header("Authorization", format!("Bearer {api_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let cfg = json_body(config).await;
    assert_eq!(cfg["mesh_enabled"], true);
    assert_eq!(cfg["mesh_bind"], first_bind);
    assert!(cfg.get("passphrase").is_none());

    let rebind = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/admin/mesh/bind")
                .header("Authorization", format!("Bearer {api_key}"))
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"bind":"127.0.0.1:0"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rebind.status(), StatusCode::OK);
    let rebound = json_body(rebind).await;
    let second_bind = rebound["mesh_bind"].as_str().unwrap().to_string();
    assert!(second_bind.starts_with("127.0.0.1:"));
    // Ephemeral port should change across rebinds in normal cases; if equal, still OK.
    let _ = first_bind;

    let bad = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/admin/mesh/bind")
                .header("Authorization", format!("Bearer {api_key}"))
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"bind":"not-an-addr"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    let err = json_body(bad).await;
    assert!(err["error"].as_str().unwrap().contains("invalid mesh bind"));

    let _ = std::fs::remove_dir_all(&data_dir);
    assert!(!second_bind.is_empty());
}

#[tokio::test]
async fn mesh_bind_requires_auth() {
    let data_dir = std::env::temp_dir().join(format!("horde-mesh-auth-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&data_dir).expect("mkdir");
    let (app, _state, _key) = build_app(test_config(data_dir.clone(), None))
        .await
        .expect("build_app");
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/admin/mesh/bind")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"bind":"127.0.0.1:0"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let _ = std::fs::remove_dir_all(&data_dir);
}
