use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerMode {
    Cli,
    Fixtures,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub bind_addr: String,
    pub cache_ttl_secs: u64,
    pub mode: ServerMode,
    pub github_repos: Vec<String>,
    /// One composed JQL query per Jira project (or a single query in the
    /// no-`[jira_board]` escape-hatch case), fetched and streamed independently.
    pub jira_queries: Vec<String>,
    /// Jira site domain (e.g. `https://quera.atlassian.net`), used to build
    /// browse links. Defaults to the QuEra site.
    pub jira_base_url: String,
    pub github_project: Option<GitHubProject>,
    pub jira: Option<JiraConfig>,
}

/// Jira REST credentials used for writes (dates + status transitions). `acli`
/// cannot set custom fields, so writes go through the REST API via `curl` with
/// basic auth (`email:token`). Configured in `config.toml` as a `[jira]` table.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JiraConfig {
    pub email: String,
    pub token: String,
    #[serde(default = "default_jira_base_url")]
    pub base_url: String,
}

fn default_jira_base_url() -> String {
    "https://quera.atlassian.net".to_string()
}

/// Structured selection of the Jira project(s) to pull work items from,
/// configured in `config.toml` as a `[jira_board]` table. The listed project
/// keys compose a `project in (...)` clause; see `compose_jira_jql`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JiraBoard {
    #[serde(default)]
    pub projects: Vec<String>,
}

/// People whose related tickets (assigned to / created by) are pulled across
/// all projects, configured as a `[jira_people]` table. Users are emails.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JiraPeople {
    #[serde(default)]
    pub users: Vec<String>,
}

/// GitHub Projects v2 board used to enrich issues with planning dates.
/// Configured in `config.toml` as a `[github_project]` table.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitHubProject {
    pub owner: String,
    pub number: u64,
    #[serde(default = "default_start_date_field")]
    pub start_date_field: String,
    #[serde(default = "default_target_date_field")]
    pub target_date_field: String,
    #[serde(default = "default_status_field")]
    pub status_field: String,
}

fn default_start_date_field() -> String {
    "Start date".to_string()
}

fn default_status_field() -> String {
    "Status".to_string()
}

fn default_target_date_field() -> String {
    "Target date".to_string()
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EnvOverrides<'a> {
    pub bind_addr: Option<&'a str>,
    pub cache_ttl_secs: Option<&'a str>,
    pub mode: Option<&'a str>,
    pub github_repo: Option<&'a str>,
    pub jira_jql: Option<&'a str>,
}

#[derive(Debug)]
pub enum ConfigError {
    MissingHomeDir,
    Io(std::io::Error),
    ParseToml(toml::de::Error),
    InvalidCacheTtlEnv(String),
    InvalidModeEnv(String),
    InvalidGitHubRepo(String),
    InvalidJiraProject(String),
    InvalidJiraUser(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingHomeDir => write!(f, "HOME is not set"),
            Self::Io(error) => write!(f, "{error}"),
            Self::ParseToml(error) => write!(f, "{error}"),
            Self::InvalidCacheTtlEnv(value) => {
                write!(f, "invalid QUASAR_CACHE_TTL_SECS value: {value}")
            }
            Self::InvalidModeEnv(value) => {
                write!(f, "invalid QUASAR_MODE value: {value}")
            }
            Self::InvalidGitHubRepo(repo) => {
                write!(
                    f,
                    "invalid GitHub repo slug: {repo} (expected owner/repo with no whitespace)"
                )
            }
            Self::InvalidJiraProject(key) => {
                write!(
                    f,
                    "invalid Jira project key: {key} (must be non-empty with no whitespace)"
                )
            }
            Self::InvalidJiraUser(user) => {
                write!(
                    f,
                    "invalid Jira user: {user} (must be a non-empty email with no whitespace)"
                )
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<toml::de::Error> for ConfigError {
    fn from(value: toml::de::Error) -> Self {
        Self::ParseToml(value)
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    bind_addr: Option<String>,
    cache_ttl_secs: Option<u64>,
    mode: Option<ServerMode>,
    github_repos: Option<Vec<String>>,
    jira_jql: Option<String>,
    jira_board: Option<JiraBoard>,
    jira_base_url: Option<String>,
    jira_people: Option<JiraPeople>,
    github_project: Option<GitHubProject>,
    jira: Option<JiraConfig>,
}

pub fn default_config_path(home_dir: Option<&Path>) -> Result<PathBuf, ConfigError> {
    let home_dir = home_dir.ok_or(ConfigError::MissingHomeDir)?;
    Ok(home_dir.join(".config/quasar/config.toml"))
}

pub fn load_runtime_config(
    config_path: &Path,
    env: EnvOverrides<'_>,
) -> Result<RuntimeConfig, ConfigError> {
    let file_config = read_file_config(config_path)?;
    let bind_addr = env
        .bind_addr
        .map(str::to_string)
        .or(file_config.bind_addr)
        .unwrap_or_else(|| "127.0.0.1:3000".to_string());
    let cache_ttl_secs = env
        .cache_ttl_secs
        .map(parse_env_cache_ttl_secs)
        .transpose()?
        .or(file_config.cache_ttl_secs)
        .unwrap_or(30);
    let mode = env
        .mode
        .map(parse_env_mode)
        .transpose()?
        .or(file_config.mode)
        .unwrap_or(ServerMode::Cli);
    let github_repos = match env.github_repo {
        Some(repo) => vec![repo.to_string()],
        None => file_config.github_repos.unwrap_or_default(),
    };
    validate_github_repos(&github_repos)?;
    let jira_projects = file_config
        .jira_board
        .map(|board| board.projects)
        .unwrap_or_default();
    validate_jira_projects(&jira_projects)?;
    let jira_users = file_config
        .jira_people
        .map(|people| people.users)
        .unwrap_or_default();
    validate_jira_users(&jira_users)?;
    let jira_base_url = file_config
        .jira_base_url
        .unwrap_or_else(default_jira_base_url);
    let raw_jira_jql = env.jira_jql.map(str::to_string).or(file_config.jira_jql);
    let jira_queries = compose_jira_queries(&jira_projects, &jira_users, raw_jira_jql.as_deref());

    Ok(RuntimeConfig {
        bind_addr,
        cache_ttl_secs,
        mode,
        github_repos,
        jira_queries,
        jira_base_url,
        github_project: file_config.github_project,
        jira: file_config.jira,
    })
}

fn read_file_config(config_path: &Path) -> Result<FileConfig, ConfigError> {
    match std::fs::read_to_string(config_path) {
        Ok(contents) => Ok(toml::from_str(&contents)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(FileConfig::default()),
        Err(error) => Err(ConfigError::Io(error)),
    }
}

fn parse_env_cache_ttl_secs(value: &str) -> Result<u64, ConfigError> {
    value
        .parse::<u64>()
        .map_err(|_| ConfigError::InvalidCacheTtlEnv(value.to_string()))
}

fn parse_env_mode(value: &str) -> Result<ServerMode, ConfigError> {
    match value {
        "cli" => Ok(ServerMode::Cli),
        "fixtures" => Ok(ServerMode::Fixtures),
        _ => Err(ConfigError::InvalidModeEnv(value.to_string())),
    }
}

/// Compose the final Jira JQL from the structured board projects and an optional
/// raw JQL escape hatch, producing **one query per project** so the API can
/// fetch and stream each project independently (mirroring the per-repo GitHub
/// fan-out). Each project's `project = KEY` clause is AND'd with the raw JQL
/// filter (when set), and a single trailing `ORDER BY` is appended — the raw
/// JQL's own ordering when it has one, else a default. With no projects the raw
/// JQL (or a bare default) is the sole query, verbatim, preserving the plain-JQL
/// configuration path.
fn compose_jira_queries(
    projects: &[String],
    users: &[String],
    raw_jql: Option<&str>,
) -> Vec<String> {
    const DEFAULT_ORDER: &str = "ORDER BY updated DESC";
    let raw = raw_jql.map(str::trim).filter(|jql| !jql.is_empty());

    // Escape hatch preserved: with no projects AND no users, the raw JQL (or a
    // bare default) is the sole query, verbatim.
    if projects.is_empty() && users.is_empty() {
        return vec![raw.unwrap_or(DEFAULT_ORDER).to_string()];
    }

    let (raw_where, raw_order) = match raw {
        Some(raw) => split_order_by(raw),
        None => ("", None),
    };
    let order = raw_order.unwrap_or(DEFAULT_ORDER);

    let mut queries: Vec<String> = projects
        .iter()
        .map(|key| {
            if raw_where.is_empty() {
                format!("project = {key} {order}")
            } else {
                format!("(project = {key}) AND ({raw_where}) {order}")
            }
        })
        .collect();

    if !users.is_empty() {
        let list = users
            .iter()
            .map(|user| format!("\"{user}\""))
            .collect::<Vec<_>>()
            .join(",");
        let clause = format!("(assignee in ({list}) OR reporter in ({list}))");
        let where_part = if raw_where.is_empty() {
            clause
        } else {
            format!("({clause}) AND ({raw_where})")
        };
        queries.push(format!("{where_part} {order}"));
    }

    queries
}

/// Split a JQL string into its where-clause and a trailing `ORDER BY ...` clause
/// (matched case-insensitively on the last occurrence), if present. Both parts
/// are trimmed. `ORDER BY` is ASCII, so lowercasing preserves byte offsets.
fn split_order_by(jql: &str) -> (&str, Option<&str>) {
    match jql.to_ascii_lowercase().rfind("order by") {
        Some(idx) => (jql[..idx].trim(), Some(jql[idx..].trim())),
        None => (jql.trim(), None),
    }
}

fn validate_jira_projects(projects: &[String]) -> Result<(), ConfigError> {
    for key in projects {
        if key.is_empty() || key.chars().any(char::is_whitespace) {
            return Err(ConfigError::InvalidJiraProject(key.clone()));
        }
    }
    Ok(())
}

fn validate_jira_users(users: &[String]) -> Result<(), ConfigError> {
    for user in users {
        if user.is_empty() || user.chars().any(char::is_whitespace) || user.contains('"') {
            return Err(ConfigError::InvalidJiraUser(user.clone()));
        }
    }
    Ok(())
}

fn validate_github_repos(repos: &[String]) -> Result<(), ConfigError> {
    for repo in repos {
        if !is_valid_repo_slug(repo) {
            return Err(ConfigError::InvalidGitHubRepo(repo.clone()));
        }
    }
    Ok(())
}

fn is_valid_repo_slug(repo: &str) -> bool {
    if repo.trim() != repo || repo.chars().any(char::is_whitespace) {
        return false;
    }

    let mut parts = repo.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();

    !owner.is_empty() && !name.is_empty() && parts.next().is_none()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        default_config_path, load_runtime_config, ConfigError, EnvOverrides, RuntimeConfig,
        ServerMode,
    };

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "quasar-config-tests-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("temp test dir should be created");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn write_config(home_dir: &Path, contents: &str) -> PathBuf {
        let config_path = default_config_path(Some(home_dir)).expect("config path should resolve");
        let parent = config_path
            .parent()
            .expect("config path should have parent");
        fs::create_dir_all(parent).expect("config parent directory should be created");
        fs::write(&config_path, contents).expect("config file should be written");
        config_path
    }

    #[test]
    fn loads_multiple_github_repos_from_toml() {
        let home_dir = TestDir::new();
        let config_path = write_config(
            home_dir.path(),
            r#"
bind_addr = "0.0.0.0:4000"
cache_ttl_secs = 90
mode = "fixtures"
github_repos = ["openai/quasar", "rust-lang/rust"]
jira_jql = "project = TEAM order by updated desc"
"#,
        );

        let config =
            load_runtime_config(&config_path, EnvOverrides::default()).expect("config should load");

        assert_eq!(
            config,
            RuntimeConfig {
                bind_addr: "0.0.0.0:4000".to_string(),
                cache_ttl_secs: 90,
                mode: ServerMode::Fixtures,
                github_repos: vec!["openai/quasar".to_string(), "rust-lang/rust".to_string()],
                jira_queries: vec!["project = TEAM order by updated desc".to_string()],
                jira_base_url: "https://quera.atlassian.net".to_string(),
                github_project: None,
                jira: None,
            }
        );
    }

    #[test]
    fn loads_jira_config_from_toml() {
        let home_dir = TestDir::new();
        let config_path = write_config(
            home_dir.path(),
            r#"
[jira]
email = "khwu@quera.com"
token = "secret-token"
"#,
        );

        let config =
            load_runtime_config(&config_path, EnvOverrides::default()).expect("config should load");

        assert_eq!(
            config.jira,
            Some(super::JiraConfig {
                email: "khwu@quera.com".to_string(),
                token: "secret-token".to_string(),
                base_url: "https://quera.atlassian.net".to_string(),
            })
        );
    }

    #[test]
    fn jira_base_url_is_overridable() {
        let home_dir = TestDir::new();
        let config_path = write_config(
            home_dir.path(),
            r#"
[jira]
email = "u@example.com"
token = "t"
base_url = "https://example.atlassian.net"
"#,
        );

        let config =
            load_runtime_config(&config_path, EnvOverrides::default()).expect("config should load");
        assert_eq!(
            config.jira.expect("jira present").base_url,
            "https://example.atlassian.net"
        );
    }

    #[test]
    fn compose_jira_queries_appends_person_query_for_users() {
        let queries = super::compose_jira_queries(
            &["SSW".to_string()],
            &["a@x".to_string(), "b@x".to_string()],
            None,
        );
        assert_eq!(
            queries,
            vec![
                "project = SSW ORDER BY updated DESC".to_string(),
                "(assignee in (\"a@x\",\"b@x\") OR reporter in (\"a@x\",\"b@x\")) ORDER BY updated DESC"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn compose_jira_queries_person_query_ands_raw_jql_and_honors_order() {
        let queries = super::compose_jira_queries(
            &[],
            &["a@x".to_string()],
            Some("statusCategory != Done ORDER BY created DESC"),
        );
        assert_eq!(
            queries,
            vec![
                "((assignee in (\"a@x\") OR reporter in (\"a@x\"))) AND (statusCategory != Done) ORDER BY created DESC"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn compose_jira_queries_no_projects_no_users_is_unchanged() {
        assert_eq!(
            super::compose_jira_queries(&[], &[], Some("project = X order by created desc")),
            vec!["project = X order by created desc".to_string()]
        );
        assert_eq!(
            super::compose_jira_queries(&[], &[], None),
            vec!["ORDER BY updated DESC".to_string()]
        );
    }

    #[test]
    fn loads_jira_people_and_base_url_from_toml() {
        let home_dir = TestDir::new();
        let config_path = write_config(
            home_dir.path(),
            r#"
jira_base_url = "https://acme.atlassian.net"

[jira_board]
projects = ["SSW"]

[jira_people]
users = ["a@x", "b@x"]
"#,
        );
        let config =
            load_runtime_config(&config_path, EnvOverrides::default()).expect("config should load");
        assert_eq!(config.jira_base_url, "https://acme.atlassian.net");
        assert_eq!(
            config.jira_queries,
            vec![
                "project = SSW ORDER BY updated DESC".to_string(),
                "(assignee in (\"a@x\",\"b@x\") OR reporter in (\"a@x\",\"b@x\")) ORDER BY updated DESC"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn jira_people_rejects_user_with_whitespace() {
        let home_dir = TestDir::new();
        let config_path = write_config(home_dir.path(), "[jira_people]\nusers = [\"a b@x\"]\n");
        let error = load_runtime_config(&config_path, EnvOverrides::default())
            .expect_err("whitespace user should be rejected");
        assert!(matches!(error, ConfigError::InvalidJiraUser(_)));
    }

    #[test]
    fn jira_people_rejects_user_with_quote() {
        let home_dir = TestDir::new();
        let config_path = write_config(home_dir.path(), "[jira_people]\nusers = [\"a\\\"@x\"]\n");
        let error = load_runtime_config(&config_path, EnvOverrides::default())
            .expect_err("user with a double-quote should be rejected");
        assert!(matches!(error, ConfigError::InvalidJiraUser(_)));
    }

    #[test]
    fn jira_base_url_defaults_when_absent() {
        let home_dir = TestDir::new();
        let config_path = write_config(home_dir.path(), "github_repos = []\n");
        let config =
            load_runtime_config(&config_path, EnvOverrides::default()).expect("config should load");
        assert_eq!(config.jira_base_url, "https://quera.atlassian.net");
    }

    #[test]
    fn compose_jira_queries_board_only_adds_default_order() {
        let queries = super::compose_jira_queries(&["SSW".to_string()], &[], None);
        assert_eq!(queries, vec!["project = SSW ORDER BY updated DESC"]);
    }

    #[test]
    fn compose_jira_queries_one_query_per_project() {
        let queries =
            super::compose_jira_queries(&["SSW".to_string(), "ENG".to_string()], &[], None);
        assert_eq!(
            queries,
            vec![
                "project = SSW ORDER BY updated DESC",
                "project = ENG ORDER BY updated DESC",
            ]
        );
    }

    #[test]
    fn compose_jira_queries_ands_each_project_with_extra_jql() {
        let queries = super::compose_jira_queries(
            &["SSW".to_string(), "ENG".to_string()],
            &[],
            Some("statusCategory != Done"),
        );
        assert_eq!(
            queries,
            vec![
                "(project = SSW) AND (statusCategory != Done) ORDER BY updated DESC",
                "(project = ENG) AND (statusCategory != Done) ORDER BY updated DESC",
            ]
        );
    }

    #[test]
    fn compose_jira_queries_honors_extra_jql_order_by() {
        let queries = super::compose_jira_queries(
            &["SSW".to_string()],
            &[],
            Some("statusCategory != Done ORDER BY created DESC"),
        );
        assert_eq!(
            queries,
            vec!["(project = SSW) AND (statusCategory != Done) ORDER BY created DESC"]
        );
    }

    #[test]
    fn compose_jira_queries_raw_only_passes_through_verbatim() {
        let queries =
            super::compose_jira_queries(&[], &[], Some("project = SSW order by created desc"));
        assert_eq!(queries, vec!["project = SSW order by created desc"]);
    }

    #[test]
    fn compose_jira_queries_neither_defaults_to_order_by() {
        let queries = super::compose_jira_queries(&[], &[], None);
        assert_eq!(queries, vec!["ORDER BY updated DESC"]);
    }

    #[test]
    fn loads_jira_board_and_composes_per_project_queries() {
        let home_dir = TestDir::new();
        let config_path = write_config(
            home_dir.path(),
            r#"
[jira_board]
projects = ["SSW", "ENG"]
"#,
        );

        let config =
            load_runtime_config(&config_path, EnvOverrides::default()).expect("config should load");
        assert_eq!(
            config.jira_queries,
            vec![
                "project = SSW ORDER BY updated DESC",
                "project = ENG ORDER BY updated DESC",
            ]
        );
    }

    #[test]
    fn jira_board_ands_each_query_with_raw_jql() {
        let home_dir = TestDir::new();
        let config_path = write_config(
            home_dir.path(),
            r#"
jira_jql = "statusCategory != Done"

[jira_board]
projects = ["SSW"]
"#,
        );

        let config =
            load_runtime_config(&config_path, EnvOverrides::default()).expect("config should load");
        assert_eq!(
            config.jira_queries,
            vec!["(project = SSW) AND (statusCategory != Done) ORDER BY updated DESC"]
        );
    }

    #[test]
    fn jira_board_rejects_project_key_with_whitespace() {
        let home_dir = TestDir::new();
        let config_path = write_config(
            home_dir.path(),
            r#"
[jira_board]
projects = ["S S W"]
"#,
        );

        let error = load_runtime_config(&config_path, EnvOverrides::default())
            .expect_err("whitespace project key should be rejected");
        assert!(matches!(error, ConfigError::InvalidJiraProject(_)));
    }

    #[test]
    fn loads_github_project_from_toml() {
        let home_dir = TestDir::new();
        let config_path = write_config(
            home_dir.path(),
            r#"
github_repos = ["openai/quasar"]

[github_project]
owner = "QuEraComputing"
number = 18
"#,
        );

        let config =
            load_runtime_config(&config_path, EnvOverrides::default()).expect("config should load");

        assert_eq!(
            config.github_project,
            Some(super::GitHubProject {
                owner: "QuEraComputing".to_string(),
                number: 18,
                start_date_field: "Start date".to_string(),
                target_date_field: "Target date".to_string(),
                status_field: "Status".to_string(),
            })
        );
    }

    #[test]
    fn github_project_date_field_names_are_overridable() {
        let home_dir = TestDir::new();
        let config_path = write_config(
            home_dir.path(),
            r#"
[github_project]
owner = "acme"
number = 3
start_date_field = "Kickoff"
target_date_field = "Deadline"
"#,
        );

        let config =
            load_runtime_config(&config_path, EnvOverrides::default()).expect("config should load");
        let project = config.github_project.expect("project should be present");
        assert_eq!(project.start_date_field, "Kickoff");
        assert_eq!(project.target_date_field, "Deadline");
    }

    #[test]
    fn github_project_status_field_defaults_to_status() {
        let toml = r#"
owner = "QuEraComputing"
number = 18
"#;
        let project: super::GitHubProject = toml::from_str(toml).expect("should parse");
        assert_eq!(project.status_field, "Status");
    }

    #[test]
    fn env_repo_override_replaces_file_repos() {
        let home_dir = TestDir::new();
        let config_path = write_config(
            home_dir.path(),
            r#"
github_repos = ["openai/quasar", "rust-lang/rust"]
"#,
        );

        let config = load_runtime_config(
            &config_path,
            EnvOverrides {
                github_repo: Some("tokio-rs/tokio"),
                ..EnvOverrides::default()
            },
        )
        .expect("config should load");

        assert_eq!(config.github_repos, vec!["tokio-rs/tokio".to_string()]);
    }

    #[test]
    fn missing_file_uses_defaults() {
        let home_dir = TestDir::new();
        let config_path =
            default_config_path(Some(home_dir.path())).expect("config path should resolve");

        let config = load_runtime_config(&config_path, EnvOverrides::default())
            .expect("missing config file should fall back to defaults");

        assert_eq!(
            config,
            RuntimeConfig {
                bind_addr: "127.0.0.1:3000".to_string(),
                cache_ttl_secs: 30,
                mode: ServerMode::Cli,
                github_repos: Vec::new(),
                jira_queries: vec!["ORDER BY updated DESC".to_string()],
                jira_base_url: "https://quera.atlassian.net".to_string(),
                github_project: None,
                jira: None,
            }
        );
    }

    #[test]
    fn invalid_repo_slug_is_rejected() {
        let home_dir = TestDir::new();
        let config_path = write_config(
            home_dir.path(),
            r#"
github_repos = ["openai"]
"#,
        );

        let error = load_runtime_config(&config_path, EnvOverrides::default())
            .expect_err("invalid repo slug should be rejected");

        assert!(matches!(error, ConfigError::InvalidGitHubRepo(repo) if repo == "openai"));
    }

    #[test]
    fn invalid_cache_ttl_env_value_is_rejected() {
        let home_dir = TestDir::new();
        let config_path =
            default_config_path(Some(home_dir.path())).expect("config path should resolve");

        let error = load_runtime_config(
            &config_path,
            EnvOverrides {
                cache_ttl_secs: Some("abc"),
                ..EnvOverrides::default()
            },
        )
        .expect_err("invalid cache ttl env value should be rejected");

        assert!(matches!(error, ConfigError::InvalidCacheTtlEnv(value) if value == "abc"));
    }

    #[test]
    fn invalid_mode_env_value_is_rejected() {
        let home_dir = TestDir::new();
        let config_path =
            default_config_path(Some(home_dir.path())).expect("config path should resolve");

        let error = load_runtime_config(
            &config_path,
            EnvOverrides {
                mode: Some("staging"),
                ..EnvOverrides::default()
            },
        )
        .expect_err("invalid mode env value should be rejected");

        assert!(matches!(error, ConfigError::InvalidModeEnv(value) if value == "staging"));
    }

    #[test]
    fn repo_slug_with_trailing_whitespace_is_rejected() {
        let home_dir = TestDir::new();
        let config_path = write_config(
            home_dir.path(),
            r#"
github_repos = ["openai/quasar "]
"#,
        );

        let error = load_runtime_config(&config_path, EnvOverrides::default())
            .expect_err("repo slug with trailing whitespace should be rejected");

        assert!(matches!(error, ConfigError::InvalidGitHubRepo(repo) if repo == "openai/quasar "));
    }

    #[test]
    fn repo_slug_with_internal_whitespace_is_rejected() {
        let home_dir = TestDir::new();
        let config_path = write_config(
            home_dir.path(),
            r#"
github_repos = ["open ai/quasar"]
"#,
        );

        let error = load_runtime_config(&config_path, EnvOverrides::default())
            .expect_err("repo slug with internal whitespace should be rejected");

        assert!(matches!(error, ConfigError::InvalidGitHubRepo(repo) if repo == "open ai/quasar"));
    }
}
