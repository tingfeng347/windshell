//! Minimal read-only SFTP v3 transport over an authenticated OpenSSH master.
use anyhow::{Context, Result, bail, ensure};
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub enum Request {
    List(String),
    Read(String),
}
pub enum Reply {
    Directory(String, Vec<(String, bool)>),
    File(String, Vec<u8>),
}
pub struct Remote {
    pub socket: PathBuf,
    _directory: tempfile::TempDir,
    child: Option<Child>,
    initialized: bool,
    replies: Option<Receiver<Result<Reply>>>,
    pub busy: bool,
    pub error: Option<String>,
    started: Option<Instant>,
    pub host: String,
}
impl Remote {
    pub fn new(host: String) -> Result<Self> {
        let directory = tempfile::Builder::new().prefix("ws-").tempdir()?;
        Ok(Self {
            socket: directory.path().join("ssh"),
            _directory: directory,
            child: None,
            initialized: false,
            replies: None,
            busy: false,
            error: None,
            started: None,
            host,
        })
    }
    pub fn status(&self) -> &str {
        self.error.as_deref().unwrap_or(if self.busy {
            "加载中…"
        } else if !self.initialized {
            "连接中…"
        } else if !self.socket.exists() {
            "SSH 已断开"
        } else {
            "SFTP"
        })
    }
    pub fn request(&mut self, request: Request) -> Result<()> {
        ensure!(!self.busy, "远端文件正在加载，请稍候");
        ensure!(self.socket.exists(), "SSH 尚未连接或已断开");
        self.error = None;
        self.start(request)
    }
    pub fn poll(&mut self) -> Option<Result<Reply>> {
        if !self.initialized && self.socket.exists() {
            self.initialized = true;
            if let Err(e) = self.start(Request::List(".".into())) {
                self.error = Some(format!("{e:#}"));
                return Some(Err(e));
            }
        }
        if let Some(rx) = &self.replies {
            match rx.try_recv() {
                Ok(reply) => {
                    self.finish();
                    if let Err(e) = &reply {
                        self.error = Some(format!("{e:#}"));
                    }
                    return Some(reply);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.finish();
                    self.error = Some("SFTP 文件通道已关闭，请刷新重试".into());
                }
                _ => {}
            }
        }
        if self
            .started
            .is_some_and(|t| t.elapsed() > Duration::from_secs(20))
        {
            if let Some(child) = &mut self.child {
                let _ = child.kill();
                let _ = child.wait();
            }

            self.replies = None;
            self.started = None;
            self.busy = false;
            self.error = Some("远端文件操作超时；重新连接 SSH 后重试".into());
        }
        None
    }
    fn finish(&mut self) {
        self.busy = false;
        self.started = None;
        self.replies = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    fn start(&mut self, request: Request) -> Result<()> {
        // ProxyCommand=false prevents silent fallback to a new unauthenticated
        // connection if the master disappears between the existence check and exec.
        let mut child = Command::new("ssh")
            .arg("-S")
            .arg(&self.socket)
            .args([
                "-F",
                "/dev/null",
                "-oControlMaster=no",
                "-oBatchMode=yes",
                "-oProxyCommand=false",
                "-T",
                "-s",
                "--",
                "windshell-multiplex",
                "sftp",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take().context("SFTP stdin")?;
        let output = child.stdout.take().context("SFTP stdout")?;
        let (results, replies) = mpsc::channel();
        thread::spawn(move || {
            let mut client = Protocol {
                input,
                output,
                id: 0,
            };
            let reply = client.init().and_then(|_| match request {
                Request::List(path) => client.list(&path),
                Request::Read(path) => client.read(&path),
            });
            drop(client);
            let _ = results.send(reply);
        });
        self.child = Some(child);
        self.replies = Some(replies);
        self.busy = true;
        self.started = Some(Instant::now());
        Ok(())
    }
}
impl Drop for Remote {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct Protocol {
    input: std::process::ChildStdin,
    output: std::process::ChildStdout,
    id: u32,
}
fn string(out: &mut Vec<u8>, s: &[u8]) {
    out.extend((s.len() as u32).to_be_bytes());
    out.extend(s);
}
struct Packet {
    bytes: Vec<u8>,
    pos: usize,
}
impl Packet {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        ensure!(
            n <= self.bytes.len().saturating_sub(self.pos),
            "SFTP 响应不完整"
        );
        let start = self.pos;
        self.pos += n;
        Ok(&self.bytes[start..self.pos])
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into()?))
    }
    fn string(&mut self) -> Result<Vec<u8>> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    fn text(&mut self) -> Result<String> {
        String::from_utf8(self.string()?).context("远端路径不是 UTF-8")
    }
    fn attrs(&mut self) -> Result<(Option<u64>, Option<u32>)> {
        let flags = self.u32()?;
        let size = if flags & 1 != 0 {
            Some(u64::from_be_bytes(self.take(8)?.try_into()?))
        } else {
            None
        };
        if flags & 2 != 0 {
            self.take(8)?;
        }
        let mode = if flags & 4 != 0 {
            Some(self.u32()?)
        } else {
            None
        };
        if flags & 8 != 0 {
            self.take(8)?;
        }
        if flags & 0x80000000 != 0 {
            let n = self.u32()?;
            ensure!(n < 1024, "SFTP 属性过多");
            for _ in 0..n {
                self.string()?;
                self.string()?;
            }
        }
        Ok((size, mode))
    }
}
impl Protocol {
    fn send(&mut self, bytes: &[u8]) -> Result<()> {
        self.input.write_all(&(bytes.len() as u32).to_be_bytes())?;
        self.input.write_all(bytes)?;
        self.input.flush()?;
        Ok(())
    }
    fn receive(&mut self) -> Result<Packet> {
        let mut len = [0; 4];
        self.output
            .read_exact(&mut len)
            .context("无法读取 SFTP 响应（服务器可能未启用 SFTP）")?;
        let n = u32::from_be_bytes(len) as usize;
        ensure!(n > 0 && n <= 2 * 1024 * 1024, "SFTP 响应长度无效");
        let mut bytes = vec![0; n];
        self.output.read_exact(&mut bytes)?;
        Ok(Packet { bytes, pos: 0 })
    }
    fn init(&mut self) -> Result<()> {
        self.send(&[1, 0, 0, 0, 3])?;
        let mut p = self.receive()?;
        ensure!(p.take(1)? == [2] && p.u32()? == 3, "服务器不支持 SFTP v3");
        Ok(())
    }
    fn call(&mut self, kind: u8, args: Vec<u8>) -> Result<(u8, Packet)> {
        self.id += 1;
        let mut b = vec![kind];
        b.extend(self.id.to_be_bytes());
        b.extend(args);
        self.send(&b)?;
        let mut p = self.receive()?;
        let kind = p.take(1)?[0];
        ensure!(p.u32()? == self.id, "SFTP 请求标识不匹配");
        Ok((kind, p))
    }
    fn expect(&mut self, kind: u8, args: Vec<u8>, expected: u8) -> Result<Packet> {
        let (kind, mut p) = self.call(kind, args)?;
        if kind == 101 {
            let code = p.u32()?;
            let message = p.text()?;
            bail!("SFTP 错误 {code}: {message}");
        }
        ensure!(kind == expected, "SFTP 响应类型异常");
        Ok(p)
    }
    fn path(path: &str) -> Vec<u8> {
        let mut b = vec![];
        string(&mut b, path.as_bytes());
        b
    }
    fn close(&mut self, handle: &[u8]) -> Result<()> {
        let mut b = vec![];
        string(&mut b, handle);
        let (kind, mut p) = self.call(4, b)?;
        ensure!(kind == 101 && p.u32()? == 0, "关闭远端文件失败");
        Ok(())
    }
    fn list(&mut self, path: &str) -> Result<Reply> {
        let mut real = self.expect(16, Self::path(path), 104)?;
        ensure!(real.u32()? > 0, "远端目录不存在");
        let path = real.text()?;
        let handle = self.expect(11, Self::path(&path), 102)?.string()?;
        let result = (|| {
            let mut entries = Vec::new();
            loop {
                let mut b = vec![];
                string(&mut b, &handle);
                let (kind, mut p) = self.call(12, b)?;
                if kind == 101 {
                    let status = p.u32()?;
                    ensure!(status == 1, "读取远端目录失败：{}", p.text()?);
                    break;
                }
                ensure!(kind == 104, "SFTP 目录响应异常");
                let count = p.u32()?;
                ensure!(count <= 20000, "目录项目过多");
                for _ in 0..count {
                    let name = p.text()?;
                    p.string()?;
                    let (_, mode) = p.attrs()?;
                    if name == "." || name == ".." {
                        continue;
                    }
                    ensure!(
                        !name.contains('/') && !name.contains('\0'),
                        "远端返回无效文件名"
                    );
                    let kind = mode.unwrap_or(0) & 0o170000;
                    if kind == 0o100000 || kind == 0o040000 {
                        entries.push((name, kind == 0o040000));
                    }
                }
                ensure!(entries.len() <= 20000, "目录超过 20000 项");
            }
            Ok(Reply::Directory(path, entries))
        })();
        let close = self.close(&handle);
        let reply = result?;
        close?;
        Ok(reply)
    }
    fn read(&mut self, path: &str) -> Result<Reply> {
        let (size, mode) = self.expect(17, Self::path(path), 105)?.attrs()?;
        ensure!(
            mode.is_some_and(|m| m & 0o170000 == 0o100000),
            "仅支持预览普通文件"
        );
        ensure!(
            size.is_some_and(|n| n <= 1024 * 1024),
            "文件超过 1 MiB，暂不支持预览"
        );
        let mut args = Self::path(path);
        args.extend(1u32.to_be_bytes());
        args.extend(0u32.to_be_bytes());
        let handle = self.expect(3, args, 102)?.string()?;
        let result = (|| {
            let mut data = Vec::new();
            loop {
                let mut args = vec![];
                string(&mut args, &handle);
                args.extend((data.len() as u64).to_be_bytes());
                args.extend(32768u32.to_be_bytes());
                let (kind, mut p) = self.call(5, args)?;
                if kind == 101 {
                    ensure!(p.u32()? == 1, "远端文件读取失败：{}", p.text()?);
                    break;
                }
                ensure!(kind == 103, "SFTP 数据响应异常");
                let chunk = p.string()?;
                ensure!(!chunk.is_empty(), "远端返回空数据包");
                data.extend(chunk);
                ensure!(data.len() <= 1024 * 1024, "文件超过 1 MiB");
            }
            Ok(Reply::File(path.into(), data))
        })();
        let close = self.close(&handle);
        let reply = result?;
        close?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_packets_fail_without_allocation_or_overread() {
        let mut truncated = Packet {
            bytes: vec![0, 0, 0, 8, b'a'],
            pos: 0,
        };
        assert!(truncated.string().is_err());
        let mut oversized = Packet {
            bytes: u32::MAX.to_be_bytes().to_vec(),
            pos: 0,
        };
        assert!(oversized.string().is_err());
        let mut attrs = Packet {
            bytes: 1u32.to_be_bytes().to_vec(),
            pos: 0,
        };
        assert!(attrs.attrs().is_err());
    }
}
