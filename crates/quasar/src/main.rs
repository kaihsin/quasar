use std::{env, error::Error, net::SocketAddr, path::Path};

use tokio::net::TcpListener;
use quasar::api::{self, AppState, GitHubSource, JiraSource};
use quasar::config::{self, EnvOverrides, RuntimeConfig, ServerMode};

fn resolve_startup_config(
    home_dir: Option<&Path>,
    env: EnvOverrides<'_>,
) -> Result<RuntimeConfig, config::ConfigError> {
    let config_path = config::default_config_path(home_dir)?;
    config::load_runtime_config(&config_path, env)
}

fn build_app_state(config: &RuntimeConfig) -> AppState {
    match config.mode {
        ServerMode::Cli => AppState::new(
            GitHubSource::Cli,
            JiraSource::Cli,
            config.cache_ttl_secs,
            config.github_repos.clone(),
            config.jira_queries.clone(),
            config.jira_base_url.clone(),
            config.jira_people.clone(),
            config.jira_jql.clone(),
            config.github_project.clone(),
            config.jira.clone(),
        ),
        ServerMode::Fixtures => AppState::new(
            GitHubSource::Fixture("crates/quasar/tests/fixtures/github/issues.json".into()),
            JiraSource::Fixture("crates/quasar/tests/fixtures/jira/issues.json".into()),
            config.cache_ttl_secs,
            config.github_repos.clone(),
            config.jira_queries.clone(),
            config.jira_base_url.clone(),
            config.jira_people.clone(),
            config.jira_jql.clone(),
            config.github_project.clone(),
            config.jira.clone(),
        ),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let home_dir = env::var_os("HOME");
    let config = resolve_startup_config(
        home_dir.as_deref().map(Path::new),
        EnvOverrides {
            bind_addr: env::var("QUASAR_BIND").ok().as_deref(),
            cache_ttl_secs: env::var("QUASAR_CACHE_TTL_SECS").ok().as_deref(),
            mode: env::var("QUASAR_MODE").ok().as_deref(),
            github_repo: env::var("QUASAR_GITHUB_REPO").ok().as_deref(),
            jira_jql: env::var("QUASAR_JIRA_JQL").ok().as_deref(),
        },
    )?;

    let app = api::router(build_app_state(&config));
    let bind_addr: SocketAddr = config.bind_addr.parse()?;
    let listener = TcpListener::bind(bind_addr).await?;

    println!("quasar listening on http://{}", config.bind_addr);

    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::resolve_startup_config;
    use quasar::config::{EnvOverrides, RuntimeConfig, ServerMode};

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
                "quasar-main-tests-{}-{unique}",
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

    fn write_user_config(home_dir: &Path, contents: &str) {
        let config_dir = home_dir.join(".config/quasar");
        fs::create_dir_all(&config_dir).expect("config directory should be created");
        fs::write(config_dir.join("config.toml"), contents).expect("config file should be written");
    }

    #[test]
    fn startup_resolver_reads_user_config_file() {
        let home_dir = TestDir::new();
        write_user_config(
            home_dir.path(),
            r#"
bind_addr = "0.0.0.0:4010"
cache_ttl_secs = 45
mode = "fixtures"
github_repos = ["openai/quasar", "rust-lang/rust"]
jira_jql = "project = TEAM order by updated desc"
"#,
        );

        let config = resolve_startup_config(Some(home_dir.path()), EnvOverrides::default())
            .expect("config should resolve");

        assert_eq!(
            config,
            RuntimeConfig {
                bind_addr: "0.0.0.0:4010".to_string(),
                cache_ttl_secs: 45,
                mode: ServerMode::Fixtures,
                github_repos: vec!["openai/quasar".to_string(), "rust-lang/rust".to_string()],
                jira_queries: vec!["project = TEAM order by updated desc".to_string()],
                jira_base_url: "https://quera.atlassian.net".to_string(),
                jira_people: Vec::new(),
                jira_jql: Some("project = TEAM order by updated desc".to_string()),
                github_project: None,
                jira: None,
            }
        );
    }

    #[test]
    fn startup_resolver_prefers_env_repo_override() {
        let home_dir = TestDir::new();
        write_user_config(
            home_dir.path(),
            r#"
github_repos = ["openai/quasar", "rust-lang/rust"]
"#,
        );

        let config = resolve_startup_config(
            Some(home_dir.path()),
            EnvOverrides {
                github_repo: Some("tokio-rs/tokio"),
                ..EnvOverrides::default()
            },
        )
        .expect("config should resolve");

        assert_eq!(config.github_repos, vec!["tokio-rs/tokio".to_string()]);
    }
}
