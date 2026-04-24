use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use sqlx::mysql::MySqlConnectOptions;
use thiserror::Error;

const DEFAULT_ADDR: &str = "127.0.0.1:8090";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub addr: String,
    pub bench_root: PathBuf,
    pub site_name: String,
    pub site_config: PathBuf,
    pub db_host: String,
    pub db_port: u16,
    pub db_name: String,
    pub db_user: String,
    pub db_password: String,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("{path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid ERP_DB_PORT: {0}")]
    InvalidDbPort(#[from] std::num::ParseIntError),
    #[error("site name is empty")]
    EmptySiteName,
    #[error("db_name is empty in {0}")]
    EmptyDbName(PathBuf),
    #[error("db_password is empty in {0}")]
    EmptyDbPassword(PathBuf),
    #[error("getwd: {0}")]
    Getwd(#[from] std::io::Error),
}

#[derive(Debug, Deserialize)]
struct CommonSiteConfig {
    #[serde(default)]
    default_site: String,
}

#[derive(Debug, Deserialize)]
struct SiteConfig {
    #[serde(default)]
    db_host: String,
    #[serde(default)]
    db_port: u16,
    #[serde(default)]
    db_name: String,
    #[serde(default)]
    db_password: String,
}

pub fn load_from_env() -> Result<Config, ConfigError> {
    let bench_root = match trimmed_env("ERP_BENCH_ROOT") {
        Some(value) => PathBuf::from(value),
        None => env::current_dir()?,
    };

    let site_name = match trimmed_env("ERP_SITE_NAME") {
        Some(value) => value,
        None => {
            let common_path = bench_root.join("sites").join("common_site_config.json");
            load_common_site_config(&common_path)?
                .default_site
                .trim()
                .to_string()
        }
    };
    if site_name.is_empty() {
        return Err(ConfigError::EmptySiteName);
    }

    let site_config = trimmed_env("ERP_SITE_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            bench_root
                .join("sites")
                .join(&site_name)
                .join("site_config.json")
        });

    let site_cfg = load_site_config(&site_config)?;
    let mut db_host = site_cfg.db_host.trim().to_string();
    if db_host.is_empty() {
        db_host = "127.0.0.1".to_string();
    }

    let mut db_port = if site_cfg.db_port == 0 {
        3306
    } else {
        site_cfg.db_port
    };
    if let Some(raw) = trimmed_env("ERP_DB_PORT") {
        db_port = raw.parse()?;
    }
    if let Some(host) = trimmed_env("ERP_DB_HOST") {
        db_host = host;
    }

    let db_name = site_cfg.db_name.trim().to_string();
    if db_name.is_empty() {
        return Err(ConfigError::EmptyDbName(site_config));
    }
    let db_password = site_cfg.db_password.trim().to_string();
    if db_password.is_empty() {
        return Err(ConfigError::EmptyDbPassword(site_config));
    }

    let db_user = trimmed_env("ERP_DB_USER").unwrap_or_else(|| db_name.clone());

    Ok(Config {
        addr: trimmed_env("ERP_READ_ADDR").unwrap_or_else(|| DEFAULT_ADDR.to_string()),
        bench_root,
        site_name,
        site_config,
        db_host,
        db_port,
        db_name,
        db_user,
        db_password,
    })
}

impl Config {
    pub fn connect_options(&self) -> MySqlConnectOptions {
        MySqlConnectOptions::new()
            .host(&self.db_host)
            .port(self.db_port)
            .username(&self.db_user)
            .password(&self.db_password)
            .database(&self.db_name)
            .charset("utf8mb4")
    }
}

fn load_common_site_config(path: &Path) -> Result<CommonSiteConfig, ConfigError> {
    load_json(path)
}

fn load_site_config(path: &Path) -> Result<SiteConfig, ConfigError> {
    load_json(path)
}

fn load_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, ConfigError> {
    let data = fs::read(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(&data).map_err(|source| ConfigError::Json {
        path: path.to_path_buf(),
        source,
    })
}

fn trimmed_env(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_from_env_reads_bench_site_config() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bench_root = temp.path();
        let common_path = bench_root.join("sites").join("common_site_config.json");
        let site_path = bench_root
            .join("sites")
            .join("erp.localhost")
            .join("site_config.json");

        fs::create_dir_all(site_path.parent().expect("site parent")).expect("mkdir site");
        fs::write(&common_path, r#"{"default_site":"erp.localhost"}"#).expect("write common");
        fs::write(&site_path, r#"{"db_name":"erpdb","db_password":"secret"}"#).expect("write site");

        unsafe {
            env::set_var("ERP_BENCH_ROOT", bench_root);
            env::remove_var("ERP_SITE_NAME");
            env::remove_var("ERP_SITE_CONFIG");
            env::remove_var("ERP_READ_ADDR");
            env::remove_var("ERP_DB_HOST");
            env::remove_var("ERP_DB_PORT");
            env::remove_var("ERP_DB_USER");
        }

        let cfg = load_from_env().expect("load config");

        assert_eq!(cfg.site_name, "erp.localhost");
        assert_eq!(cfg.db_host, "127.0.0.1");
        assert_eq!(cfg.db_port, 3306);
        assert_eq!(cfg.db_user, "erpdb");
    }
}
