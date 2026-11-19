use std::{collections::BTreeMap, path::PathBuf, process::Stdio, time::Duration};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
    time::timeout,
};

const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_INFERENCE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize)]
pub struct LayaRequest {
    pub id: String,
    pub state: Value,
    pub question: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayaStatus {
    Prediction,
    Abstain,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LayaDecision {
    pub id: String,
    pub status: LayaStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probabilities: Option<BTreeMap<String, f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer_confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin: Option<f64>,
    pub truncated: bool,
    pub diagnostics: Vec<String>,
    pub model_repository: String,
    pub model_revision: String,
    pub runtime_version: String,
    pub mlx_version: String,
    pub model_load_ms: f64,
    pub inference_ms: f64,
    pub input_tokens: u64,
    pub dropped_tokens: u64,
}

struct Worker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

pub struct LayaClient {
    python: PathBuf,
    model_path: PathBuf,
    startup_timeout: Duration,
    inference_timeout: Duration,
    worker: Mutex<Option<Worker>>,
}

impl LayaClient {
    pub fn new() -> Result<Self> {
        let python = std::env::var_os("LAYA_PYTHON")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| {
                    PathBuf::from(home)
                        .join(".local/share/mcp-for-page-builders/laya-mlx/bin/python")
                })
            })
            .context("set LAYA_PYTHON to the isolated Laya Python executable")?;
        let model_path = std::env::var_os("LAYA_MODEL_PATH")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| {
                PathBuf::from(home)
                    .join(".cache/mcp-for-page-builders/laya-hf/hub/models--aac6fef--laya-multilingual-mlx/snapshots")
                    .join("ba40c87fcb357f1643d04d71323af9cdc3b9e591")
            }))
            .context("set LAYA_MODEL_PATH to the pinned local model snapshot")?;
        if !python.is_file() {
            bail!(
                "Laya Python not found at {}; set LAYA_PYTHON to the isolated environment executable",
                python.display()
            );
        }
        if !model_path.is_dir() {
            bail!(
                "pinned Laya model not found at {}; set LAYA_MODEL_PATH to its local snapshot directory",
                model_path.display()
            );
        }
        Ok(Self {
            python,
            model_path,
            startup_timeout: env_timeout("LAYA_STARTUP_TIMEOUT_MS", DEFAULT_STARTUP_TIMEOUT)?,
            inference_timeout: env_timeout("LAYA_INFERENCE_TIMEOUT_MS", DEFAULT_INFERENCE_TIMEOUT)?,
            worker: Mutex::new(None),
        })
    }

    pub fn with_timeouts(mut self, startup: Duration, inference: Duration) -> Self {
        self.startup_timeout = startup;
        self.inference_timeout = inference;
        self
    }

    pub fn set_inference_timeout(&mut self, inference: Duration) {
        self.inference_timeout = inference;
    }

    pub async fn predict(&self, request: &LayaRequest) -> Result<LayaDecision> {
        let mut retained = self.worker.lock().await;
        if let Some(worker) = retained.as_mut() {
            if worker.child.try_wait()?.is_some() {
                let dead = retained.take().expect("worker was present");
                stop(dead).await?;
            }
        }
        if retained.is_none() {
            *retained = Some(self.start_worker().await?);
        }

        let worker = retained.as_mut().expect("worker started above");
        let line = serde_json::to_vec(request)?;
        let exchange = async {
            worker.stdin.write_all(&line).await?;
            worker.stdin.write_all(b"\n").await?;
            worker.stdin.flush().await?;
            let mut response = String::new();
            let bytes = worker.stdout.read_line(&mut response).await?;
            if bytes == 0 {
                bail!("Laya worker exited before returning a prediction");
            }
            let value: Value =
                serde_json::from_str(&response).context("Laya worker returned invalid JSON")?;
            if value.get("id").and_then(Value::as_str) != Some(request.id.as_str()) {
                bail!("Laya worker response id did not match the request");
            }
            if value.get("ok").and_then(Value::as_bool) != Some(true) {
                let diagnostics = value
                    .get("diagnostics")
                    .cloned()
                    .unwrap_or_else(|| json!(["unspecified worker failure"]));
                bail!("Laya worker transport failed: {diagnostics}");
            }
            let decision: LayaDecision = serde_json::from_value(value)
                .context("Laya worker decision has an invalid schema")?;
            validate_decision(&decision, request)?;
            Ok(decision)
        };
        match timeout(self.inference_timeout, exchange).await {
            Ok(Ok(prediction)) => Ok(prediction),
            Ok(Err(error)) => {
                let failed = retained.take().expect("worker was present");
                stop(failed).await?;
                Err(error)
            }
            Err(_) => {
                let timed_out = retained.take().expect("worker was present");
                stop(timed_out).await?;
                bail!(
                    "Laya inference exceeded {:?}; worker was stopped and will restart on next call",
                    self.inference_timeout
                )
            }
        }
    }

    pub async fn shutdown(&self) -> Result<()> {
        if let Some(worker) = self.worker.lock().await.take() {
            stop(worker).await?;
        }
        Ok(())
    }

    async fn start_worker(&self) -> Result<Worker> {
        let mut child = Command::new(&self.python)
            .arg("-u")
            .arg("-c")
            .arg(include_str!("../scripts/laya_worker.py"))
            .env("LAYA_MODEL_PATH", &self.model_path)
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1")
            .env("HF_HUB_DISABLE_TELEMETRY", "1")
            .env_remove("HF_TOKEN")
            .env_remove("HUGGINGFACE_HUB_TOKEN")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .context("could not start the configured Laya Python worker")?;
        let stdin = child
            .stdin
            .take()
            .context("Laya worker stdin was unavailable")?;
        let stdout = child
            .stdout
            .take()
            .context("Laya worker stdout was unavailable")?;
        let mut worker = Worker {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        };
        let mut ready = String::new();
        let startup = timeout(self.startup_timeout, worker.stdout.read_line(&mut ready)).await;
        match startup {
            Ok(Ok(bytes)) if bytes > 0 => {
                let metadata: Value = match serde_json::from_str(&ready) {
                    Ok(metadata) => metadata,
                    Err(error) => {
                        stop(worker).await?;
                        return Err(error).context("Laya worker startup returned invalid JSON");
                    }
                };
                if metadata.get("type").and_then(Value::as_str) != Some("ready") {
                    stop(worker).await?;
                    bail!("Laya worker did not report ready: {metadata}");
                }
                if metadata.get("model_repository").and_then(Value::as_str)
                    != Some("aac6fef/laya-multilingual-mlx")
                    || metadata.get("model_revision").and_then(Value::as_str)
                        != Some("ba40c87fcb357f1643d04d71323af9cdc3b9e591")
                    || metadata.get("runtime_version").and_then(Value::as_str) != Some("0.3.0")
                    || metadata.get("mlx_version").and_then(Value::as_str) != Some("0.32.3")
                {
                    stop(worker).await?;
                    bail!("Laya worker reported an unexpected model or runtime version");
                }
                Ok(worker)
            }
            Ok(Ok(_)) => {
                stop(worker).await?;
                bail!("Laya worker exited before startup; check stderr for the local model error")
            }
            Ok(Err(error)) => {
                stop(worker).await?;
                Err(error).context("could not read Laya worker startup")
            }
            Err(_) => {
                stop(worker).await?;
                bail!(
                    "Laya worker startup exceeded {:?} and was stopped",
                    self.startup_timeout
                )
            }
        }
    }
}

fn validate_decision(decision: &LayaDecision, request: &LayaRequest) -> Result<()> {
    if decision.id != request.id {
        bail!("Laya worker response id did not match the request");
    }
    if decision.model_repository != "aac6fef/laya-multilingual-mlx"
        || decision.model_revision != "ba40c87fcb357f1643d04d71323af9cdc3b9e591"
        || decision.runtime_version != "0.3.0"
        || decision.mlx_version != "0.32.3"
    {
        bail!("Laya decision reported an unexpected model or runtime version");
    }
    for (field, value) in [
        ("model_load_ms", decision.model_load_ms),
        ("inference_ms", decision.inference_ms),
    ] {
        if !value.is_finite() || value < 0.0 {
            bail!("Laya worker returned invalid {field}");
        }
    }
    match decision.status {
        LayaStatus::Abstain => {
            if decision.probabilities.is_some()
                || decision.answer_confidence.is_some()
                || decision.margin.is_some()
            {
                bail!("abstaining Laya response must not include prediction values");
            }
        }
        LayaStatus::Prediction => {
            let probabilities = decision
                .probabilities
                .as_ref()
                .context("Laya prediction omitted probabilities")?;
            if probabilities.len() != request.options.len()
                || request
                    .options
                    .iter()
                    .any(|label| !probabilities.contains_key(label))
            {
                bail!("Laya prediction returned an incomplete probability set");
            }
            if probabilities
                .values()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
                || !close_to_one(probabilities.values().sum())
            {
                bail!("Laya prediction returned invalid probabilities");
            }
            for (field, value) in [
                ("answer_confidence", decision.answer_confidence),
                ("margin", decision.margin),
            ] {
                if !value.is_some_and(|value| value.is_finite() && (0.0..=1.0).contains(&value)) {
                    bail!("Laya prediction returned invalid {field}");
                }
            }
            let mut ordered = probabilities.values().copied().collect::<Vec<_>>();
            ordered.sort_by(f64::total_cmp);
            let expected_margin = ordered[ordered.len() - 1] - ordered[ordered.len() - 2];
            if !decision
                .margin
                .is_some_and(|margin| (margin - expected_margin).abs() <= 0.002)
            {
                bail!("Laya prediction margin did not match its probabilities");
            }
        }
    }
    Ok(())
}

fn close_to_one(value: f64) -> bool {
    (value - 1.0).abs() <= 0.002
}

async fn stop(mut worker: Worker) -> Result<()> {
    if worker.child.try_wait()?.is_none() {
        worker
            .child
            .kill()
            .await
            .context("could not stop Laya worker")?;
    }
    worker
        .child
        .wait()
        .await
        .context("could not reap Laya worker")?;
    Ok(())
}

fn env_timeout(name: &str, default: Duration) -> Result<Duration> {
    let Some(value) = std::env::var_os(name) else {
        return Ok(default);
    };
    let millis: u64 = value
        .to_string_lossy()
        .parse()
        .with_context(|| format!("{name} must be a positive integer in milliseconds"))?;
    if millis == 0 {
        bail!("{name} must be greater than zero");
    }
    Ok(Duration::from_millis(millis))
}
