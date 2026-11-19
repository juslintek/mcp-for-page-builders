use std::time::Duration;

use mcp_for_page_builders::laya::{LayaClient, LayaRequest, LayaStatus};
use serde_json::json;

fn request(id: &str, state: impl Into<serde_json::Value>, question: &str) -> LayaRequest {
    LayaRequest {
        id: id.to_owned(),
        state: state.into(),
        question: question.to_owned(),
        options: vec![
            "native Elementor style".to_owned(),
            "custom CSS".to_owned(),
            "insufficient evidence".to_owned(),
        ],
    }
}

#[tokio::test]
async fn offline_worker_predicts_truncates_times_out_and_restarts() {
    let mut client = LayaClient::new()
        .expect("install the pinned local Laya runtime and checkpoint")
        .with_timeouts(Duration::from_secs(180), Duration::from_nanos(1));
    let error = client
        .predict(&request(
            "forced-timeout",
            json!({"css":"padding: 16px"}),
            "Map this CSS to the best Elementor destination.",
        ))
        .await
        .expect_err("the deliberately short inference deadline should stop the worker");
    assert!(error.to_string().contains("exceeded"));

    client.set_inference_timeout(Duration::from_secs(180));
    let english = client
        .predict(&request(
            "english",
            json!({"css":"padding: 16px", "target":"Elementor section"}),
            "Which destination best matches this CSS?",
        ))
        .await
        .expect("English inference should succeed offline");
    assert_eq!(english.status, LayaStatus::Prediction);
    let probabilities = english.probabilities.expect("prediction probabilities");
    assert_eq!(probabilities.len(), 3);
    assert!((probabilities.values().sum::<f64>() - 1.0).abs() <= 0.002);

    let lithuanian = client
        .predict(&request(
            "lithuanian",
            json!({"css":"padding: 16px", "target":"Elementor sekcija"}),
            "Kur geriausia perkelti šią CSS taisyklę?",
        ))
        .await
        .expect("Lithuanian inference should succeed offline");
    assert_eq!(lithuanian.status, LayaStatus::Prediction);
    assert_eq!(
        lithuanian
            .probabilities
            .expect("prediction probabilities")
            .len(),
        3
    );

    let truncated = client
        .predict(&request(
            "truncated",
            json!("padding: 16px; ".repeat(1_500)),
            "Map this CSS to the best Elementor destination.",
        ))
        .await
        .expect("truncated inference should return an explicit abstention");
    assert_eq!(truncated.status, LayaStatus::Abstain);
    assert!(truncated.truncated);
    assert!(truncated.dropped_tokens > 0);
    assert!(
        truncated
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("truncated"))
    );
    client
        .shutdown()
        .await
        .expect("worker must stop and reap cleanly");
}
