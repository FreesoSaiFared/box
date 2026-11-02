//! One guest request executes a mixed, preflighted program.
//! A successful input delivery is *not* a verified postcondition.
//! A journal with a pending effect stops replay until its effects are reconciled.
use std::collections::{HashMap, HashSet};
use std::path::{Path as FilePath, PathBuf};
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::Json;
use box_common::{resolve_in_canonical_jail, ApiError};
use box_cua::{validate_recipe, RecipeRequest};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{exec, files, AppState};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputRef {
    #[serde(rename = "from")]
    pub source: String,
    #[serde(default)]
    pub pointer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ProgramStep {
    Exec {
        id: String,
        command: Value,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        timeout_ms: Option<u64>,
        #[serde(default)]
        env: Option<HashMap<String, String>>,
        #[serde(default)]
        stdin: Option<String>,
        #[serde(default)]
        stdin_from: Option<OutputRef>,
    },
    WriteFile {
        id: String,
        path: String,
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        content_from: Option<OutputRef>,
        #[serde(default)]
        encoding: Option<String>,
    },
    ReadFile {
        id: String,
        path: String,
    },
    Cua {
        id: String,
        recipe: RecipeRequest,
    },
    Assert {
        id: String,
        #[serde(rename = "from")]
        source: String,
        #[serde(default)]
        pointer: String,
        #[serde(default)]
        equals: Option<Value>,
        #[serde(default)]
        contains: Option<String>,
        #[serde(default)]
        present: Option<bool>,
    },
    Wait {
        id: String,
        ms: u64,
    },
}

impl ProgramStep {
    fn id(&self) -> &str {
        match self {
            Self::Exec { id, .. }
            | Self::WriteFile { id, .. }
            | Self::ReadFile { id, .. }
            | Self::Cua { id, .. }
            | Self::Assert { id, .. }
            | Self::Wait { id, .. } => id,
        }
    }
    fn op(&self) -> &'static str {
        match self {
            Self::Exec { .. } => "exec",
            Self::WriteFile { .. } => "write_file",
            Self::ReadFile { .. } => "read_file",
            Self::Cua { .. } => "cua",
            Self::Assert { .. } => "assert",
            Self::Wait { .. } => "wait",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub journal: Option<String>,
    #[serde(default)]
    pub resume: bool,
    #[serde(default)]
    pub stop_on_error: Option<bool>,
    pub steps: Vec<ProgramStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramStepResult {
    pub id: String,
    pub op: String,
    pub ok: bool,
    /// delivered: tool returned; observed: read/inspection; verified: explicit assertion.
    pub stage: String,
    pub ms: u64,
    pub output: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramResponse {
    pub ok: bool,
    pub name: Option<String>,
    pub ran: usize,
    pub reused: usize,
    pub assertions_passed: usize,
    pub assertions_total: usize,
    pub elapsed_ms: u64,
    pub stopped_at: Option<String>,
    pub needs_reconciliation: Option<String>,
    pub steps: Vec<ProgramStepResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Journal {
    plan: Value,
    pending: Option<String>,
    receipt: ProgramResponse,
}

fn preflight(state: &AppState, req: &ProgramRequest) -> Result<Option<PathBuf>, ApiError> {
    if req.steps.is_empty() || req.steps.len() > 256 {
        return Err(ApiError::invalid_request(
            "program must contain 1..=256 steps",
        ));
    }
    if req.resume && req.journal.is_none() {
        return Err(ApiError::invalid_request("resume requires journal"));
    }
    let mut known = HashSet::<String>::new();
    for step in &req.steps {
        let id = step.id();
        if id.trim().is_empty() || !known.insert(id.to_string()) {
            return Err(ApiError::invalid_request(format!(
                "missing or duplicate step id: {id:?}"
            )));
        }
        match step {
            ProgramStep::Exec {
                command,
                cwd,
                stdin,
                stdin_from,
                ..
            } => {
                let spec: exec::CommandSpec = serde_json::from_value(command.clone())
                    .map_err(|e| ApiError::invalid_request(format!("{id}: command: {e}")))?;
                match spec {
                    exec::CommandSpec::Argv(v) if v.is_empty() || v[0].is_empty() => {
                        return Err(ApiError::invalid_request(format!("{id}: empty argv")))
                    }
                    exec::CommandSpec::Shell(s) if s.trim().is_empty() => {
                        return Err(ApiError::invalid_request(format!(
                            "{id}: empty shell command"
                        )))
                    }
                    _ => {}
                }
                if let Some(cwd) = cwd {
                    resolve_in_canonical_jail(&state.workspace, cwd)?;
                }
                if stdin.is_some() && stdin_from.is_some() {
                    return Err(ApiError::invalid_request(format!(
                        "{id}: choose stdin or stdin_from"
                    )));
                }
                if let Some(reference) = stdin_from {
                    check_ref(id, reference, &known, true)?;
                }
            }
            ProgramStep::WriteFile {
                path,
                content,
                content_from,
                ..
            } => {
                check_file(state, id, path)?;
                if content.is_some() == content_from.is_some() {
                    return Err(ApiError::invalid_request(format!(
                        "{id}: exactly one of content/content_from required"
                    )));
                }
                if let Some(reference) = content_from {
                    check_ref(id, reference, &known, true)?;
                }
            }
            ProgramStep::ReadFile { path, .. } => {
                check_file(state, id, path)?;
            }
            ProgramStep::Cua { recipe, .. } => {
                validate_recipe(&state.cua, recipe).map_err(|e| {
                    ApiError::invalid_request(format!("{id}: invalid CUA recipe: {e}"))
                })?;
            }
            ProgramStep::Assert {
                source,
                pointer,
                equals,
                contains,
                present,
                ..
            } => {
                if source == id || !known.contains(source) {
                    return Err(ApiError::invalid_request(format!(
                        "{id}: assertion refers to a future/unknown result {source}"
                    )));
                }
                check_pointer(id, pointer)?;
                if equals.is_none() && contains.is_none() && present.is_none() {
                    return Err(ApiError::invalid_request(format!(
                        "{id}: assertion has no expectation"
                    )));
                }
            }
            ProgramStep::Wait { ms, .. } if *ms > 10_000 => {
                return Err(ApiError::invalid_request(format!(
                    "{id}: wait exceeds 10000 ms"
                )))
            }
            ProgramStep::Wait { .. } => {}
        }
    }
    let journal = match &req.journal {
        Some(s) => Some(check_file(state, "journal", s)?),
        None => None,
    };
    Ok(journal)
}

fn check_file(state: &AppState, id: &str, path: &str) -> Result<PathBuf, ApiError> {
    if path.trim().is_empty() {
        return Err(ApiError::invalid_request(format!("{id}: empty path")));
    }
    let resolved = resolve_in_canonical_jail(&state.workspace, path)?;
    if resolved == state.workspace {
        return Err(ApiError::invalid_request(format!(
            "{id}: path is workspace root"
        )));
    }
    Ok(resolved)
}

fn check_pointer(id: &str, pointer: &str) -> Result<(), ApiError> {
    if !pointer.is_empty() && !pointer.starts_with('/') {
        return Err(ApiError::invalid_request(format!(
            "{id}: JSON pointer must start with /"
        )));
    }
    Ok(())
}

fn check_ref(
    id: &str,
    reference: &OutputRef,
    known: &HashSet<String>,
    self_is_known: bool,
) -> Result<(), ApiError> {
    if !known.contains(&reference.source) || (self_is_known && reference.source == id) {
        return Err(ApiError::invalid_request(format!(
            "{id}: unresolved reference {}",
            reference.source
        )));
    }
    check_pointer(id, &reference.pointer)
}

fn get_ref(outputs: &HashMap<String, Value>, reference: &OutputRef) -> Result<Value, String> {
    outputs
        .get(&reference.source)
        .ok_or_else(|| format!("result {} unavailable", reference.source))?
        .pointer(&reference.pointer)
        .cloned()
        .ok_or_else(|| {
            format!(
                "unobserved JSON pointer {} in {}",
                reference.pointer, reference.source
            )
        })
}

fn get_text(outputs: &HashMap<String, Value>, reference: &OutputRef) -> Result<String, String> {
    get_ref(outputs, reference)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("{}{} is not a string", reference.source, reference.pointer))
}

fn assert_result(
    outputs: &HashMap<String, Value>,
    source: &str,
    pointer: &str,
    equals: &Option<Value>,
    contains: &Option<String>,
    present: Option<bool>,
) -> Result<Value, String> {
    let root = outputs
        .get(source)
        .ok_or_else(|| format!("{source}: no observation"))?;
    let observed = root.pointer(pointer);
    if let Some(expected) = present {
        if observed.is_some() != expected {
            return Err(format!(
                "{source}{pointer}: expected present={expected}, got {}",
                observed.is_some()
            ));
        }
    }
    if let Some(expected) = equals {
        if observed != Some(expected) {
            return Err(format!(
                "{source}{pointer}: expected {expected}, observed {}",
                observed.cloned().unwrap_or(Value::Null)
            ));
        }
    }
    if let Some(needle) = contains {
        if !observed
            .and_then(Value::as_str)
            .is_some_and(|s| s.contains(needle))
        {
            return Err(format!("{source}{pointer}: does not contain {needle:?}"));
        }
    }
    Ok(json!({"source": source, "pointer": pointer, "verified": true, "observed": observed}))
}

async fn execute(
    state: &AppState,
    step: &ProgramStep,
    outputs: &HashMap<String, Value>,
) -> Result<(Value, &'static str), String> {
    match step {
        ProgramStep::Exec {
            command,
            cwd,
            timeout_ms,
            env,
            stdin,
            stdin_from,
            ..
        } => {
            let input = match stdin_from {
                Some(reference) => Some(get_text(outputs, reference)?),
                None => stdin.clone(),
            };
            let spec = serde_json::from_value(command.clone()).map_err(|e| format!("{e}"))?;
            let response = exec::handle(
                State(state.clone()),
                Json(exec::ExecRequest {
                    command: spec,
                    cwd: cwd.clone(),
                    timeout_ms: *timeout_ms,
                    env: env.clone(),
                    stdin: input,
                    pty: Some(false),
                    detach: Some(false),
                }),
            )
            .await
            .map_err(|e| format!("{e:?}"))?
            .0;
            let result = serde_json::to_value(&response).map_err(|e| e.to_string())?;
            if response.exit_code != Some(0) || response.timed_out {
                return Err(format!(
                    "exec exit={:?}, timed_out={}, output={}",
                    response.exit_code, response.timed_out, result
                ));
            }
            Ok((result, "delivered"))
        }
        ProgramStep::WriteFile {
            path,
            content,
            content_from,
            encoding,
            ..
        } => {
            let data = match content_from {
                Some(reference) => get_text(outputs, reference)?,
                None => content.clone().unwrap_or_default(),
            };
            let response = files::put(
                State(state.clone()),
                Json(files::FilePutRequest {
                    path: path.clone(),
                    content: data,
                    encoding: encoding.clone(),
                    create_dirs: Some(true),
                }),
            )
            .await
            .map_err(|e| format!("{e:?}"))?
            .0;
            Ok((
                serde_json::to_value(response).map_err(|e| e.to_string())?,
                "delivered",
            ))
        }
        ProgramStep::ReadFile { path, .. } => {
            let response = files::get(
                State(state.clone()),
                Query(files::FileQuery {
                    path: Some(path.clone()),
                    encoding: Some("utf8".into()),
                }),
            )
            .await
            .map_err(|e| format!("{e:?}"))?
            .0;
            Ok((
                serde_json::to_value(response).map_err(|e| e.to_string())?,
                "observed",
            ))
        }
        ProgramStep::Cua { recipe, .. } => {
            let response = box_cua::run_recipe(&state.cua, recipe)
                .await
                .map_err(|e| format!("CUA delivery: {e}"))?;
            let inspected = response.steps.iter().any(|s| s.observed.is_some());
            let ok = response.ok;
            let output = serde_json::to_value(response).map_err(|e| e.to_string())?;
            if !ok {
                return Err(format!("CUA step failed; see guest receipt: {output}"));
            }
            Ok((output, if inspected { "observed" } else { "delivered" }))
        }
        ProgramStep::Assert {
            source,
            pointer,
            equals,
            contains,
            present,
            ..
        } => Ok((
            assert_result(outputs, source, pointer, equals, contains, *present)?,
            "verified",
        )),
        ProgramStep::Wait { ms, .. } => {
            tokio::time::sleep(Duration::from_millis(*ms)).await;
            Ok((json!({"waited_ms": ms}), "delivered"))
        }
    }
}

async fn persist(path: &FilePath, data: &Journal) -> Result<(), ApiError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::io(e.to_string()))?;
    }
    let tmp = path.with_extension("guest-recipe.tmp");
    let bytes = serde_json::to_vec_pretty(data).map_err(|e| ApiError::io(e.to_string()))?;
    tokio::fs::write(&tmp, bytes)
        .await
        .map_err(|e| ApiError::io(e.to_string()))?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|e| ApiError::io(e.to_string()))?;
    Ok(())
}

pub async fn handle(
    State(state): State<AppState>,
    Json(req): Json<ProgramRequest>,
) -> Result<Json<ProgramResponse>, ApiError> {
    let journal_path = preflight(&state, &req)?;
    let plan =
        serde_json::to_value(&req.steps).map_err(|e| ApiError::invalid_request(e.to_string()))?;
    let started = Instant::now();
    let mut record = if req.resume {
        let path = journal_path.as_ref().expect("preflight enforced journal");
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| ApiError::io(e.to_string()))?;
        let stored: Journal = serde_json::from_slice(&bytes)
            .map_err(|e| ApiError::invalid_request(format!("invalid journal: {e}")))?;
        if stored.plan != plan {
            return Err(ApiError::invalid_request(
                "resume plan differs from the journal plan",
            ));
        }
        stored
    } else {
        if journal_path.as_ref().is_some_and(|p| p.exists()) {
            return Err(ApiError::invalid_request(
                "journal already exists; choose another path or resume",
            ));
        }
        Journal {
            plan,
            pending: None,
            receipt: ProgramResponse {
                ok: true,
                name: req.name.clone(),
                ran: 0,
                reused: 0,
                assertions_passed: 0,
                assertions_total: req
                    .steps
                    .iter()
                    .filter(|s| matches!(s, ProgramStep::Assert { .. }))
                    .count(),
                elapsed_ms: 0,
                stopped_at: None,
                needs_reconciliation: None,
                steps: Vec::new(),
            },
        }
    };
    if let Some(pending) = &record.pending {
        record.receipt.ok = false;
        record.receipt.needs_reconciliation = Some(pending.clone());
        record.receipt.stopped_at = Some(pending.clone());
        return Ok(Json(record.receipt));
    }
    let mut outputs = HashMap::<String, Value>::new();
    for item in &record.receipt.steps {
        if item.ok {
            outputs.insert(item.id.clone(), item.output.clone());
        }
    }
    if let Some(first_failed) = record.receipt.steps.iter().position(|r| !r.ok) {
        if first_failed + 1 != record.receipt.steps.len() {
            return Err(ApiError::invalid_request("resume would replay effects executed after a failed step; reconcile the journal first"));
        }
        record.receipt.steps.truncate(first_failed);
        record.receipt.ran = first_failed;
        record.receipt.assertions_passed = record
            .receipt
            .steps
            .iter()
            .filter(|r| r.stage == "verified")
            .count();
    }
    let reused = record.receipt.steps.len();
    record.receipt.reused = reused;
    record.receipt.stopped_at = None;
    record.receipt.needs_reconciliation = None;
    record.receipt.ok = true;
    for step in req.steps.iter().skip(reused) {
        let id = step.id().to_string();
        record.pending = Some(id.clone());
        if let Some(path) = &journal_path {
            persist(path, &record).await?;
        }
        let tick = Instant::now();
        let result = execute(&state, step, &outputs).await;
        let item = match result {
            Ok((output, stage)) => {
                outputs.insert(id.clone(), output.clone());
                ProgramStepResult {
                    id: id.clone(),
                    op: step.op().into(),
                    ok: true,
                    stage: stage.into(),
                    ms: tick.elapsed().as_millis() as u64,
                    output,
                    error: None,
                }
            }
            Err(error) => ProgramStepResult {
                id: id.clone(),
                op: step.op().into(),
                ok: false,
                stage: "failed".into(),
                ms: tick.elapsed().as_millis() as u64,
                output: Value::Null,
                error: Some(error),
            },
        };
        if item.stage == "verified" {
            record.receipt.assertions_passed += 1;
        }
        if !item.ok {
            record.receipt.ok = false;
            record.receipt.stopped_at = Some(id);
        }
        record.receipt.steps.push(item);
        record.receipt.ran += 1;
        record.receipt.elapsed_ms += tick.elapsed().as_millis() as u64;
        record.pending = None;
        if let Some(path) = &journal_path {
            persist(path, &record).await?;
        }
        if !record.receipt.ok && req.stop_on_error.unwrap_or(true) {
            break;
        }
    }
    let counted_ms: u64 = record.receipt.steps.iter().skip(reused).map(|x| x.ms).sum();
    record.receipt.elapsed_ms += (started.elapsed().as_millis() as u64).saturating_sub(counted_ms);
    Ok(Json(record.receipt))
}
