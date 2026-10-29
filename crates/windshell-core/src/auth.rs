//! OpenSSH askpass bridge. The helper receives only a loopback address and a
//! random capability token in its environment, never a password or private key.
use crate::{Authentication, Host, Operation, command};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Command, ExitStatus},
    thread,
    time::Duration,
};
use zeroize::Zeroizing;

// Intentionally no Debug or Serialize: credentials never belong in logs/config.
#[derive(Clone, Default)]
pub struct Credentials {
    pub password: Zeroizing<String>,
    pub private_key: Zeroizing<String>,
    pub passphrase: Zeroizing<String>,
}

impl Credentials {
    pub fn ready(&self, method: Authentication) -> bool {
        match method {
            Authentication::Auto => true,
            Authentication::Password => !self.password.is_empty(),
            Authentication::PrivateKey => !self.private_key.is_empty(),
        }
    }
    pub fn validate(&self, method: Authentication) -> Result<()> {
        match method {
            Authentication::Auto => {}
            Authentication::Password => {
                ensure!(!self.password.is_empty(), "请输入登录密码");
                ensure!(
                    !self.password.contains(['\r', '\n', '\0']),
                    "密码不能包含换行或空字符"
                );
            }
            Authentication::PrivateKey => {
                let key = self.private_key.trim();
                let types = [
                    "OPENSSH PRIVATE KEY",
                    "RSA PRIVATE KEY",
                    "EC PRIVATE KEY",
                    "PRIVATE KEY",
                    "ENCRYPTED PRIVATE KEY",
                ];
                ensure!(
                    types
                        .iter()
                        .any(|kind| key.starts_with(&format!("-----BEGIN {kind}-----"))
                            && key.ends_with(&format!("-----END {kind}-----"))),
                    "请粘贴完整私钥（包含 BEGIN / END），不是 .pub 公钥"
                );
                ensure!(
                    !self.passphrase.contains(['\r', '\n', '\0']),
                    "私钥口令不能包含换行或空字符"
                );
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct Prompt {
    pub text: String,
    pub confirm: bool,
}

#[derive(Serialize, Deserialize)]
struct Request {
    token: String,
    prompt: Prompt,
}

fn send(stream: &mut impl Write, value: &impl Serialize) -> Result<()> {
    let data = Zeroizing::new(serde_json::to_vec(value)?);
    stream.write_all(&(data.len() as u32).to_be_bytes())?;
    stream.write_all(&data)?;
    stream.flush()?;
    Ok(())
}

fn receive<T: DeserializeOwned>(stream: &mut impl Read) -> Result<T> {
    let mut size = [0; 4];
    stream.read_exact(&mut size)?;
    let size = u32::from_be_bytes(size) as usize;
    ensure!(size <= 64 * 1024, "认证消息过大");
    let mut data = Zeroizing::new(vec![0; size]);
    stream.read_exact(&mut data)?;
    Ok(serde_json::from_slice(&data)?)
}

/// Entry point when the same binary is launched by SSH_ASKPASS. No terminal UI
/// is initialized here; stdout is strictly the one-line response for OpenSSH.
pub fn askpass_helper() -> Result<()> {
    let address = std::env::var("WINDSHELL_ASKPASS_ADDR")?;
    let token = std::env::var("WINDSHELL_ASKPASS_TOKEN")?;
    let text = std::env::args().nth(1).unwrap_or_default();
    let confirm = std::env::var("SSH_ASKPASS_PROMPT").as_deref() == Ok("confirm")
        || text
            .to_ascii_lowercase()
            .contains("are you sure you want to continue connecting");
    let mut stream = TcpStream::connect(address)?;
    send(
        &mut stream,
        &Request {
            token,
            prompt: Prompt { text, confirm },
        },
    )?;
    let response: Option<String> = receive(&mut stream)?;
    let Some(response) = response.map(Zeroizing::new) else {
        bail!("已取消认证")
    };
    ensure!(!response.contains(['\r', '\n', '\0']), "认证回复必须是单行");
    let mut out = std::io::stdout().lock();
    out.write_all(response.as_bytes())?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

/// Run an operation on the real terminal, routing authentication requests to the
/// caller's UI. Pasted keys use a private temporary file kept alive until exit.
/// The OS removes it on normal completion/cancellation; SIGKILL is not cleanup.
pub fn run(
    host: &Host,
    operation: &Operation,
    ssh_config: Option<&Path>,
    credentials: &Credentials,
    helper: &Path,
    mut prompt: impl FnMut(&Prompt) -> Result<Option<Zeroizing<String>>>,
) -> Result<ExitStatus> {
    let (mut cmd, bridge) = AuthBridge::prepare(host, operation, ssh_config, credentials, helper)?;
    let mut child = cmd
        .spawn()
        .context("启动 OpenSSH 失败，请检查 ssh/scp 是否已加入 PATH")?;
    let result = (|| {
        loop {
            if let Some(status) = child.try_wait()? {
                return Ok(status);
            }
            bridge.poll(&mut prompt)?;
            thread::sleep(Duration::from_millis(30));
        }
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

/// Owns the askpass listener and temporary identity for an embedded SSH session.
pub struct AuthBridge {
    listener: TcpListener,
    token: String,
    _key: Option<tempfile::NamedTempFile>,
}
impl AuthBridge {
    pub fn prepare(
        host: &Host,
        operation: &Operation,
        ssh_config: Option<&Path>,
        credentials: &Credentials,
        helper: &Path,
    ) -> Result<(Command, Self)> {
        credentials.validate(host.authentication)?;
        let mut key = None;
        let base = command(host, operation, ssh_config)?;
        let mut cmd = Command::new(base.get_program());
        if host.authentication == Authentication::PrivateKey {
            let mut file = tempfile::NamedTempFile::new().context("创建临时私钥失败")?;
            // OpenSSH requires a trailing newline, including for clipboard input.
            file.write_all(credentials.private_key.trim().as_bytes())?;
            file.write_all(b"\n")?;
            file.flush()?;
            cmd.arg("-i").arg(file.path());
            key = Some(file);
        }
        cmd.args(base.get_args());
        let listener = TcpListener::bind("127.0.0.1:0").context("启动本地认证通道失败")?;
        listener.set_nonblocking(true)?;
        let mut random = [0_u8; 32];
        getrandom::fill(&mut random).map_err(|e| anyhow::anyhow!("生成认证令牌失败：{e}"))?;
        let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
        cmd.env("SSH_ASKPASS", helper)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("WINDSHELL_ASKPASS_ADDR", listener.local_addr()?.to_string())
            .env("WINDSHELL_ASKPASS_TOKEN", &token);
        Ok((
            cmd,
            Self {
                listener,
                token,
                _key: key,
            },
        ))
    }
    pub fn poll(
        &self,
        mut prompt: impl FnMut(&Prompt) -> Result<Option<Zeroizing<String>>>,
    ) -> Result<()> {
        match self.listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                let request: Request = match receive(&mut stream) {
                    Ok(r) => r,
                    Err(_) => return Ok(()),
                };
                if request.token != self.token {
                    return Ok(());
                }
                let response = prompt(&request.prompt)?;
                send(&mut stream, &response.as_ref().map(|s| s.as_str()))?;
                ensure!(response.is_some(), "已取消认证");
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_reject_public_keys_and_do_not_enter_host_storage() -> Result<()> {
        let mut creds = Credentials::default();
        creds
            .private_key
            .push_str("ssh-ed25519 AAAA not-a-private-key");
        assert!(creds.validate(Authentication::PrivateKey).is_err());
        assert!(creds.validate(Authentication::Password).is_err());
        creds.password.push_str("test-only-secret");
        creds.validate(Authentication::Password)?;
        let host = Host {
            name: "test".into(),
            address: "localhost".into(),
            authentication: Authentication::Password,
            ..Default::default()
        };
        let serialized = toml::to_string(&host)?;
        assert!(!serialized.contains("test-only-secret"));
        Ok(())
    }
    #[test]
    fn framed_messages_round_trip_and_reject_oversized_input() -> Result<()> {
        let mut bytes = Vec::new();
        send(&mut bytes, &Some("answer"))?;
        let reply: Option<String> = receive(&mut bytes.as_slice())?;
        assert_eq!(reply.as_deref(), Some("answer"));
        assert!(receive::<Request>(&mut (70_000u32.to_be_bytes().as_slice())).is_err());
        Ok(())
    }
}
