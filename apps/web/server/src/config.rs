use crate::error::WebBootstrapError;
use ora_logging::{FileLoggingConfig, LogLevel, LogOutput, LoggingConfig, RotationPolicy};
use std::env;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroUsize;

const DEFAULT_HOST: &str = "0.0.0.0";
const DEFAULT_PORT: u16 = 32578;
const DEFAULT_LOG_LEVEL: &str = "info";
const DEFAULT_LOG_MODE: &str = "stdout";
const DEFAULT_LOG_PATH: &str = "./ora.log";
const DEFAULT_LOG_MAX_DAYS: &str = "3";

/// Groups the runtime configuration required to bootstrap the web server process.
pub struct RuntimeConfig {
    server: ServerConfig,
    logging: LoggingConfig,
}

impl RuntimeConfig {
    /// Loads the runtime configuration from the environment-backed server contract.
    pub fn from_env() -> Result<Self, WebBootstrapError> {
        Self::from_reader(|key| env::var(key).ok())
    }

    /// Returns the server bind configuration used by the runtime.
    pub fn server(&self) -> &ServerConfig {
        &self.server
    }

    /// Returns the shared logging configuration used during process bootstrap.
    pub fn logging(&self) -> &LoggingConfig {
        &self.logging
    }

    /// Loads the runtime configuration from a caller-provided variable reader for testability.
    fn from_reader(
        mut read_variable: impl FnMut(&str) -> Option<String>,
    ) -> Result<Self, WebBootstrapError> {
        Ok(Self {
            server: ServerConfig::from_reader(&mut read_variable)?,
            logging: read_logging_config(&mut read_variable)?,
        })
    }
}

/// Describes the host and port that the HTTP server binds to.
pub struct ServerConfig {
    host: IpAddr,
    port: u16,
}

impl ServerConfig {
    /// Returns the bind host used by the HTTP listener.
    pub fn host(&self) -> IpAddr {
        self.host
    }

    /// Returns the bind port used by the HTTP listener.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Combines the configured host and port into the socket address consumed by Tokio.
    pub fn socket_address(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }

    /// Loads the bind host and port from a caller-provided variable reader for testability.
    fn from_reader(
        mut read_variable: impl FnMut(&str) -> Option<String>,
    ) -> Result<Self, WebBootstrapError> {
        let raw_host = read_variable("ORA_HOST").unwrap_or_else(|| DEFAULT_HOST.to_string());
        let host = raw_host
            .parse::<IpAddr>()
            .map_err(|source| WebBootstrapError::InvalidHost {
                value: raw_host.clone(),
                source,
            })?;
        let raw_port = read_variable("ORA_PORT").unwrap_or_else(|| DEFAULT_PORT.to_string());
        let port = raw_port
            .parse::<u16>()
            .map_err(|source| WebBootstrapError::InvalidPort {
                value: raw_port.clone(),
                source,
            })?;

        Ok(Self { host, port })
    }
}

/// Loads the logging configuration from the environment contract defined for the web server bootstrap.
fn read_logging_config(
    mut read_variable: impl FnMut(&str) -> Option<String>,
) -> Result<LoggingConfig, WebBootstrapError> {
    let level = match read_variable("ORA_LOG_LEVEL")
        .unwrap_or_else(|| DEFAULT_LOG_LEVEL.to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "debug" => LogLevel::Debug,
        "info" => LogLevel::Info,
        "warn" => LogLevel::Warn,
        "error" => LogLevel::Error,
        value => {
            return Err(WebBootstrapError::InvalidLogLevel {
                value: value.to_string(),
            });
        }
    };
    let file_config = FileLoggingConfig::new(
        read_variable("ORA_LOG_PATH").unwrap_or_else(|| DEFAULT_LOG_PATH.to_string()),
        RotationPolicy::Daily,
        read_log_max_days(&mut read_variable)?,
    );
    let output = match read_variable("ORA_LOG_MODE")
        .unwrap_or_else(|| DEFAULT_LOG_MODE.to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "stdout" => LogOutput::Stdout,
        "file" => LogOutput::File(file_config),
        "stdout_and_file" => LogOutput::StdoutAndFile(file_config),
        value => {
            return Err(WebBootstrapError::InvalidLogMode {
                value: value.to_string(),
            });
        }
    };

    Ok(LoggingConfig::new(level, output))
}

/// Parses the configured retention window and rejects zero-day values explicitly.
fn read_log_max_days(
    mut read_variable: impl FnMut(&str) -> Option<String>,
) -> Result<NonZeroUsize, WebBootstrapError> {
    let raw_value =
        read_variable("ORA_LOG_MAX_DAYS").unwrap_or_else(|| DEFAULT_LOG_MAX_DAYS.to_string());
    let parsed_value =
        raw_value
            .parse::<usize>()
            .map_err(|source| WebBootstrapError::InvalidLogMaxDays {
                value: raw_value.clone(),
                source,
            })?;

    NonZeroUsize::new(parsed_value).ok_or(WebBootstrapError::InvalidLogMaxDaysZero)
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_HOST, DEFAULT_PORT, RuntimeConfig, ServerConfig};
    use crate::error::WebBootstrapError;
    use pretty_assertions::assert_eq;

    /// Verifies the server configuration defaults to the documented host and port.
    #[test]
    fn loads_default_server_configuration() {
        let config = ServerConfig::from_reader(|_| None).unwrap_or_else(|error| {
            panic!("expected default server configuration to load: {error}");
        });

        assert_eq!(config.host().to_string(), DEFAULT_HOST.to_string());
        assert_eq!(config.port(), DEFAULT_PORT);
    }

    /// Verifies invalid port values fail with a typed bootstrap error.
    #[test]
    fn rejects_invalid_port_configuration() {
        let error = match ServerConfig::from_reader(|key| match key {
            "ORA_HOST" => Some(DEFAULT_HOST.to_string()),
            "ORA_PORT" => Some("not-a-port".to_string()),
            _ => None,
        }) {
            Ok(_) => panic!("expected invalid port configuration to fail"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            WebBootstrapError::InvalidPort { value, .. } if value == "not-a-port"
        ));
    }

    /// Verifies the runtime configuration loads both the server and logging contracts together.
    #[test]
    fn loads_runtime_configuration() {
        let config = RuntimeConfig::from_reader(|_| None).unwrap_or_else(|error| {
            panic!("expected runtime configuration to load: {error}");
        });

        assert_eq!(
            config.server().socket_address().to_string(),
            format!("{DEFAULT_HOST}:{DEFAULT_PORT}")
        );
    }
}
