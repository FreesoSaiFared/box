//! End-to-end route tests for one-request mixed guest recipes.
use crate::{app, AppState};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

async fn send(dir: &TempDir, body: Value, auth: bool) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .uri("/v1/recipe")
        .method("POST")
        .header("content-type", "application/json");
    if auth {
        request = request.header("authorization", "Bearer secret-token");
    }
    let response = app(AppState::for_test(
        dir.path().canonicalize().unwrap(),
        "secret-token",
    ))
    .oneshot(request.body(Body::from(body.to_string())).unwrap())
    .await
    .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let data = serde_json::from_slice(&bytes).unwrap();
    (status, data)
}

fn program() -> Value {
    json!({
        "name": "mixed-guest-example",
        "journal": "receipts/mixed.json",
        "steps": [
            {"op":"exec", "id":"generate", "command":["/bin/sh","-c","printf honey"]},
            {"op":"write_file", "id":"save", "path":"out/created.txt",
             "content_from":{"from":"generate","pointer":"/stdout"}},
            {"op":"read_file","id":"inspect","path":"out/created.txt"},
            {"op":"assert","id":"verify","from":"inspect","pointer":"/content",
             "equals":"honey"}
        ]
    })
}

#[tokio::test]
async fn mixed_exec_file_read_assert_journal_and_resume() {
    let dir = TempDir::new().unwrap();
    let (status, output) = send(&dir, program(), true).await;
    assert_eq!(status, StatusCode::OK, "{output}");
    assert_eq!(output["ok"], true, "{output}");
    assert_eq!(output["ran"], 4);
    assert_eq!(output["assertions_passed"], 1);
    assert_eq!(output["assertions_total"], 1);
    assert_eq!(output["steps"][0]["stage"], "delivered");
    assert_eq!(output["steps"][2]["stage"], "observed");
    assert_eq!(output["steps"][3]["stage"], "verified");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("out/created.txt")).unwrap(),
        "honey"
    );
    let receipt_path = dir.path().join("receipts/mixed.json");
    assert!(receipt_path.exists());
    let mut resume = program();
    resume["resume"] = json!(true);
    let (status2, output2) = send(&dir, resume, true).await;
    assert_eq!(status2, StatusCode::OK, "{output2}");
    assert_eq!(output2["ok"], true);
    assert_eq!(output2["reused"], 4);
    assert_eq!(output2["ran"], 4);
    assert_eq!(output2["assertions_passed"], 1);
}

#[tokio::test]
async fn late_lint_error_prevents_earlier_effects() {
    let dir = TempDir::new().unwrap();
    let (status, output) = send(
        &dir,
        json!({
            "steps": [
                {"op":"write_file", "id":"never", "path":"should-not-exist.txt", "content":"bad"},
                {"op":"cua", "id":"invalid", "recipe":{
                    "steps":[{"op":"click","x":5000,"y":20}]
                }}
            ]
        }),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{output}");
    assert!(!dir.path().join("should-not-exist.txt").exists());
}

#[tokio::test]
async fn failed_assertion_stops_before_next_effect() {
    let dir = TempDir::new().unwrap();
    let (status, output) = send(
        &dir,
        json!({
            "steps": [
                {"op":"exec","id":"run","command":["/bin/sh","-c","printf actual"]},
                {"op":"assert","id":"check","from":"run","pointer":"/stdout","equals":"expected"},
                {"op":"write_file","id":"not-allowed","path":"not-written.txt","content":"no"}
            ]
        }),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{output}");
    assert_eq!(output["ok"], false);
    assert_eq!(output["stopped_at"], "check");
    assert_eq!(output["steps"][1]["stage"], "failed");
    assert_eq!(output["ran"], 2);
    assert!(!dir.path().join("not-written.txt").exists());
}

#[tokio::test]
async fn unresolved_effect_must_not_be_replayed_blindly() {
    let dir = TempDir::new().unwrap();
    let (_status, first) = send(&dir, program(), true).await;
    assert_eq!(first["ok"], true, "{first}");
    let path = dir.path().join("receipts/mixed.json");
    let mut snapshot: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    snapshot["pending"] = json!("generate");
    std::fs::write(&path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
    let mut resume = program();
    resume["resume"] = json!(true);
    let (status, second) = send(&dir, resume, true).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["ok"], false);
    assert_eq!(second["needs_reconciliation"], "generate");
}

#[tokio::test]
async fn resume_refuses_changed_plan() {
    let dir = TempDir::new().unwrap();
    let (_, first) = send(&dir, program(), true).await;
    assert_eq!(first["ok"], true, "{first}");
    let mut changed = program();
    changed["resume"] = json!(true);
    changed["steps"][0]["command"] = json!(["/bin/sh", "-c", "printf altered"]);
    let (status, _) = send(&dir, changed, true).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn rejects_unavailable_result_and_missing_auth() {
    let dir = TempDir::new().unwrap();
    let (status, _) = send(&dir, program(), false).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(
        &dir,
        json!({
            "steps": [{"op":"assert","id":"before","from":"later","pointer":"/ok","equals":true},
                      {"op":"wait","id":"later","ms":1}]
        }),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn delivery_is_not_verification() {
    let dir = TempDir::new().unwrap();
    let (status, output) = send(
        &dir,
        json!({
            "steps": [{"op":"exec","id":"run","command":["/bin/sh","-c","printf done"]}]
        }),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{output}");
    assert_eq!(output["ok"], true);
    assert_eq!(output["assertions_total"], 0);
    assert_eq!(output["assertions_passed"], 0);
    assert_eq!(output["steps"][0]["stage"], "delivered");
}

#[tokio::test]
async fn resume_does_not_replay_later_effects_after_non_stopping_failure() {
    let dir = TempDir::new().unwrap();
    let mut req = json!({
        "name":"mixed-failure",
        "journal":"receipts/mixed-failure.json",
        "stop_on_error":false,
        "steps":[
            {"op":"exec","id":"run","command":["/bin/sh","-c","printf observed"]},
            {"op":"assert","id":"fails","from":"run","pointer":"/stdout","equals":"different"},
            {"op":"write_file","id":"late-effect","path":"output.txt","content":"once"}
        ]
    });
    let (status, result) = send(&dir, req.clone(), true).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["ok"], false);
    assert_eq!(result["ran"], 3);
    assert!(dir.path().join("output.txt").exists());
    req["resume"] = json!(true);
    let (status, _) = send(&dir, req, true).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("output.txt")).unwrap(),
        "once"
    );
}
