use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use horde_api::{build_app, resolve_ui_dir, ApiConfig};
use horde_protocol::{JobCreateRequest, JobRouting, ModelFingerprint, WorkKind};
use tower::ServiceExt;

fn test_config(data_dir: std::path::PathBuf) -> ApiConfig {
    let ui_dir = resolve_ui_dir();
    assert!(
        ui_dir.join("index.html").is_file(),
        "expected ui/index.html at {}",
        ui_dir.display()
    );
    ApiConfig {
        data_dir,
        passphrase: "test-passphrase-not-a-secret".into(),
        hostname: "test-host".into(),
        bind: "127.0.0.1:0".parse().unwrap(),
        mesh_bind: None,
        bootstrap_peers: vec![],
        ui_dir,
    }
}

#[tokio::test]
async fn serves_operator_ui_and_lists_jobs() {
    let data_dir = std::env::temp_dir().join(format!("horde-ui-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&data_dir).expect("mkdir");
    let (app, _state, api_key) = build_app(test_config(data_dir.clone()))
        .await
        .expect("build_app");

    let ui = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/ui/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("ui oneshot");
    assert_eq!(ui.status(), StatusCode::OK);
    let html = String::from_utf8(
        to_bytes(ui.into_body(), 1024 * 1024)
            .await
            .expect("ui body")
            .to_vec(),
    )
    .expect("utf8");
    assert!(html.contains("Horde"), "brand in UI");
    assert!(html.contains("Host") && html.contains("Nodes"));
    assert!(html.contains("Jobs") && html.contains("Monitor"));

    let root = app
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .expect("root");
    assert!(
        root.status().is_redirection(),
        "root should redirect to /ui/"
    );

    let config_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/admin/config")
                .header("Authorization", format!("Bearer {api_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("config");
    assert_eq!(config_res.status(), StatusCode::OK);
    let config: serde_json::Value = serde_json::from_slice(
        &to_bytes(config_res.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(config.get("bind").is_some());
    assert!(config.get("passphrase").is_none());
    assert_eq!(config["mesh_enabled"], false);

    let list_empty = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/jobs")
                .header("Authorization", format!("Bearer {api_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("list empty");
    assert_eq!(list_empty.status(), StatusCode::OK);
    let empty: Vec<serde_json::Value> = serde_json::from_slice(
        &to_bytes(list_empty.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(empty.is_empty());

    let create = JobCreateRequest {
        fingerprint: ModelFingerprint {
            model_id: "stub/echo".into(),
            revision_hash: "deadbeefcafebabe".into(),
            tokenizer_revision: None,
            precision: "fp16".into(),
        },
        kind: WorkKind::StubEcho,
        payload: "hello".into(),
        routing: JobRouting::Local,
        requested_vram_mb: None,
        priority_hint: None,
    };
    let body = serde_json::to_vec(&create).unwrap();
    let created_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/jobs")
                .header("Authorization", format!("Bearer {api_key}"))
                .header("Content-Type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .expect("create");
    assert_eq!(created_res.status(), StatusCode::ACCEPTED);

    let mut found = false;
    for _ in 0..50 {
        let list_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/jobs?limit=10")
                    .header("Authorization", format!("Bearer {api_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("list");
        let list: Vec<serde_json::Value> = serde_json::from_slice(
            &to_bytes(list_res.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        if !list.is_empty() {
            assert_eq!(list.len(), 1);
            found = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let _ = std::fs::remove_dir_all(&data_dir);
    assert!(found, "job never appeared in GET /v1/jobs");
}
