use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use horde_api::{build_app, resolve_ui_dir, ApiConfig};
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
async fn mute_list_invite_ttl_health_and_ui_assets() {
    let data_dir =
        std::env::temp_dir().join(format!("horde-enh2-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&data_dir).unwrap();
    let (app, _state, api_key) = build_app(test_config(data_dir.clone()))
        .await
        .expect("build_app");

    // Dark themes + shortcuts/checklist markers in operator UI.
    let ui = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/ui/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ui.status(), StatusCode::OK);
    let html = String::from_utf8(
        to_bytes(ui.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains("checklist-banner"));
    assert!(html.contains("offline-banner"));
    assert!(html.contains("throttle-banner"));
    assert!(html.contains("theme-os-sync"));
    assert!(html.contains("invite-ttl"));
    assert!(html.contains("mute-form"));
    assert!(html.contains("result-preview"));
    assert!(html.contains("shortcuts-list"));

    let night = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/ui/css/themes/night-slate.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(night.status(), StatusCode::OK);

    let charcoal = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/ui/css/themes/charcoal.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(charcoal.status(), StatusCode::OK);

    let help = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/ui/help/OPERATIONS.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(help.status(), StatusCode::OK);
    let ops = String::from_utf8(
        to_bytes(help.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(ops.contains("Keyboard shortcuts"));
    assert!(ops.contains("Per-peer mute"));
    assert!(ops.to_lowercase().contains("throttle") || ops.contains("Throttling"));

    // Mint invite exposes expires_at for UI countdown / renew.
    let mint = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/admin/invites")
                .header("Authorization", format!("Bearer {api_key}"))
                .header("Content-Type", "application/json")
                .body(Body::from(
                    br#"{"purpose":"mesh_join","ttl_secs":120}"#.as_slice(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(mint.status(), StatusCode::OK);
    let invite = json_body(mint).await;
    assert!(invite.get("expires_at").is_some());
    assert!(invite.get("token").is_some());

    // Soft mute / unmute (pause accept — not blocklist).
    let peer = "peer-fingerprint-placeholder";
    let mute = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/admin/mesh/mute")
                .header("Authorization", format!("Bearer {api_key}"))
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({ "node_id": peer })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(mute.status(), StatusCode::OK);
    let muted = json_body(mute).await;
    assert_eq!(muted["muted"], peer);

    let list = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/admin/mesh/mute")
                .header("Authorization", format!("Bearer {api_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let listed = json_body(list).await;
    let ids = listed["muted_node_ids"].as_array().unwrap();
    assert!(ids.iter().any(|v| v.as_str() == Some(peer)));

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
    let h = json_body(health).await;
    assert!(h["muted"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v.as_str() == Some(peer)));

    let unmute = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/v1/admin/mesh/mute/{peer}"))
                .header("Authorization", format!("Bearer {api_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unmute.status(), StatusCode::OK);
    let um = json_body(unmute).await;
    assert_eq!(um["unmuted"], true);

    // Blocklist still separate from mute.
    let bl = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/admin/mesh/blocklist")
                .header("Authorization", format!("Bearer {api_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bl.status(), StatusCode::OK);
    let blocked = json_body(bl).await;
    assert!(blocked["blocked_node_ids"]
        .as_array()
        .unwrap()
        .iter()
        .all(|v| v.as_str() != Some(peer)));

    let _ = std::fs::remove_dir_all(&data_dir);
}
