use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use horde_api::{build_app, resolve_ui_dir, ApiConfig};
use horde_protocol::{JobCreateRequest, JobRouting, ModelFingerprint, WorkKind};
use tower::ServiceExt;

fn test_config(data_dir: std::path::PathBuf) -> ApiConfig {
    let ui_dir = resolve_ui_dir();
    ApiConfig {
        data_dir,
        passphrase: "test-passphrase-not-a-secret".into(),
        hostname: "test-host".into(),
        bind: "127.0.0.1:0".parse().unwrap(),
        mesh_bind: Some("127.0.0.1:0".parse().unwrap()),
        bootstrap_peers: vec![],
        ui_dir,
    }
}

async fn json_body(res: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&to_bytes(res.into_body(), 1024 * 1024).await.unwrap()).unwrap()
}

#[tokio::test]
async fn timeline_drain_diagnostics_export_and_help() {
    let data_dir =
        std::env::temp_dir().join(format!("horde-enh-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&data_dir).unwrap();
    let (app, state, api_key) = build_app(test_config(data_dir.clone()))
        .await
        .expect("build_app");

    let help = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/ui/help/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(help.status(), StatusCode::OK);
    let help_html = String::from_utf8(
        to_bytes(help.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(help_html.contains("Operations") || help_html.contains("OPERATIONS"));

    let ops = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/ui/help/OPERATIONS.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ops.status(), StatusCode::OK);

    let create = JobCreateRequest {
        fingerprint: ModelFingerprint {
            model_id: "stub/echo".into(),
            revision_hash: "deadbeefcafebabe".into(),
            tokenizer_revision: None,
            precision: "fp16".into(),
        },
        kind: WorkKind::StubEcho,
        payload: "timeline".into(),
        routing: JobRouting::Local,
        requested_vram_mb: None,
        priority_hint: None,
    };
    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/jobs")
                .header("Authorization", format!("Bearer {api_key}"))
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&create).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::ACCEPTED);
    let job = json_body(created).await;
    let job_id = job["id"].as_str().unwrap().to_string();
    assert!(!job["timeline"].as_array().unwrap().is_empty());

    // Wait for completion so timeline includes running/succeeded.
    for _ in 0..50 {
        let st = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/jobs/{job_id}"))
                    .header("Authorization", format!("Bearer {api_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = json_body(st).await;
        if body["status"] == "succeeded" || body["status"] == "failed" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let tl = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/jobs/{job_id}/timeline"))
                .header("Authorization", format!("Bearer {api_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(tl.status(), StatusCode::OK);
    let timeline = json_body(tl).await;
    assert!(timeline["timeline"].as_array().unwrap().len() >= 2);

    let mint = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/admin/invites")
                .header("Authorization", format!("Bearer {api_key}"))
                .header("Content-Type", "application/json")
                .body(Body::from(
                    br#"{"purpose":"mesh_join","ttl_secs":600}"#.as_slice(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(mint.status(), StatusCode::OK);
    let invite = json_body(mint).await;
    assert!(invite.get("token").is_some());
    assert!(invite.get("join_uri").is_some());
    assert!(invite["join_uri"]
        .as_str()
        .unwrap_or("")
        .starts_with("horde://join?"));

    let diag = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/admin/diagnostics/join")
                .header("Authorization", format!("Bearer {api_key}"))
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "peer_addr": "127.0.0.1:9797",
                        "token": invite["token"],
                        "client_time": chrono::Utc::now().to_rfc3339(),
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(diag.status(), StatusCode::OK);
    let checks = json_body(diag).await;
    assert!(checks["checks"].as_array().unwrap().iter().any(|c| c["id"] == "api_key"));

    let health = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/admin/mesh/health")
                .header("Authorization", format!("Bearer {api_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);

    let export = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/admin/export")
                .header("Authorization", format!("Bearer {api_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(export.status(), StatusCode::OK);
    let snap = json_body(export).await;
    assert!(snap.get("jobs").is_some());
    assert!(snap.get("peer_stats").is_some());

    let drain = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/admin/drain")
                .header("Authorization", format!("Bearer {api_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(drain.status(), StatusCode::OK);
    let d = json_body(drain).await;
    assert_eq!(d["draining"], true);

    // New jobs rejected while draining.
    let rejected = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/jobs")
                .header("Authorization", format!("Bearer {api_key}"))
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&create).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(rejected.status().is_client_error());

    // Drain should become ready (no in-flight) and notify shutdown.
    for _ in 0..50 {
        if state.head.drain_status().await.ready_to_exit {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(state.head.drain_status().await.ready_to_exit);

    let _ = std::fs::remove_dir_all(&data_dir);
}
