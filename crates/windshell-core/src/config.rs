use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Authentication {
    #[default]
    Auto,
    Password,
    PrivateKey,
}

impl Authentication {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "SSH 配置 / Agent",
            Self::Password => "密码",
            Self::PrivateKey => "粘贴私钥",
        }
    }
    pub fn next(self) -> Self {
        match self {
            Self::Auto => Self::Password,
            Self::Password => Self::PrivateKey,
            Self::PrivateKey => Self::Auto,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Host {
    pub name: String,
    /// A DNS name, IP address, or alias from ~/.ssh/config.
    pub address: String,
    #[serde(default)]
    pub user: String,
    /// None preserves the port from OpenSSH configuration.
    pub port: Option<u16>,
    #[serde(default)]
    pub identity: String,
    #[serde(default)]
    pub authentication: Authentication,
}

impl Host {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() || self.name.chars().any(char::is_control) {
            bail!("连接名称不能为空或包含控制字符");
        }
        if self.address.is_empty()
            || self.address.starts_with('-')
            || self
                .address
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || "@/\\[]".contains(c))
        {
            bail!("主机须为 SSH 别名、域名或 IP（IPv6 不加方括号）");
        }
        if self.user.starts_with('-')
            || self
                .user
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || "@: /\\".contains(c))
        {
            bail!("用户名格式无效");
        }
        if self.port == Some(0) {
            bail!("端口须在 1–65535 之间");
        }
        if self.identity.chars().any(char::is_control) {
            bail!("密钥路径不能包含控制字符");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostStore {
    #[serde(default)]
    pub hosts: Vec<Host>,
}

impl HostStore {
    pub fn load(path: &Path) -> Result<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e).context("读取连接配置失败"),
        };
        let store: Self = toml::from_str(&text).context("配置 TOML 无效，原文件未修改")?;
        store.validate()?;
        Ok(store)
    }

    pub fn validate(&self) -> Result<()> {
        let mut names = std::collections::HashSet::new();
        for host in &self.hosts {
            host.validate()?;
            if !names.insert(&host.name) {
                bail!("连接名称重复：{}", host.name);
            }
        }
        Ok(())
    }

    /// Validate first, then atomically replace the file in the same directory.
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent).context("创建配置目录失败")?;
        let mut temp = tempfile::NamedTempFile::new_in(parent).context("创建配置临时文件失败")?;
        // tempfile creates private files (0600 on Unix); secrets are never stored.
        temp.write_all(toml::to_string_pretty(self)?.as_bytes())?;
        temp.as_file().sync_all()?;
        temp.persist(path).context("保存连接配置失败")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_failed_save_preserves_original() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("hosts.toml");
        let mut store = HostStore {
            hosts: vec![Host {
                name: "生产 主机".into(),
                address: "prod".into(),
                ..Host::default()
            }],
        };
        store.save(&path)?;
        assert_eq!(HostStore::load(&path)?.hosts, store.hosts);
        store.hosts.push(store.hosts[0].clone());
        assert!(store.save(&path).is_err());
        assert_eq!(HostStore::load(&path)?.hosts.len(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(path)?.permissions().mode() & 0o777, 0o600);
        }
        Ok(())
    }

    #[test]
    fn corrupt_config_is_not_an_empty_store() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("hosts.toml");
        fs::write(&path, "not toml!")?;
        assert!(HostStore::load(&path).is_err());
        assert_eq!(fs::read_to_string(path)?, "not toml!");
        Ok(())
    }
}
