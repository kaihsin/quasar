use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, StatusCode},
    response::Response,
    routing::{get, patch},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    adapters,
    cache::{CacheOutcome, ResponseCache},
    clients::command_runner::{CommandRunner, SystemCommandRunner},
    config::{GitHubProject, JiraConfig},
    domain::{
        SourceWarning, SummaryResponse, WorkItem, WorkItemDetail, WorkItemsResponse, WorkSource,
    },
};

#[derive(Clone)]
pub enum GitHubSource {
    Fixture(PathBuf),
    Cli,
}

#[derive(Clone)]
pub enum JiraSource {
    Fixture(PathBuf),
    Cli,
}

#[derive(Clone)]
pub struct AppState {
    pub github_source: GitHubSource,
    pub jira_source: JiraSource,
    pub cache: Arc<ResponseCache>,
    pub date_cache: Arc<ResponseCache>,
    pub runner: Arc<dyn CommandRunner>,
    pub github_repos: Vec<String>,
    pub jira_queries: Vec<String>,
    pub jira_base_url: String,
    pub jira_people: Vec<String>,
    pub jira_jql: Option<String>,
    pub github_project: Option<GitHubProject>,
    pub jira_config: Option<JiraConfig>,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        github_source: GitHubSource,
        jira_source: JiraSource,
        cache_ttl_secs: u64,
        jira_date_cache_ttl_secs: u64,
        github_repos: Vec<String>,
        jira_queries: Vec<String>,
        jira_base_url: String,
        jira_people: Vec<String>,
        jira_jql: Option<String>,
        github_project: Option<GitHubProject>,
        jira_config: Option<JiraConfig>,
    ) -> Self {
        Self {
            github_source,
            jira_source,
            cache: Arc::new(ResponseCache::new(std::time::Duration::from_secs(
                cache_ttl_secs,
            ))),
            date_cache: Arc::new(ResponseCache::new(std::time::Duration::from_secs(
                jira_date_cache_ttl_secs,
            ))),
            runner: Arc::new(SystemCommandRunner),
            github_repos,
            jira_queries,
            jira_base_url,
            jira_people,
            jira_jql,
            github_project,
            jira_config,
        }
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/work-items", get(work_items))
        .route("/api/work-items/stream", get(work_items_stream))
        .route("/api/summary", get(summary))
        .route("/api/activity", get(activity))
        .route("/api/work-item-detail", get(work_item_detail))
        .route("/api/work-item-field", patch(update_work_item_field))
        .route("/api/work-item-assignees", patch(update_work_item_assignees))
        .route("/api/people", get(people))
        .route("/api/person-work-items", get(person_work_items))
        .with_state(state)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn work_items(State(state): State<AppState>) -> Json<WorkItemsResponse> {
    Json(fetch_work_items(&state))
}

// Streams work items as newline-delimited JSON (NDJSON). Each source (GitHub
// repo, Jira) emits an `items` chunk as soon as its CLI call resolves, so the
// frontend can render cards progressively instead of waiting for every source.
// A final `done` chunk carries the fetch metadata.
async fn work_items_stream(State(state): State<AppState>) -> Response {
    // Small buffer: the blocking resolver runs ahead of the client, and
    // back-pressure here is fine since each chunk is one source's batch.
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, std::convert::Infallible>>(16);

    // The source adapters use a blocking CommandRunner, so resolve off the
    // async runtime and hand each chunk back over the channel as it completes.
    tokio::task::spawn_blocking(move || {
        resolve_work_items(&state, |chunk| {
            if let Ok(mut line) = serde_json::to_string(&chunk) {
                line.push('\n');
                // Ignore send errors: they mean the client disconnected.
                let _ = tx.blocking_send(Ok(line));
            }
        });
    });

    Response::builder()
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(ReceiverStream::new(rx)))
        .expect("streaming response should build")
}

async fn summary(State(state): State<AppState>) -> Json<SummaryResponse> {
    let response = fetch_work_items(&state);
    let mut totals_by_source = BTreeMap::new();
    let mut totals_by_status = BTreeMap::new();

    for item in &response.data {
        *totals_by_source.entry(item.source.to_string()).or_insert(0) += 1;
        *totals_by_status.entry(item.status.clone()).or_insert(0) += 1;
    }

    Json(SummaryResponse {
        totals_by_source,
        totals_by_status,
        warnings: response.warnings,
        fetched_at: response.fetched_at,
        cache_status: response.cache_status,
    })
}

async fn activity(State(state): State<AppState>) -> Json<WorkItemsResponse> {
    let mut response = fetch_work_items(&state);
    response
        .data
        .sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Json(response)
}

#[derive(Deserialize)]
struct DetailQuery {
    id: String,
}

async fn work_item_detail(
    State(state): State<AppState>,
    Query(query): Query<DetailQuery>,
) -> Result<Json<WorkItemDetail>, (StatusCode, String)> {
    fetch_work_item_detail(&state, &query.id)
        .map(Json)
        .map_err(|error| (error.status, error.message))
}

#[derive(Debug)]
struct DetailError {
    status: StatusCode,
    message: String,
}

fn fetch_work_item_detail(state: &AppState, id: &str) -> Result<WorkItemDetail, DetailError> {
    if let Some(rest) = id.strip_prefix("github:") {
        let (repo, number) = rest.rsplit_once('#').ok_or_else(|| DetailError {
            status: StatusCode::BAD_REQUEST,
            message: format!("malformed GitHub id: {id}"),
        })?;
        if number.is_empty() {
            return Err(DetailError {
                status: StatusCode::BAD_REQUEST,
                message: format!("missing GitHub issue number in id: {id}"),
            });
        }
        let result = match &state.github_source {
            GitHubSource::Fixture(path) => {
                // Fixture mode: serve the sibling detail fixture regardless of repo/number.
                let detail_path = path.with_file_name("issue-detail.json");
                adapters::github::load_fixture_issue_detail(&detail_path, repo)
            }
            GitHubSource::Cli => {
                let mut detail =
                    match adapters::github::fetch_issue_detail(state.runner.as_ref(), repo, number)
                    {
                        Ok(detail) => detail,
                        Err(error) => {
                            return Err(DetailError {
                                status: StatusCode::BAD_GATEWAY,
                                message: error.to_string(),
                            })
                        }
                    };
                if let Some(project) = state.github_project.as_ref() {
                    let fields = adapters::github::enrich_detail_project_fields(
                        state.runner.as_ref(),
                        repo,
                        number,
                        project,
                    );
                    detail.item.start_date = fields.start_date;
                    detail.item.target_date = fields.target_date;
                    detail.project_status = fields.project_status;
                    detail.status_options = fields.status_options;
                }
                detail.assignee_options =
                    adapters::github::fetch_assignable_users(state.runner.as_ref(), repo)
                        .into_iter()
                        .map(|login| crate::domain::AssigneeOption {
                            id: login.clone(),
                            name: login,
                        })
                        .collect();
                Ok(detail)
            }
        };
        return result.map_err(|error| DetailError {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
        });
    }

    if let Some(key) = id.strip_prefix("jira:") {
        if key.is_empty() {
            return Err(DetailError {
                status: StatusCode::BAD_REQUEST,
                message: format!("missing Jira issue key in id: {id}"),
            });
        }
        let result = match &state.jira_source {
            JiraSource::Fixture(path) => {
                let detail_path = path.with_file_name("issue-detail.json");
                adapters::jira::load_fixture_issue_detail(&detail_path, &state.jira_base_url)
            }
            JiraSource::Cli => {
                let mut detail =
                    match adapters::jira::fetch_issue_detail(state.runner.as_ref(), key, &state.jira_base_url) {
                        Ok(detail) => detail,
                        Err(error) => {
                            return Err(DetailError {
                                status: StatusCode::BAD_GATEWAY,
                                message: error.to_string(),
                            })
                        }
                    };
                // With write credentials, surface the reachable workflow statuses
                // (transition targets) plus the current one so the UI can offer a
                // status dropdown. Best-effort: leaves options empty on failure.
                if let Some(jira) = state.jira_config.as_ref() {
                    let mut options =
                        adapters::jira::fetch_status_options(state.runner.as_ref(), jira, key);
                    if !options.contains(&detail.item.status) {
                        options.insert(0, detail.item.status.clone());
                    }
                    detail.project_status = Some(detail.item.status.clone());
                    detail.status_options = options;
                    detail.assignee_options =
                        adapters::jira::fetch_assignable_users(state.runner.as_ref(), jira, key);
                }
                Ok(detail)
            }
        };
        return result.map_err(|error| DetailError {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
        });
    }

    Err(DetailError {
        status: StatusCode::BAD_REQUEST,
        message: format!("unrecognized work-item id: {id}"),
    })
}

#[derive(Deserialize)]
struct UpdateFieldRequest {
    id: String,
    field: String,
    #[serde(default)]
    value: Option<String>,
}

#[derive(Serialize)]
struct UpdateDatesResponse {
    ok: bool,
}

async fn update_work_item_field(
    State(state): State<AppState>,
    Json(body): Json<UpdateFieldRequest>,
) -> Result<Json<UpdateDatesResponse>, (StatusCode, String)> {
    fetch_work_item_field(&state, &body)
        .map(|_| Json(UpdateDatesResponse { ok: true }))
        .map_err(|error| (error.status, error.message))
}

fn fetch_work_item_field(state: &AppState, body: &UpdateFieldRequest) -> Result<(), DetailError> {
    let field = body.field.as_str();
    if !matches!(field, "start" | "target" | "status") {
        return Err(DetailError {
            status: StatusCode::BAD_REQUEST,
            message: format!("unknown field: {field}"),
        });
    }

    // Validate date fields up front so malformed client input yields 400 rather
    // than a wasted CLI/HTTP round-trip. None/empty means "clear" and is valid.
    if matches!(field, "start" | "target") {
        if let Some(value) = body.value.as_deref() {
            if !value.is_empty() && !is_iso_date(value) {
                return Err(DetailError {
                    status: StatusCode::BAD_REQUEST,
                    message: "date must be YYYY-MM-DD".to_string(),
                });
            }
        }
    }

    if let Some(key) = body.id.strip_prefix("jira:") {
        return update_jira_field(state, key, field, body.value.as_deref());
    }

    let rest = body.id.strip_prefix("github:").ok_or_else(|| DetailError {
        status: StatusCode::BAD_REQUEST,
        message: format!("unrecognized work-item id: {}", body.id),
    })?;
    let (repo, number) = rest.rsplit_once('#').ok_or_else(|| DetailError {
        status: StatusCode::BAD_REQUEST,
        message: format!("malformed GitHub id: {}", body.id),
    })?;
    if number.is_empty() {
        return Err(DetailError {
            status: StatusCode::BAD_REQUEST,
            message: "missing issue number".to_string(),
        });
    }
    if !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(DetailError {
            status: StatusCode::BAD_REQUEST,
            message: "issue number must be numeric".to_string(),
        });
    }

    match &state.github_source {
        GitHubSource::Fixture(_) => {
            return Err(DetailError {
                status: StatusCode::CONFLICT,
                message: "writes unavailable in fixture mode".to_string(),
            })
        }
        GitHubSource::Cli => {}
    }

    let project = state.github_project.as_ref().ok_or_else(|| DetailError {
        status: StatusCode::CONFLICT,
        message: "no github_project configured; cannot edit fields".to_string(),
    })?;

    let result = match field {
        "start" => adapters::github::set_project_date(
            state.runner.as_ref(),
            repo,
            number,
            project,
            adapters::github::DateField::Start,
            body.value.as_deref(),
        ),
        "target" => adapters::github::set_project_date(
            state.runner.as_ref(),
            repo,
            number,
            project,
            adapters::github::DateField::Target,
            body.value.as_deref(),
        ),
        _ => adapters::github::set_project_status(
            state.runner.as_ref(),
            repo,
            number,
            project,
            body.value.as_deref().filter(|v| !v.is_empty()),
        ),
    };
    result.map_err(|error| DetailError {
        status: StatusCode::BAD_GATEWAY,
        message: error.to_string(),
    })?;

    state.cache.invalidate("work-items");
    Ok(())
}

// Write a Jira field via the REST API (dates as custom fields, status as a
// workflow transition). Date format is already validated by the caller.
fn update_jira_field(
    state: &AppState,
    key: &str,
    field: &str,
    value: Option<&str>,
) -> Result<(), DetailError> {
    if key.is_empty() {
        return Err(DetailError {
            status: StatusCode::BAD_REQUEST,
            message: "missing Jira issue key".to_string(),
        });
    }

    // Fixture mode has no live site to write to.
    if matches!(state.jira_source, JiraSource::Fixture(_)) {
        return Err(DetailError {
            status: StatusCode::CONFLICT,
            message: "writes unavailable in fixture mode".to_string(),
        });
    }

    let jira = state.jira_config.as_ref().ok_or_else(|| DetailError {
        status: StatusCode::CONFLICT,
        message: "no [jira] credentials configured; cannot edit Jira fields".to_string(),
    })?;

    let result = match field {
        "start" => adapters::jira::set_target_date(
            state.runner.as_ref(),
            jira,
            key,
            adapters::jira::DateField::Start,
            value,
        ),
        "target" => adapters::jira::set_target_date(
            state.runner.as_ref(),
            jira,
            key,
            adapters::jira::DateField::Target,
            value,
        ),
        // Jira status is workflow-driven; there is no "no status", so an empty
        // value is a client error rather than a clear.
        _ => match value.filter(|v| !v.is_empty()) {
            Some(status) => adapters::jira::set_status(state.runner.as_ref(), jira, key, status),
            None => {
                return Err(DetailError {
                    status: StatusCode::BAD_REQUEST,
                    message: "status value is required".to_string(),
                })
            }
        },
    };
    result.map_err(|error| DetailError {
        status: StatusCode::BAD_GATEWAY,
        message: error.to_string(),
    })?;

    state.cache.invalidate("work-items");
    Ok(())
}

#[derive(Deserialize)]
struct UpdateAssigneesRequest {
    id: String,
    #[serde(default)]
    assignee_ids: Vec<String>,
}

async fn update_work_item_assignees(
    State(state): State<AppState>,
    Json(body): Json<UpdateAssigneesRequest>,
) -> Result<Json<UpdateDatesResponse>, (StatusCode, String)> {
    set_work_item_assignees(&state, &body)
        .map(|_| Json(UpdateDatesResponse { ok: true }))
        .map_err(|error| (error.status, error.message))
}

fn set_work_item_assignees(
    state: &AppState,
    body: &UpdateAssigneesRequest,
) -> Result<(), DetailError> {
    if let Some(key) = body.id.strip_prefix("jira:") {
        if key.is_empty() {
            return Err(DetailError {
                status: StatusCode::BAD_REQUEST,
                message: "missing Jira issue key".to_string(),
            });
        }
        if body.assignee_ids.len() > 1 {
            return Err(DetailError {
                status: StatusCode::BAD_REQUEST,
                message: "Jira work items allow at most one assignee".to_string(),
            });
        }
        if matches!(state.jira_source, JiraSource::Fixture(_)) {
            return Err(DetailError {
                status: StatusCode::CONFLICT,
                message: "writes unavailable in fixture mode".to_string(),
            });
        }
        let jira = state.jira_config.as_ref().ok_or_else(|| DetailError {
            status: StatusCode::CONFLICT,
            message: "no [jira] credentials configured; cannot edit Jira fields".to_string(),
        })?;
        adapters::jira::set_assignee(
            state.runner.as_ref(),
            jira,
            key,
            body.assignee_ids.first().map(String::as_str),
        )
        .map_err(|error| DetailError {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
        })?;
        state.cache.invalidate("work-items");
        return Ok(());
    }

    let rest = body.id.strip_prefix("github:").ok_or_else(|| DetailError {
        status: StatusCode::BAD_REQUEST,
        message: format!("unrecognized work-item id: {}", body.id),
    })?;
    let (repo, number) = rest.rsplit_once('#').ok_or_else(|| DetailError {
        status: StatusCode::BAD_REQUEST,
        message: format!("malformed GitHub id: {}", body.id),
    })?;
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return Err(DetailError {
            status: StatusCode::BAD_REQUEST,
            message: "issue number must be numeric".to_string(),
        });
    }
    match &state.github_source {
        GitHubSource::Fixture(_) => {
            return Err(DetailError {
                status: StatusCode::CONFLICT,
                message: "writes unavailable in fixture mode".to_string(),
            })
        }
        GitHubSource::Cli => {}
    }
    adapters::github::set_assignees(state.runner.as_ref(), repo, number, &body.assignee_ids)
        .map_err(|error| DetailError {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
        })?;
    state.cache.invalidate("work-items");
    Ok(())
}

#[derive(Serialize)]
struct PeopleResponse {
    users: Vec<String>,
}

async fn people(State(state): State<AppState>) -> Json<PeopleResponse> {
    Json(PeopleResponse {
        users: state.jira_people.clone(),
    })
}

#[derive(Deserialize)]
struct PersonQuery {
    user: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersonWorkItemsResponse {
    user: String,
    account_id: Option<String>,
    created_by: Vec<WorkItem>,
    mentioned: Vec<WorkItem>,
}

async fn person_work_items(
    State(state): State<AppState>,
    Query(query): Query<PersonQuery>,
) -> Result<Json<PersonWorkItemsResponse>, (StatusCode, String)> {
    fetch_person_work_items(&state, &query.user)
        .map(Json)
        .map_err(|error| (error.status, error.message))
}

fn fetch_person_work_items(
    state: &AppState,
    user: &str,
) -> Result<PersonWorkItemsResponse, DetailError> {
    if !state.jira_people.iter().any(|u| u == user) {
        return Err(DetailError {
            status: StatusCode::BAD_REQUEST,
            message: format!("unknown configured person: {user}"),
        });
    }
    if matches!(state.jira_source, JiraSource::Fixture(_)) {
        return Ok(PersonWorkItemsResponse {
            user: user.to_string(),
            account_id: None,
            created_by: Vec::new(),
            mentioned: Vec::new(),
        });
    }

    let cache_key = format!("person:{user}");
    let now = Instant::now();
    if let CacheOutcome::Hit(payload) = state.cache.get(&cache_key, now) {
        if let Ok(cached) = serde_json::from_str::<PersonWorkItemsResponse>(&payload) {
            return Ok(cached);
        }
    }

    let runner = state.runner.as_ref();
    let account_id = adapters::jira::fetch_account_id_via_reporter(runner, user).map(|(id, _)| id);
    let queries = crate::config::compose_person_queries(
        user,
        account_id.as_deref(),
        state.jira_jql.as_deref(),
    );

    let created_by =
        adapters::jira::search_work_items(runner, &queries.created_by, &state.jira_base_url)
            .map_err(|error| DetailError {
                status: StatusCode::BAD_GATEWAY,
                message: error.to_string(),
            })?;

    // Mentioned is best-effort: a failure there must not sink created-by.
    let mut mentioned = match &queries.mentioned {
        Some(jql) => adapters::jira::search_work_items(runner, jql, &state.jira_base_url)
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let created_ids: std::collections::HashSet<&str> =
        created_by.iter().map(|item| item.id.as_str()).collect();
    mentioned.retain(|item| !created_ids.contains(item.id.as_str()));

    let response = PersonWorkItemsResponse {
        user: user.to_string(),
        account_id,
        created_by,
        mentioned,
    };
    if let Ok(payload) = serde_json::to_string(&response) {
        state.cache.insert(&cache_key, payload, now);
    }
    Ok(response)
}

fn is_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes.iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
}

// One line of the NDJSON stream. Borrows from the resolver's buffers so we can
// emit a source's items without cloning them before they land in the full set.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum StreamChunk<'a> {
    Items {
        data: &'a [WorkItem],
        warnings: &'a [SourceWarning],
    },
    Done {
        fetched_at: &'a str,
        cache_status: &'a str,
    },
}

fn fetch_work_items(state: &AppState) -> WorkItemsResponse {
    // Batch callers (summary, activity) don't need the incremental chunks.
    resolve_work_items(state, |_chunk| {})
}

// Core work-item resolution. Invokes `emit` with an `Items` chunk per source as
// it resolves (for streaming), then a final `Done` chunk, and returns the full
// merged response for batch callers. On a cache hit the cached payload is
// emitted as a single `Items` chunk.
fn resolve_work_items<F>(state: &AppState, mut emit: F) -> WorkItemsResponse
where
    F: FnMut(StreamChunk),
{
    let cache_key = "work-items";
    let now = Instant::now();

    if let CacheOutcome::Hit(payload) = state.cache.get(cache_key, now) {
        let mut response: WorkItemsResponse =
            serde_json::from_str(&payload).expect("cached work items payload should deserialize");
        response.cache_status = "hit".to_string();
        emit(StreamChunk::Items {
            data: &response.data,
            warnings: &response.warnings,
        });
        emit(StreamChunk::Done {
            fetched_at: &response.fetched_at,
            cache_status: &response.cache_status,
        });
        return response;
    }

    // Emits a resolved source's items as their own chunk, then folds them into
    // the accumulated set for caching and batch callers.
    fn emit_items<F: FnMut(StreamChunk)>(emit: &mut F, items: Vec<WorkItem>, data: &mut Vec<WorkItem>) {
        emit(StreamChunk::Items {
            data: &items,
            warnings: &[],
        });
        data.extend(items);
    }
    // Emits a source failure as a warning-only chunk, then records the warning.
    fn emit_warning<F: FnMut(StreamChunk)>(
        emit: &mut F,
        warning: SourceWarning,
        warnings: &mut Vec<SourceWarning>,
    ) {
        emit(StreamChunk::Items {
            data: &[],
            warnings: std::slice::from_ref(&warning),
        });
        warnings.push(warning);
    }

    let mut data = Vec::new();
    let mut warnings = Vec::new();

    match &state.github_source {
        GitHubSource::Fixture(path) => match adapters::github::load_fixture_work_items(path) {
            Ok(items) => emit_items(&mut emit, items, &mut data),
            Err(error) => emit_warning(
                &mut emit,
                SourceWarning {
                    source: WorkSource::GitHub,
                    message: error.to_string(),
                },
                &mut warnings,
            ),
        },
        GitHubSource::Cli => {
            if state.github_repos.is_empty() {
                emit_warning(
                    &mut emit,
                    SourceWarning {
                        source: WorkSource::GitHub,
                        message: "No GitHub repos configured for CLI mode".to_string(),
                    },
                    &mut warnings,
                );
            } else {
                for repo in &state.github_repos {
                    match adapters::github::load_work_items_with_runner(
                        state.runner.as_ref(),
                        repo,
                        state.github_project.as_ref(),
                    ) {
                        Ok(items) => emit_items(&mut emit, items, &mut data),
                        Err(error) => emit_warning(
                            &mut emit,
                            SourceWarning {
                                source: WorkSource::GitHub,
                                message: format!("GitHub repo {repo} failed: {error}"),
                            },
                            &mut warnings,
                        ),
                    }
                }
            }
        }
    }

    match &state.jira_source {
        JiraSource::Fixture(path) => match adapters::jira::load_fixture_work_items(path, &state.jira_base_url) {
            Ok(items) => emit_items(&mut emit, items, &mut data),
            Err(error) => emit_warning(
                &mut emit,
                SourceWarning {
                    source: WorkSource::Jira,
                    message: error.to_string(),
                },
                &mut warnings,
            ),
        },
        // One `acli` query per configured Jira project, emitted as its own chunk
        // so each project streams independently (mirroring the GitHub repo loop).
        // One project failing surfaces a warning without sinking the others.
        JiraSource::Cli => {
            for jql in &state.jira_queries {
                match adapters::jira::load_work_items_with_runner(state.runner.as_ref(), jql, &state.jira_base_url) {
                    Ok(items) => emit_items(&mut emit, items, &mut data),
                    Err(error) => emit_warning(
                        &mut emit,
                        SourceWarning {
                            source: WorkSource::Jira,
                            message: error.to_string(),
                        },
                        &mut warnings,
                    ),
                }
            }
        }
    }

    data.sort_by(|left, right| left.id.cmp(&right.id));
    // A ticket can match multiple queries (e.g. a board project and the person
    // query); collapse duplicates by id (sorted above, so dups are adjacent).
    data.dedup_by(|a, b| a.id == b.id);

    let response = WorkItemsResponse {
        data,
        warnings,
        fetched_at: timestamp_string(),
        cache_status: "miss".to_string(),
    };

    let payload =
        serde_json::to_string(&response).expect("work items response should serialize to cache");
    state.cache.insert(cache_key, payload, now);

    emit(StreamChunk::Done {
        fetched_at: &response.fetched_at,
        cache_status: &response.cache_status,
    });

    response
}

fn timestamp_string() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_secs()
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        path::PathBuf,
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };

    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use serde_json::Value;
    use tower::ServiceExt;

    use super::{
        fetch_work_item_field, fetch_work_items, resolve_work_items, router,
        set_work_item_assignees, AppState, GitHubSource, JiraSource, StreamChunk,
        UpdateAssigneesRequest, UpdateFieldRequest,
    };
    use crate::{
        cache::{CacheOutcome, ResponseCache},
        clients::command_runner::{CommandResult, CommandRunner, CommandRunnerError},
        config::GitHubProject,
        domain::WorkSource,
    };

    struct MockCommandRunner {
        responses: HashMap<String, CommandResult<String>>,
        calls: Mutex<Vec<(String, Vec<String>)>>,
    }

    impl MockCommandRunner {
        fn new(responses: HashMap<String, CommandResult<String>>) -> Self {
            Self {
                responses,
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl CommandRunner for MockCommandRunner {
        fn run(&self, program: &str, args: &[&str]) -> CommandResult<String> {
            self.calls
                .lock()
                .expect("calls mutex should not be poisoned")
                .push((
                    program.to_string(),
                    args.iter().map(|arg| (*arg).to_string()).collect(),
                ));

            let repo = args
                .windows(2)
                .find_map(|window| (window[0] == "-R").then_some(window[1]))
                .unwrap_or("")
                .to_string();

            self.responses
                .get(&repo)
                .cloned()
                .unwrap_or_else(|| Err(CommandRunnerError::new(format!("unexpected repo: {repo}"))))
        }
    }

    fn fixture_path(source: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(source)
            .join("issues.json")
    }

    fn app_state(github_source: PathBuf, jira_source: PathBuf) -> AppState {
        AppState {
            github_source: GitHubSource::Fixture(github_source),
            jira_source: JiraSource::Fixture(jira_source),
            cache: Arc::new(ResponseCache::new(Duration::from_secs(30))),
            date_cache: Arc::new(ResponseCache::new(std::time::Duration::from_secs(600))),
            runner: Arc::new(crate::clients::command_runner::SystemCommandRunner),
            github_repos: Vec::new(),
            jira_queries: vec!["order by updated desc".to_string()],
            jira_base_url: "https://quera.atlassian.net".to_string(),
            jira_people: Vec::new(),
            jira_jql: None,
            github_project: None,
            jira_config: None,
        }
    }

    #[tokio::test]
    async fn work_items_endpoint_returns_merged_fixture_data() {
        let app = router(app_state(fixture_path("github"), fixture_path("jira")));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/work-items")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should be produced");

        assert_eq!(response.status(), StatusCode::OK);

        let body = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let payload: Value = serde_json::from_slice(&body).expect("payload should be json");

        assert_eq!(
            payload["data"]
                .as_array()
                .expect("data should be array")
                .len(),
            2
        );
        assert_eq!(
            payload["warnings"]
                .as_array()
                .expect("warnings should be array")
                .len(),
            0
        );
        assert_eq!(payload["cache_status"], "miss");
    }

    #[tokio::test]
    async fn summary_endpoint_returns_aggregated_counts() {
        let app = router(app_state(fixture_path("github"), fixture_path("jira")));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/summary")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should be produced");

        assert_eq!(response.status(), StatusCode::OK);

        let body = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let payload: Value = serde_json::from_slice(&body).expect("payload should be json");

        assert_eq!(payload["totals_by_source"]["github"], 1);
        assert_eq!(payload["totals_by_source"]["jira"], 1);
        assert_eq!(payload["cache_status"], "miss");
    }

    #[tokio::test]
    async fn work_items_endpoint_returns_partial_data_when_one_source_fails() {
        let app = router(app_state(
            fixture_path("github"),
            fixture_path("jira").join("missing.json"),
        ));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/work-items")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should be produced");

        assert_eq!(response.status(), StatusCode::OK);

        let body = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let payload: Value = serde_json::from_slice(&body).expect("payload should be json");

        assert_eq!(
            payload["data"]
                .as_array()
                .expect("data should be array")
                .len(),
            1
        );
        assert_eq!(
            payload["warnings"]
                .as_array()
                .expect("warnings should be array")
                .len(),
            1
        );
        assert_eq!(payload["warnings"][0]["source"], "jira");
        assert_eq!(payload["cache_status"], "miss");
    }

    #[tokio::test]
    async fn work_items_stream_emits_a_chunk_per_source_then_done() {
        let app = router(app_state(fixture_path("github"), fixture_path("jira")));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/work-items/stream")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should be produced");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .expect("content-type header should be set"),
            "application/x-ndjson"
        );

        let body = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let text = String::from_utf8(body.to_vec()).expect("body should be utf-8");

        let chunks: Vec<Value> = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).expect("each line should be json"))
            .collect();

        // Two source chunks (one github item, one jira item) plus a done chunk.
        let item_chunks: Vec<&Value> = chunks
            .iter()
            .filter(|chunk| chunk["type"] == "items")
            .collect();
        let total_items: usize = item_chunks
            .iter()
            .map(|chunk| chunk["data"].as_array().map(|d| d.len()).unwrap_or(0))
            .sum();
        assert_eq!(total_items, 2);

        let done = chunks.last().expect("stream should end with a chunk");
        assert_eq!(done["type"], "done");
        assert_eq!(done["cache_status"], "miss");
        assert!(done["fetched_at"].is_string());
    }

    #[tokio::test]
    async fn work_items_endpoint_returns_cache_hit_on_second_request() {
        let app = router(app_state(fixture_path("github"), fixture_path("jira")));

        let first_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/work-items")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("first response should be produced");
        let first_body = first_response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let first_payload: Value =
            serde_json::from_slice(&first_body).expect("payload should be json");

        let second_response = app
            .oneshot(
                Request::builder()
                    .uri("/api/work-items")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("second response should be produced");
        let second_body = second_response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let second_payload: Value =
            serde_json::from_slice(&second_body).expect("payload should be json");

        assert_eq!(first_payload["cache_status"], "miss");
        assert_eq!(second_payload["cache_status"], "hit");
    }

    #[test]
    fn fetch_work_items_keeps_successful_github_repo_items_when_one_repo_fails() {
        let github_payload =
            std::fs::read_to_string(fixture_path("github")).expect("fixture should read");
        let state = AppState {
            github_source: GitHubSource::Cli,
            jira_source: JiraSource::Fixture(fixture_path("jira")),
            cache: Arc::new(ResponseCache::new(Duration::from_secs(30))),
            date_cache: Arc::new(ResponseCache::new(std::time::Duration::from_secs(600))),
            runner: Arc::new(MockCommandRunner::new(HashMap::from([
                ("openai/quasar".to_string(), Ok(github_payload)),
                (
                    "rust-lang/rust".to_string(),
                    Err(CommandRunnerError::new("gh auth expired")),
                ),
            ]))),
            github_repos: vec!["openai/quasar".to_string(), "rust-lang/rust".to_string()],
            jira_queries: vec!["order by updated desc".to_string()],
            jira_base_url: "https://quera.atlassian.net".to_string(),
            jira_people: Vec::new(),
            jira_jql: None,
            github_project: None,
            jira_config: None,
        };

        let response = fetch_work_items(&state);

        assert_eq!(response.data.len(), 2);
        assert!(
            response
                .data
                .iter()
                .any(|item| item.repo.as_deref() == Some("openai/quasar"))
        );
        assert_eq!(response.warnings.len(), 1);
        assert_eq!(response.warnings[0].source, WorkSource::GitHub);
        assert!(response.warnings[0].message.contains("rust-lang/rust"));
        assert!(response.warnings[0].message.contains("gh auth expired"));
    }

    // Matches `acli` calls by their `--jql` argument; treats `view` (planning-date
    // enrichment) calls as failures so enrichment stays best-effort in tests.
    struct JiraQueryMock {
        by_jql: HashMap<String, CommandResult<String>>,
    }

    impl CommandRunner for JiraQueryMock {
        fn run(&self, _program: &str, args: &[&str]) -> CommandResult<String> {
            if args.iter().any(|arg| *arg == "view") {
                return Err(CommandRunnerError::new("no per-issue view in test"));
            }
            let jql = args
                .windows(2)
                .find_map(|window| (window[0] == "--jql").then_some(window[1]))
                .unwrap_or("");
            self.by_jql
                .get(jql)
                .cloned()
                .unwrap_or_else(|| Err(CommandRunnerError::new(format!("unexpected jql: {jql}"))))
        }
    }

    fn jira_search_payload(key: &str, summary: &str) -> String {
        format!(
            r#"[{{"key":"{key}","fields":{{"summary":"{summary}","status":{{"name":"To Do"}}}}}}]"#
        )
    }

    fn jira_cli_state(runner: JiraQueryMock, jira_queries: Vec<String>) -> AppState {
        AppState {
            // Empty GitHub repos: emits a warning-only chunk with no items, so it
            // doesn't interfere with the Jira item chunks under test.
            github_source: GitHubSource::Cli,
            jira_source: JiraSource::Cli,
            cache: Arc::new(ResponseCache::new(Duration::from_secs(30))),
            date_cache: Arc::new(ResponseCache::new(std::time::Duration::from_secs(600))),
            runner: Arc::new(runner),
            github_repos: Vec::new(),
            jira_queries,
            jira_base_url: "https://quera.atlassian.net".to_string(),
            jira_people: Vec::new(),
            jira_jql: None,
            github_project: None,
            jira_config: None,
        }
    }

    // Mirrors `jira_cli_state` but with `JiraSource::Cli` and configured people,
    // for exercising the People-page person-work-items path.
    fn jira_cli_state_people<R: CommandRunner + 'static>(
        runner: R,
        people: Vec<String>,
    ) -> AppState {
        AppState {
            github_source: GitHubSource::Cli,
            jira_source: JiraSource::Cli,
            cache: Arc::new(ResponseCache::new(Duration::from_secs(30))),
            date_cache: Arc::new(ResponseCache::new(std::time::Duration::from_secs(600))),
            runner: Arc::new(runner),
            github_repos: Vec::new(),
            jira_queries: Vec::new(),
            jira_base_url: "https://quera.atlassian.net".to_string(),
            jira_people: people,
            jira_jql: None,
            github_project: None,
            jira_config: None,
        }
    }

    #[tokio::test]
    async fn people_endpoint_lists_configured_users() {
        let mut state = app_state(fixture_path("github"), fixture_path("jira"));
        state.jira_people = vec!["a@x".to_string(), "b@x".to_string()];
        let app = router(state);
        let response = app
            .oneshot(Request::builder().uri("/api/people").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["users"][0], "a@x");
        assert_eq!(payload["users"][1], "b@x");
    }

    #[test]
    fn person_work_items_rejects_unconfigured_user() {
        let mut state = app_state(fixture_path("github"), fixture_path("jira"));
        state.jira_people = vec!["a@x".to_string()];
        let error = super::fetch_person_work_items(&state, "stranger@x")
            .expect_err("unconfigured user should be rejected");
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn person_work_items_fixture_mode_returns_empty() {
        let mut state = app_state(fixture_path("github"), fixture_path("jira"));
        state.jira_people = vec!["a@x".to_string()];
        let result = super::fetch_person_work_items(&state, "a@x").expect("ok");
        assert!(result.created_by.is_empty() && result.mentioned.is_empty());
        assert_eq!(result.user, "a@x");
        assert_eq!(result.account_id, None);
    }

    #[test]
    fn person_work_items_dedupes_mentioned_against_created_by() {
        // Cli runner answering: (a) reporter --limit 1 resolve -> accountId acc:1,
        // (b) created-by search -> SSW-1, (c) mentioned text~ search -> SSW-1 + SSW-2.
        // Expect created_by=[SSW-1], mentioned=[SSW-2] (SSW-1 removed as dup).
        struct Runner;
        impl CommandRunner for Runner {
            fn run(&self, _p: &str, args: &[&str]) -> CommandResult<String> {
                let jql = args
                    .windows(2)
                    .find_map(|w| (w[0] == "--jql").then_some(w[1]))
                    .unwrap_or("");
                let has_limit = args.iter().any(|a| *a == "--limit");
                if jql.starts_with("reporter =") && has_limit {
                    Ok(r#"[{"key":"SSW-1","fields":{"reporter":{"accountId":"acc:1","displayName":"Ann"}}}]"#.to_string())
                } else if jql.starts_with("reporter =") {
                    Ok(jira_search_payload("SSW-1", "created"))
                } else {
                    // text ~ "acc:1" mentioned search -> two issues
                    Ok(format!(
                        r#"[{{"key":"SSW-1","fields":{{"summary":"created","status":{{"name":"To Do"}}}}}},{{"key":"SSW-2","fields":{{"summary":"mention","status":{{"name":"To Do"}}}}}}]"#
                    ))
                }
            }
        }
        let state = jira_cli_state_people(Runner, vec!["a@x".to_string()]);
        let result = super::fetch_person_work_items(&state, "a@x").expect("ok");
        let created: Vec<&str> = result.created_by.iter().map(|i| i.external_id.as_str()).collect();
        let mentioned: Vec<&str> = result.mentioned.iter().map(|i| i.external_id.as_str()).collect();
        assert_eq!(created, vec!["SSW-1"]);
        assert_eq!(mentioned, vec!["SSW-2"]);
        assert_eq!(result.account_id.as_deref(), Some("acc:1"));
    }

    #[test]
    fn resolve_work_items_streams_one_chunk_per_jira_project() {
        let queries = vec![
            "project = SSW ORDER BY updated DESC".to_string(),
            "project = TEI ORDER BY updated DESC".to_string(),
        ];
        let runner = JiraQueryMock {
            by_jql: HashMap::from([
                (queries[0].clone(), Ok(jira_search_payload("SSW-1", "S one"))),
                (queries[1].clone(), Ok(jira_search_payload("TEI-1", "T one"))),
            ]),
        };
        let state = jira_cli_state(runner, queries);

        // Record the number of Jira items in each emitted chunk.
        let mut jira_chunk_sizes: Vec<usize> = Vec::new();
        resolve_work_items(&state, |chunk| {
            if let StreamChunk::Items { data, .. } = chunk {
                let jira = data
                    .iter()
                    .filter(|item| item.source == WorkSource::Jira)
                    .count();
                if jira > 0 {
                    jira_chunk_sizes.push(jira);
                }
            }
        });

        // Two projects -> two separate item chunks, one item each.
        assert_eq!(jira_chunk_sizes, vec![1, 1]);
    }

    #[test]
    fn fetch_work_items_keeps_successful_jira_project_items_when_one_query_fails() {
        let queries = vec![
            "project = SSW ORDER BY updated DESC".to_string(),
            "project = TEI ORDER BY updated DESC".to_string(),
        ];
        let runner = JiraQueryMock {
            by_jql: HashMap::from([
                (queries[0].clone(), Ok(jira_search_payload("SSW-1", "S one"))),
                (
                    queries[1].clone(),
                    Err(CommandRunnerError::new("acli auth expired")),
                ),
            ]),
        };
        let state = jira_cli_state(runner, queries);

        let response = fetch_work_items(&state);

        let jira_items: Vec<_> = response
            .data
            .iter()
            .filter(|item| item.source == WorkSource::Jira)
            .collect();
        assert_eq!(jira_items.len(), 1);
        assert_eq!(jira_items[0].container, "SSW");

        let jira_warnings: Vec<_> = response
            .warnings
            .iter()
            .filter(|warning| warning.source == WorkSource::Jira)
            .collect();
        assert_eq!(jira_warnings.len(), 1);
        assert!(jira_warnings[0].message.contains("acli auth expired"));
    }

    #[test]
    fn resolve_work_items_dedupes_items_with_same_id() {
        let queries = vec![
            "project = SSW ORDER BY updated DESC".to_string(),
            "(assignee in (\"a@x\")) ORDER BY updated DESC".to_string(),
        ];
        let runner = JiraQueryMock {
            by_jql: HashMap::from([
                (queries[0].clone(), Ok(jira_search_payload("SSW-1", "dup"))),
                (queries[1].clone(), Ok(jira_search_payload("SSW-1", "dup"))),
            ]),
        };
        let state = jira_cli_state(runner, queries);
        let response = fetch_work_items(&state);
        let ssw1 = response
            .data
            .iter()
            .filter(|item| item.id == "jira:SSW-1")
            .count();
        assert_eq!(ssw1, 1, "duplicate ids should be collapsed");
    }

    #[tokio::test]
    async fn detail_endpoint_returns_github_detail_from_fixture() {
        let app = router(app_state(fixture_path("github"), fixture_path("jira")));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/work-item-detail?id=github:openai/quasar%23123")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should be produced");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let payload: Value = serde_json::from_slice(&body).expect("payload should be json");
        assert_eq!(payload["item"]["source"], "github");
        assert!(
            payload["comments"]
                .as_array()
                .expect("comments array")
                .len()
                >= 1
        );
    }

    #[tokio::test]
    async fn detail_endpoint_returns_jira_detail_from_fixture() {
        let app = router(app_state(fixture_path("github"), fixture_path("jira")));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/work-item-detail?id=jira:ABC-42")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should be produced");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let payload: Value = serde_json::from_slice(&body).expect("payload should be json");
        assert_eq!(payload["item"]["source"], "jira");
    }

    #[tokio::test]
    async fn detail_endpoint_rejects_unparseable_id() {
        let app = router(app_state(fixture_path("github"), fixture_path("jira")));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/work-item-detail?id=nonsense")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should be produced");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn detail_endpoint_rejects_github_id_without_hash() {
        let app = router(app_state(fixture_path("github"), fixture_path("jira")));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/work-item-detail?id=github:foo")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should be produced");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn detail_endpoint_rejects_github_id_with_empty_number() {
        let app = router(app_state(fixture_path("github"), fixture_path("jira")));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/work-item-detail?id=github:openai/quasar%23")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should be produced");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn update_field_rejects_fixture_mode_for_start() {
        let app = router(app_state(fixture_path("github"), fixture_path("jira")));
        let response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/work-item-field")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"id":"github:openai/quasar#123","field":"start","value":"2026-07-01"}"#,
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn update_field_rejects_jira_id() {
        let app = router(app_state(fixture_path("github"), fixture_path("jira")));
        let response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/work-item-field")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"id":"jira:ABC-42","field":"status","value":"Done"}"#,
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn update_field_rejects_bad_field() {
        let app = router(app_state(fixture_path("github"), fixture_path("jira")));
        let response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/work-item-field")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"id":"github:o/r#1","field":"middle","value":"x"}"#,
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn update_field_rejects_fixture_mode_for_status() {
        let app = router(app_state(fixture_path("github"), fixture_path("jira")));
        let response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/work-item-field")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"id":"github:o/r#1","field":"status","value":"Done"}"#,
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn update_field_rejects_bad_date_for_start() {
        let app = router(app_state(fixture_path("github"), fixture_path("jira")));
        let response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/work-item-field")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"id":"github:o/r#1","field":"start","value":"07/01/2026"}"#,
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn update_field_rejects_non_numeric_number() {
        let app = router(app_state(fixture_path("github"), fixture_path("jira")));
        let response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/work-item-field")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"id":"github:o/r#abc","field":"start","value":"2026-07-01"}"#,
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn update_field_success_invalidates_work_items_cache() {
        // Routing runner answers both the CLI-mode work-items population
        // (issue list + enrichment graphql) and the date-write graphql sequence.
        struct RoutingRunner {
            issues: String,
        }
        impl CommandRunner for RoutingRunner {
            fn run(&self, _program: &str, args: &[&str]) -> CommandResult<String> {
                if !args.contains(&"graphql") {
                    // `gh issue list`
                    return Ok(self.issues.clone());
                }
                let query = args
                    .iter()
                    .find_map(|arg| arg.strip_prefix("query="))
                    .unwrap_or("");
                if query.contains("projectV2(number") {
                    Ok(
                        r#"{"data":{"organization":{"projectV2":{"id":"PVT_1","fields":{"nodes":[
                        {"id":"FLD_START","name":"Start date"},
                        {"id":"FLD_TARGET","name":"Target date"}
                    ]}}}}}"#
                            .to_string(),
                    )
                } else if query.contains("issue(number") {
                    Ok(
                        r#"{"data":{"repository":{"issue":{"id":"ISSUE_1","projectItems":{"nodes":[
                        {"id":"ITEM_1","project":{"number":18}}
                    ]}}}}}"#
                            .to_string(),
                    )
                } else if query.contains("updateProjectV2ItemFieldValue") {
                    Ok(
                        r#"{"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"ITEM_1"}}}}"#
                            .to_string(),
                    )
                } else {
                    // enrichment query (repo-scoped issues); return empty page.
                    Ok(r#"{"data":{"repository":{"issues":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[]}}}}"#.to_string())
                }
            }
        }

        let state = AppState {
            github_source: GitHubSource::Cli,
            jira_source: JiraSource::Fixture(fixture_path("jira")),
            cache: Arc::new(ResponseCache::new(Duration::from_secs(30))),
            date_cache: Arc::new(ResponseCache::new(std::time::Duration::from_secs(600))),
            runner: Arc::new(RoutingRunner {
                issues: std::fs::read_to_string(fixture_path("github"))
                    .expect("fixture should read"),
            }),
            github_repos: vec!["openai/quasar".to_string()],
            jira_queries: vec!["order by updated desc".to_string()],
            jira_base_url: "https://quera.atlassian.net".to_string(),
            jira_people: Vec::new(),
            jira_jql: None,
            github_project: Some(GitHubProject {
                owner: "QuEraComputing".into(),
                number: 18,
                start_date_field: "Start date".into(),
                target_date_field: "Target date".into(),
                status_field: "Status".into(),
            }),
            jira_config: None,
        };

        // Populate the cache (miss on first fetch).
        let populated = fetch_work_items(&state);
        assert_eq!(populated.cache_status, "miss");
        assert!(matches!(
            state.cache.get("work-items", Instant::now()),
            CacheOutcome::Hit(_)
        ));

        // A successful write must invalidate the cache.
        let result = fetch_work_item_field(
            &state,
            &UpdateFieldRequest {
                id: "github:openai/quasar#123".to_string(),
                field: "start".to_string(),
                value: Some("2026-07-01".to_string()),
            },
        );
        assert!(
            result.is_ok(),
            "expected ok, got status {:?}",
            result.err().map(|error| error.status)
        );
        assert_eq!(
            state.cache.get("work-items", Instant::now()),
            CacheOutcome::Miss
        );
    }

    #[test]
    fn update_field_status_success_invalidates_work_items_cache() {
        // Routing runner answers both the CLI-mode work-items population
        // (issue list + enrichment graphql) and the status-write graphql sequence.
        struct RoutingRunner {
            issues: String,
        }
        impl CommandRunner for RoutingRunner {
            fn run(&self, _program: &str, args: &[&str]) -> CommandResult<String> {
                if !args.contains(&"graphql") {
                    // `gh issue list`
                    return Ok(self.issues.clone());
                }
                let query = args
                    .iter()
                    .find_map(|arg| arg.strip_prefix("query="))
                    .unwrap_or("");
                if query.contains("projectV2(number") {
                    Ok(
                        r#"{"data":{"organization":{"projectV2":{"id":"PVT_1","fields":{"nodes":[
                        {"id":"FLD_STATUS","name":"Status","options":[
                            {"id":"OPT_TODO","name":"Todo"},{"id":"OPT_DONE","name":"Done"}]}
                    ]}}}}}"#
                            .to_string(),
                    )
                } else if query.contains("issue(number") {
                    Ok(
                        r#"{"data":{"repository":{"issue":{"id":"ISSUE_1","projectItems":{"nodes":[
                        {"id":"ITEM_1","project":{"number":18}}
                    ]}}}}}"#
                            .to_string(),
                    )
                } else if query.contains("updateProjectV2ItemFieldValue") {
                    Ok(
                        r#"{"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"ITEM_1"}}}}"#
                            .to_string(),
                    )
                } else {
                    // enrichment query (repo-scoped issues); return empty page.
                    Ok(r#"{"data":{"repository":{"issues":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[]}}}}"#.to_string())
                }
            }
        }

        let state = AppState {
            github_source: GitHubSource::Cli,
            jira_source: JiraSource::Fixture(fixture_path("jira")),
            cache: Arc::new(ResponseCache::new(Duration::from_secs(30))),
            date_cache: Arc::new(ResponseCache::new(std::time::Duration::from_secs(600))),
            runner: Arc::new(RoutingRunner {
                issues: std::fs::read_to_string(fixture_path("github"))
                    .expect("fixture should read"),
            }),
            github_repos: vec!["openai/quasar".to_string()],
            jira_queries: vec!["order by updated desc".to_string()],
            jira_base_url: "https://quera.atlassian.net".to_string(),
            jira_people: Vec::new(),
            jira_jql: None,
            github_project: Some(GitHubProject {
                owner: "QuEraComputing".into(),
                number: 18,
                start_date_field: "Start date".into(),
                target_date_field: "Target date".into(),
                status_field: "Status".into(),
            }),
            jira_config: None,
        };

        // Populate the cache (miss on first fetch).
        let populated = fetch_work_items(&state);
        assert_eq!(populated.cache_status, "miss");
        assert!(matches!(
            state.cache.get("work-items", Instant::now()),
            CacheOutcome::Hit(_)
        ));

        // A successful status write must invalidate the cache.
        let result = fetch_work_item_field(
            &state,
            &UpdateFieldRequest {
                id: "github:openai/quasar#123".to_string(),
                field: "status".to_string(),
                value: Some("Done".to_string()),
            },
        );
        assert!(
            result.is_ok(),
            "expected ok, got status {:?}",
            result.err().map(|error| error.status)
        );
        assert_eq!(
            state.cache.get("work-items", Instant::now()),
            CacheOutcome::Miss
        );
    }

    #[test]
    fn fetch_work_items_warns_when_no_github_repos_are_configured_for_cli_mode() {
        let state = AppState {
            github_source: GitHubSource::Cli,
            jira_source: JiraSource::Fixture(fixture_path("jira")),
            cache: Arc::new(ResponseCache::new(Duration::from_secs(30))),
            date_cache: Arc::new(ResponseCache::new(std::time::Duration::from_secs(600))),
            runner: Arc::new(MockCommandRunner::new(HashMap::new())),
            github_repos: Vec::new(),
            jira_queries: vec!["order by updated desc".to_string()],
            jira_base_url: "https://quera.atlassian.net".to_string(),
            jira_people: Vec::new(),
            jira_jql: None,
            github_project: None,
            jira_config: None,
        };

        let response = fetch_work_items(&state);

        assert_eq!(response.data.len(), 1);
        assert_eq!(response.warnings.len(), 1);
        assert_eq!(response.warnings[0].source, WorkSource::GitHub);
        assert!(response.warnings[0]
            .message
            .contains("No GitHub repos configured"));
    }

    #[test]
    fn work_item_detail_cli_includes_enriched_project_fields() {
        use crate::config::GitHubProject;
        struct Runner;
        impl CommandRunner for Runner {
            fn run(&self, _p: &str, args: &[&str]) -> CommandResult<String> {
                let is_enrich = args.iter().any(|a| {
                    a.strip_prefix("query=")
                        .map_or(false, |q| q.contains("issue(number"))
                });
                if is_enrich {
                    Ok(r#"{"data":{"repository":{"issue":{"projectItems":{"nodes":[
                    {"project":{"number":18,"status":{"options":[{"name":"Todo"},{"name":"Done"}]}},
                     "fieldValues":{"nodes":[
                        {"date":"2026-06-01","field":{"name":"Start date"}},
                        {"name":"Done","field":{"name":"Status"}}
                     ]}}]}}}}}"#
                        .to_string())
                } else if args.join(" ").contains("issue view") {
                    Ok(r#"{"number":123,"title":"t","url":"https://github.com/o/r/issues/123","state":"OPEN","assignees":[],"labels":[],"createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z","author":{"login":"a"}}"#.to_string())
                } else if args.iter().any(|a| a.contains("assignees")) {
                    // `gh api repos/o/r/assignees --paginate`
                    Ok("[]".to_string())
                } else {
                    Err(CommandRunnerError::new("unexpected"))
                }
            }
        }
        let state = AppState {
            github_source: GitHubSource::Cli,
            jira_source: JiraSource::Fixture(fixture_path("jira")),
            cache: Arc::new(ResponseCache::new(Duration::from_secs(30))),
            date_cache: Arc::new(ResponseCache::new(std::time::Duration::from_secs(600))),
            runner: Arc::new(Runner),
            github_repos: vec!["o/r".to_string()],
            jira_queries: vec!["order by updated desc".to_string()],
            jira_base_url: "https://quera.atlassian.net".to_string(),
            jira_people: Vec::new(),
            jira_jql: None,
            github_project: Some(GitHubProject {
                owner: "QuEraComputing".into(),
                number: 18,
                start_date_field: "Start date".into(),
                target_date_field: "Target date".into(),
                status_field: "Status".into(),
            }),
            jira_config: None,
        };
        let detail = super::fetch_work_item_detail(&state, "github:o/r#123").expect("detail");
        assert_eq!(detail.item.start_date, "2026-06-01");
        assert_eq!(detail.project_status.as_deref(), Some("Done"));
        assert_eq!(detail.status_options, vec!["Todo", "Done"]);
    }

    fn test_jira_config() -> crate::config::JiraConfig {
        crate::config::JiraConfig {
            email: "u@example.com".to_string(),
            token: "tok".to_string(),
            base_url: "https://example.atlassian.net".to_string(),
        }
    }

    // Answers the Jira REST curl calls: date PUT, transitions GET, transition POST.
    struct JiraWriteRunner {
        calls: Mutex<Vec<Vec<String>>>,
    }
    impl CommandRunner for JiraWriteRunner {
        fn run(&self, _program: &str, args: &[&str]) -> CommandResult<String> {
            let owned: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            self.calls.lock().unwrap().push(owned.clone());
            let is_transitions = owned.iter().any(|a| a.ends_with("/transitions"));
            let has_body = owned.iter().any(|a| a == "--data");
            if is_transitions && !has_body {
                Ok(r#"{"transitions":[{"id":"41","to":{"name":"Done"}}]}"#.to_string())
            } else {
                Ok(String::new())
            }
        }
    }

    fn jira_write_state(runner: Arc<dyn CommandRunner>, jira: Option<crate::config::JiraConfig>) -> AppState {
        AppState {
            github_source: GitHubSource::Fixture(fixture_path("github")),
            jira_source: JiraSource::Cli,
            cache: Arc::new(ResponseCache::new(Duration::from_secs(30))),
            date_cache: Arc::new(ResponseCache::new(std::time::Duration::from_secs(600))),
            runner,
            github_repos: Vec::new(),
            jira_queries: vec!["order by updated desc".to_string()],
            jira_base_url: "https://quera.atlassian.net".to_string(),
            jira_people: Vec::new(),
            jira_jql: None,
            github_project: None,
            jira_config: jira,
        }
    }

    #[test]
    fn update_jira_date_succeeds_and_invalidates_cache() {
        let runner = Arc::new(JiraWriteRunner {
            calls: Mutex::new(Vec::new()),
        });
        let state = jira_write_state(runner.clone(), Some(test_jira_config()));

        fetch_work_item_field(
            &state,
            &UpdateFieldRequest {
                id: "jira:SSW-1".to_string(),
                field: "target".to_string(),
                value: Some("2026-07-20".to_string()),
            },
        )
        .expect("jira date write should succeed");

        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "one PUT to the issue");
        assert!(calls[0].contains(&"PUT".to_string()));
        assert!(calls[0]
            .iter()
            .any(|a| a.contains("customfield_10023") && a.contains("2026-07-20")));
        assert_eq!(
            state.cache.get("work-items", Instant::now()),
            CacheOutcome::Miss
        );
    }

    #[test]
    fn update_jira_status_transitions_the_issue() {
        let runner = Arc::new(JiraWriteRunner {
            calls: Mutex::new(Vec::new()),
        });
        let state = jira_write_state(runner.clone(), Some(test_jira_config()));

        fetch_work_item_field(
            &state,
            &UpdateFieldRequest {
                id: "jira:SSW-1".to_string(),
                field: "status".to_string(),
                value: Some("Done".to_string()),
            },
        )
        .expect("jira status write should succeed");

        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "GET transitions then POST one");
        assert!(calls[1].contains(&"POST".to_string()));
        assert!(calls[1].iter().any(|a| a.contains("\"id\":\"41\"")));
    }

    #[test]
    fn update_jira_field_without_config_conflicts() {
        let runner = Arc::new(JiraWriteRunner {
            calls: Mutex::new(Vec::new()),
        });
        let state = jira_write_state(runner, None);

        let error = fetch_work_item_field(
            &state,
            &UpdateFieldRequest {
                id: "jira:SSW-1".to_string(),
                field: "target".to_string(),
                value: Some("2026-07-20".to_string()),
            },
        )
        .expect_err("missing jira config should conflict");
        assert_eq!(error.status, StatusCode::CONFLICT);
    }

    #[test]
    fn update_jira_status_requires_value() {
        let runner = Arc::new(JiraWriteRunner {
            calls: Mutex::new(Vec::new()),
        });
        let state = jira_write_state(runner, Some(test_jira_config()));

        let error = fetch_work_item_field(
            &state,
            &UpdateFieldRequest {
                id: "jira:SSW-1".to_string(),
                field: "status".to_string(),
                value: None,
            },
        )
        .expect_err("empty status should be rejected");
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn update_assignees_rejects_fixture_mode() {
        let app = router(app_state(fixture_path("github"), fixture_path("jira")));
        let response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/work-item-assignees")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"id":"github:openai/quasar#123","assignee_ids":["alice"]}"#,
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn update_assignees_rejects_unknown_id() {
        let app = router(app_state(fixture_path("github"), fixture_path("jira")));
        let response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/work-item-assignees")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"id":"nonsense","assignee_ids":[]}"#))
                    .expect("request should build"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn update_jira_assignee_rejects_multiple() {
        // The >1 check must fire before any CLI call, so an unconfigured runner
        // (which would error on use) proves no write was attempted.
        let runner = Arc::new(JiraWriteRunner {
            calls: Mutex::new(Vec::new()),
        });
        let state = jira_write_state(runner.clone(), Some(test_jira_config()));

        let error = set_work_item_assignees(
            &state,
            &UpdateAssigneesRequest {
                id: "jira:SSW-1".to_string(),
                assignee_ids: vec!["a".to_string(), "b".to_string()],
            },
        )
        .expect_err("multiple jira assignees should be rejected");
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert!(
            runner.calls.lock().unwrap().is_empty(),
            "no CLI call should be made before the count check"
        );
    }

    #[test]
    fn update_assignees_github_success_invalidates_work_items_cache() {
        // Routing runner answers both the CLI-mode work-items population
        // (`gh issue list`) and the assignee-write sequence
        // (`gh issue view --json assignees` then `gh issue edit`).
        struct RoutingRunner {
            issues: String,
        }
        impl CommandRunner for RoutingRunner {
            fn run(&self, _program: &str, args: &[&str]) -> CommandResult<String> {
                if args.contains(&"list") {
                    return Ok(self.issues.clone());
                }
                if args.contains(&"view") {
                    return Ok(r#"{"assignees":[]}"#.to_string());
                }
                // `gh issue edit`
                Ok(String::new())
            }
        }

        let state = AppState {
            github_source: GitHubSource::Cli,
            jira_source: JiraSource::Fixture(fixture_path("jira")),
            cache: Arc::new(ResponseCache::new(Duration::from_secs(30))),
            date_cache: Arc::new(ResponseCache::new(std::time::Duration::from_secs(600))),
            runner: Arc::new(RoutingRunner {
                issues: std::fs::read_to_string(fixture_path("github"))
                    .expect("fixture should read"),
            }),
            github_repos: vec!["openai/quasar".to_string()],
            jira_queries: vec!["order by updated desc".to_string()],
            jira_base_url: "https://quera.atlassian.net".to_string(),
            jira_people: Vec::new(),
            jira_jql: None,
            github_project: None,
            jira_config: None,
        };

        // Populate the cache (miss on first fetch).
        let populated = fetch_work_items(&state);
        assert_eq!(populated.cache_status, "miss");
        assert!(matches!(
            state.cache.get("work-items", Instant::now()),
            CacheOutcome::Hit(_)
        ));

        // A successful assignee write must invalidate the cache.
        let result = set_work_item_assignees(
            &state,
            &UpdateAssigneesRequest {
                id: "github:openai/quasar#123".to_string(),
                assignee_ids: vec!["alice".to_string()],
            },
        );
        assert!(
            result.is_ok(),
            "expected ok, got status {:?}",
            result.err().map(|error| error.status)
        );
        assert_eq!(
            state.cache.get("work-items", Instant::now()),
            CacheOutcome::Miss
        );
    }
}
