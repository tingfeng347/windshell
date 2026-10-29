use crate::{Authentication, Host};
use anyhow::{Result, bail};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug)]
pub enum Operation {
    Shell,
    /// Single file, exact remote path; no shell expansion or wildcard matching.
    Upload {
        local: PathBuf,
        remote: String,
    },
    Download {
        remote: String,
        local: PathBuf,
    },
}

/// Inherit the real terminal for authentication, progress, and full-screen apps.
/// No local shell, no password persistence, no bypass of known_hosts checking.
pub fn command(host: &Host, operation: &Operation, ssh_config: Option<&Path>) -> Result<Command> {
    host.validate()?;
    let shell = matches!(operation, Operation::Shell);
    let mut cmd = Command::new(if shell { "ssh" } else { "scp" });
    if let Some(path) = ssh_config {
        cmd.arg("-F").arg(path);
    }
    if shell {
        cmd.arg("-tt");
    }
    cmd.args([
        "-o",
        "ServerAliveInterval=30",
        "-o",
        "ServerAliveCountMax=3",
    ]);
    if let Some(port) = host.port {
        cmd.arg(if shell { "-p" } else { "-P" })
            .arg(port.to_string());
    }
    match host.authentication {
        Authentication::Auto => {}
        Authentication::Password => {
            cmd.args([
                "-o",
                "PreferredAuthentications=password,keyboard-interactive",
                "-o",
                "PubkeyAuthentication=no",
                "-o",
                "BatchMode=no",
            ]);
        }
        Authentication::PrivateKey => {
            cmd.args([
                "-o",
                "PreferredAuthentications=publickey",
                "-o",
                "IdentitiesOnly=yes",
                "-o",
                "IdentityAgent=none",
                "-o",
                "BatchMode=no",
            ]);
        }
    }
    if host.authentication == Authentication::Auto && !host.identity.is_empty() {
        cmd.arg("-i").arg(&host.identity);
    }
    if shell {
        if !host.user.is_empty() {
            cmd.arg("-l").arg(&host.user);
        }
        cmd.arg("--").arg(&host.address);
    } else {
        let (local, remote, upload) = match operation {
            Operation::Upload { local, remote } => (local, remote, true),
            Operation::Download { remote, local } => (local, remote, false),
            Operation::Shell => unreachable!(),
        };
        if remote.is_empty()
            || remote.starts_with('~')
            || remote
                .chars()
                .any(|c| c.is_control() || "*?[]\\".contains(c))
        {
            bail!("远端路径不能为空，且暂不支持 ~ 前缀、通配符、反斜杠或控制字符");
        }
        if upload && !local.is_file() {
            bail!("上传源必须是存在的普通文件");
        }
        // Absolute local paths prevent ':' being interpreted as a remote spec.
        let local = std::path::absolute(local)?;
        let address = if host.address.contains(':') {
            format!("[{}]", host.address)
        } else {
            host.address.clone()
        };
        let user = if host.user.is_empty() {
            String::new()
        } else {
            format!("{}@", host.user)
        };
        let remote = format!("{user}{address}:{remote}");
        cmd.arg("--");
        if upload {
            cmd.arg(local).arg(remote);
        } else {
            cmd.arg(remote).arg(local);
        }
    }
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn host() -> Host {
        Host {
            name: "test".into(),
            address: "::1".into(),
            user: "alice".into(),
            port: Some(2222),
            identity: String::new(),
            authentication: Authentication::Auto,
        }
    }
    #[test]
    fn ipv6_download_and_spaces_are_single_arguments() -> Result<()> {
        let cmd = command(
            &host(),
            &Operation::Download {
                remote: "/tmp/a b;echo bad".into(),
                local: "a:b".into(),
            },
            None,
        )?;
        let args: Vec<_> = cmd
            .get_args()
            .map(|v| v.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"alice@[::1]:/tmp/a b;echo bad".into()));
        assert_eq!(
            args.last().unwrap(),
            &std::path::absolute("a:b")?.to_string_lossy()
        );
        assert!(args.windows(2).any(|v| v == ["-P", "2222"]));
        Ok(())
    }
    #[test]
    fn rejects_option_injection_and_download_globs() {
        let mut h = host();
        h.address = "-oProxyCommand=evil".into();
        assert!(command(&h, &Operation::Shell, None).is_err());
        assert!(
            command(
                &host(),
                &Operation::Download {
                    remote: "*".into(),
                    local: "out".into()
                },
                None,
            )
            .is_err()
        );
    }
}
