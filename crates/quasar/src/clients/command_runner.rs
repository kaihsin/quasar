use std::process::Command;

pub type CommandResult<T> = Result<T, CommandRunnerError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRunnerError {
    pub message: String,
}

impl CommandRunnerError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CommandRunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CommandRunnerError {}

pub trait CommandRunner: Send + Sync {
    fn run(&self, program: &str, args: &[&str]) -> CommandResult<String>;
}

#[derive(Debug, Default)]
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> CommandResult<String> {
        let output = Command::new(program)
            .args(args)
            .output()
            .map_err(|error| CommandRunnerError::new(error.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let message = if stderr.is_empty() {
                format!("{program} exited with status {}", output.status)
            } else {
                stderr
            };
            return Err(CommandRunnerError::new(message));
        }

        String::from_utf8(output.stdout).map_err(|error| CommandRunnerError::new(error.to_string()))
    }
}
