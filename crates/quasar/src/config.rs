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
    pub jira_jql: String,
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
    let jira_jql = env
        .jira_jql
        .map(str::to_string)
        .or(file_config.jira_jql)
        .unwrap_or_else(|| "order by updated desc".to_string());

    Ok(RuntimeConfig {
        bind_addr,
        cache_ttl_secs,
        mode,
        github_repos,
        jira_jql,
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
                jira_jql: "project = TEAM order by updated desc".to_string(),
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
                jira_jql: "order by updated desc".to_string(),
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
