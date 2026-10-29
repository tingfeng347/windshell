use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use portable_pty::{CommandBuilder, MasterPty, PtySize};
use std::{
    collections::HashMap,
    io::Write,
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread,
};
use windshell_core::{Credentials, auth::AuthBridge};

pub struct Session {
    pub parser: vt100::Parser,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    output: Receiver<Vec<u8>>,
    pub auth: Option<AuthBridge>,
    pub credentials: Credentials,
    pub exited: bool,
    size: (u16, u16),
}
impl Session {
    pub fn spawn(
        cmd: CommandBuilder,
        auth: Option<AuthBridge>,
        credentials: Credentials,
    ) -> Result<Self> {
        let pair = portable_pty::native_pty_system().openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let (tx, output) = mpsc::sync_channel(64);
        thread::spawn(move || {
            let mut buf = [0; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Ok(Self {
            parser: vt100::Parser::new(24.try_into().unwrap(), 80.try_into().unwrap(), 5000),
            master: pair.master,
            writer,
            child,
            output,
            auth,
            credentials,
            exited: false,
            size: (24, 80),
        })
    }
    pub fn tick(&mut self) -> Result<()> {
        for bytes in self.output.try_iter().take(128) {
            self.parser.process(&bytes);
        }
        if !self.exited && self.child.try_wait()?.is_some() {
            self.exited = true;
            self.auth = None;
        }
        Ok(())
    }
    pub fn stop(&mut self) -> Result<()> {
        self.child.kill()?;
        self.child.wait()?;
        self.exited = true;
        self.auth = None;
        Ok(())
    }
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        let size = (rows.max(1), cols.max(1));
        if self.size != size {
            self.master.resize(PtySize {
                rows: size.0,
                cols: size.1,
                pixel_width: 0,
                pixel_height: 0,
            })?;
            self.parser
                .screen_mut()
                .set_size(size.0.try_into().unwrap(), size.1.try_into().unwrap());
            self.size = size;
        }
        Ok(())
    }
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.parser.screen_mut().set_scrollback(0);
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }
    pub fn paste(&mut self, text: &str) -> Result<()> {
        if self.parser.screen().bracketed_paste() {
            self.write(format!("\x1b[200~{text}\x1b[201~").as_bytes())
        } else {
            self.write(text.as_bytes())
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> Result<()> {
        let app = self.parser.screen().application_cursor();
        let text = match key.code {
            KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) && c.is_ascii() => {
                ((c.to_ascii_uppercase() as u8) & 0x1f) as char
            }
            KeyCode::Char(c) => c,
            _ => '\0',
        };
        let mut bytes = if text != '\0' {
            text.to_string().into_bytes()
        } else {
            match key.code {
                KeyCode::Enter => b"\r".to_vec(),
                KeyCode::Backspace => vec![127],
                KeyCode::Tab => b"\t".to_vec(),
                KeyCode::BackTab => b"\x1b[Z".to_vec(),
                KeyCode::Esc => vec![27],
                KeyCode::Up => if app { b"\x1bOA" } else { b"\x1b[A" }.to_vec(),
                KeyCode::Down => if app { b"\x1bOB" } else { b"\x1b[B" }.to_vec(),
                KeyCode::Right => if app { b"\x1bOC" } else { b"\x1b[C" }.to_vec(),
                KeyCode::Left => if app { b"\x1bOD" } else { b"\x1b[D" }.to_vec(),
                KeyCode::Home => b"\x1b[H".to_vec(),
                KeyCode::End => b"\x1b[F".to_vec(),
                KeyCode::Delete => b"\x1b[3~".to_vec(),
                KeyCode::Insert => b"\x1b[2~".to_vec(),
                KeyCode::PageUp => b"\x1b[5~".to_vec(),
                KeyCode::PageDown => b"\x1b[6~".to_vec(),
                KeyCode::F(n) => match n {
                    1..=4 => format!("\x1bO{}", (b'P' + n - 1) as char),
                    5 => "\x1b[15~".into(),
                    6..=10 => format!("\x1b[{}~", n + 11),
                    11..=12 => format!("\x1b[{}~", n + 12),
                    _ => String::new(),
                }
                .into_bytes(),
                _ => Vec::new(),
            }
        };
        if key.modifiers.contains(KeyModifiers::ALT) {
            bytes.insert(0, 27);
        }
        self.write(&bytes)
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub enum Content {
    Terminal(Box<Session>),
    File {
        path: PathBuf,
        text: String,
        scroll: u16,
        lines: Vec<ratatui::text::Line<'static>>,
    },
}
pub struct Tab {
    pub title: String,
    pub source: Option<usize>,
    pub content: Content,
}
pub struct Entry {
    pub path: PathBuf,
    pub depth: usize,
    pub directory: bool,
    pub expanded: bool,
}
struct Tree {
    root: PathBuf,
    entries: Vec<Entry>,
    selected: usize,
}
pub struct Workspace {
    pub remotes: HashMap<usize, crate::remote::Remote>,
    trees: HashMap<usize, Tree>,
    pub source: Option<usize>,
    next_source: usize,
    local_root: PathBuf,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub root: PathBuf,
    pub entries: Vec<Entry>,
    pub selected: usize,
    pub sidebar: bool,
    pub files_focused: bool,
    pub hidden: bool,
    next_terminal: usize,
}
impl Workspace {
    pub fn new() -> Self {
        Self {
            remotes: HashMap::new(),
            trees: HashMap::new(),
            source: None,
            next_source: 0,
            local_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            tabs: vec![],
            active: 0,
            root: PathBuf::from("等待 SSH 连接"),
            entries: vec![],
            selected: 0,
            sidebar: true,
            files_focused: false,
            hidden: false,
            next_terminal: 1,
        }
    }
    pub fn sync_source(&mut self) {
        let source = self.tabs.get(self.active).and_then(|t| t.source);
        self.switch_tree(source);
    }
    fn switch_tree(&mut self, source: Option<usize>) {
        if self.source == source {
            return;
        }
        if let Some(old) = self.source {
            self.trees.insert(
                old,
                Tree {
                    root: std::mem::take(&mut self.root),
                    entries: std::mem::take(&mut self.entries),
                    selected: self.selected,
                },
            );
        }
        let tree = source
            .and_then(|id| self.trees.remove(&id))
            .unwrap_or(Tree {
                root: PathBuf::from(if source.is_some() {
                    "连接中…"
                } else {
                    "等待 SSH 连接"
                }),
                entries: vec![],
                selected: 0,
            });
        self.root = tree.root;
        self.entries = tree.entries;
        self.selected = tree.selected;
        self.source = source;
    }
    pub fn attach(&mut self, remote: crate::remote::Remote) {
        let id = self.next_source;
        self.next_source += 1;
        self.remotes.insert(id, remote);
        self.tabs[self.active].source = Some(id);
        self.sync_source();
    }
    pub fn remote_status(&self) -> String {
        match self.source.and_then(|id| self.remotes.get(&id)) {
            Some(remote) => format!("{} · {}", remote.host, remote.status()),
            None => "连接 SSH 后显示远端文件".into(),
        }
    }
    pub fn poll_remote(&mut self) -> Result<()> {
        self.sync_source();
        let replies: Vec<_> = self
            .remotes
            .iter_mut()
            .filter_map(|(&id, remote)| remote.poll().map(|r| (id, r)))
            .collect();
        let original = self.source;
        let mut error = None;
        for (id, reply) in replies {
            self.switch_tree(Some(id));
            let result = match reply {
                Ok(crate::remote::Reply::Directory(path, entries)) => {
                    self.apply_directory(path, entries);
                    Ok(())
                }
                Ok(crate::remote::Reply::File(path, data)) => {
                    self.preview(PathBuf::from(path), data, original == Some(id))
                }
                Err(e) => Err(e),
            };
            if let Err(e) = result {
                if let Some(remote) = self.remotes.get_mut(&id) {
                    remote.error = Some(format!("{e:#}"));
                }
                error = Some(e);
            }
        }
        self.switch_tree(original);
        if let Some(e) = error {
            return Err(e);
        }
        Ok(())
    }
    fn apply_directory(&mut self, path: String, entries: Vec<(String, bool)>) {
        let path = PathBuf::from(path);
        let parent = self
            .entries
            .iter()
            .position(|e| e.path == path && e.directory);
        let depth = parent.map(|i| self.entries[i].depth + 1).unwrap_or(0);
        let mut children: Vec<_> = entries
            .into_iter()
            .filter(|(name, _)| self.hidden || !name.starts_with('.'))
            .map(|(name, directory)| Entry {
                path: path.join(name),
                directory,
                depth,
                expanded: false,
            })
            .collect();
        children.sort_by(|a, b| {
            b.directory
                .cmp(&a.directory)
                .then_with(|| a.path.cmp(&b.path))
        });
        if let Some(i) = parent {
            self.entries[i].expanded = true;
            self.entries.splice(i + 1..i + 1, children);
        } else {
            self.root = path;
            self.entries = children;
            self.selected = 0;
        }
    }
    pub fn refresh(&mut self) -> Result<()> {
        if let Some(remote) = self.source.and_then(|id| self.remotes.get_mut(&id)) {
            remote.request(crate::remote::Request::List(if self.root.is_absolute() {
                self.root.to_string_lossy().into()
            } else {
                ".".into()
            }))?;
        }
        Ok(())
    }
    fn preview(&mut self, path: PathBuf, data: Vec<u8>, activate: bool) -> Result<()> {
        anyhow::ensure!(data.len() <= 1024 * 1024, "文件超过 1 MiB");
        anyhow::ensure!(!data.contains(&0), "二进制文件无法作为文本预览");
        let text = String::from_utf8(data).context("预览仅支持 UTF-8 文本")?;
        let text: String = text
            .chars()
            .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
            .collect();
        let lines = crate::highlight::code(&path, &text)?;
        self.tabs.push(Tab {
            title: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            source: self.source,
            content: Content::File {
                path,
                text,
                scroll: 0,
                lines,
            },
        });
        if activate {
            self.active = self.tabs.len() - 1;
            self.files_focused = false;
        }
        Ok(())
    }
    pub fn open(&mut self, index: usize) -> Result<()> {
        let Some(entry) = self.entries.get(index) else {
            return Ok(());
        };
        self.selected = index;
        if entry.directory {
            let depth = entry.depth;
            if entry.expanded {
                let end = self.entries[index + 1..]
                    .iter()
                    .position(|e| e.depth <= depth)
                    .map(|n| index + 1 + n)
                    .unwrap_or(self.entries.len());
                self.entries.drain(index + 1..end);
                self.entries[index].expanded = false;
            } else {
                let path = entry.path.to_string_lossy().into_owned();
                self.remotes
                    .get_mut(&self.source.context("请先连接 SSH")?)
                    .context("SSH 已关闭")?
                    .request(crate::remote::Request::List(path))?;
            }
        } else {
            let path = entry.path.clone();
            if let Some(i) = self.tabs.iter().position(|t| {
                matches!(&t.content, Content::File { path: p, .. } if p == &path)
                    && t.source == self.source
            }) {
                self.active = i;
                self.files_focused = false;
                return Ok(());
            }
            self.remotes
                .get_mut(&self.source.context("请先连接 SSH")?)
                .context("SSH 已关闭")?
                .request(crate::remote::Request::Read(
                    path.to_string_lossy().into_owned(),
                ))?;
        }
        Ok(())
    }
    pub fn local(&mut self) -> Result<()> {
        let shell = std::env::var_os("SHELL").unwrap_or_else(|| {
            if cfg!(windows) {
                "powershell.exe".into()
            } else {
                "/bin/sh".into()
            }
        });
        let mut cmd = CommandBuilder::new(shell);
        cmd.cwd(&self.local_root);
        cmd.env("TERM", "xterm-256color");
        let session = Session::spawn(cmd, None, Credentials::default())?;
        self.push(format!("终端 {}", self.next_terminal), session);
        self.next_terminal += 1;
        Ok(())
    }
    pub fn push(&mut self, title: String, session: Session) {
        self.tabs.push(Tab {
            source: None,
            title,
            content: Content::Terminal(Box::new(session)),
        });
        self.active = self.tabs.len() - 1;
        self.files_focused = false;
        self.sync_source();
    }
    pub fn scroll(&mut self, up: bool, lines: usize) {
        if self.files_focused {
            self.selected = if up {
                self.selected.saturating_sub(lines)
            } else {
                self.selected
                    .saturating_add(lines)
                    .min(self.entries.len().saturating_sub(1))
            };
        } else if let Some(tab) = self.tabs.get_mut(self.active) {
            match &mut tab.content {
                Content::File { scroll, text, .. } => {
                    *scroll = if up {
                        scroll.saturating_sub(lines as u16)
                    } else {
                        scroll.saturating_add(lines as u16).min(
                            text.lines()
                                .count()
                                .saturating_sub(1)
                                .min(u16::MAX as usize) as u16,
                        )
                    };
                }
                Content::Terminal(s) => {
                    let offset = s.parser.screen().scrollback();
                    s.parser.screen_mut().set_scrollback(if up {
                        offset.saturating_add(lines)
                    } else {
                        offset.saturating_sub(lines)
                    });
                }
            }
        }
    }
    pub fn parent(&mut self) -> Result<()> {
        if let Some(path) = self.root.parent().filter(|p| !p.as_os_str().is_empty())
            && let Some(remote) = self.source.and_then(|id| self.remotes.get_mut(&id))
        {
            remote.request(crate::remote::Request::List(
                path.to_string_lossy().into_owned(),
            ))?;
        }
        Ok(())
    }
    pub fn collapse(&mut self) {
        if let Some(entry) = self.entries.get(self.selected) {
            let depth = entry.depth;
            if entry.directory && entry.expanded {
                let _ = self.open(self.selected);
            } else if depth > 0
                && let Some(i) = (0..self.selected)
                    .rev()
                    .find(|&i| self.entries[i].depth < depth)
            {
                self.selected = i;
            }
        }
    }
    pub fn close(&mut self) {
        if !self.tabs.is_empty() {
            self.tabs.remove(self.active);
            self.remotes
                .retain(|id, _| self.tabs.iter().any(|t| t.source == Some(*id)));
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
            self.sync_source();
            self.trees.retain(|id, _| self.remotes.contains_key(id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_trees_and_previews_are_isolated() -> Result<()> {
        let mut w = Workspace::new();
        w.switch_tree(Some(1));
        w.apply_directory(
            "/home/a".into(),
            vec![("main.rs".into(), false), ("src".into(), true)],
        );
        assert!(w.entries[0].directory);
        w.preview("/home/a/main.rs".into(), b"fn main() {}".to_vec(), true)?;
        w.switch_tree(Some(2));
        assert!(w.entries.is_empty());
        w.apply_directory("/home/b".into(), vec![("other.py".into(), false)]);
        w.switch_tree(Some(1));
        assert_eq!(w.root, PathBuf::from("/home/a"));
        assert_eq!(w.entries.len(), 2);
        assert!(w.preview("bad.bin".into(), vec![0], true).is_err());
        w.open(1)?;
        assert_eq!(w.tabs.len(), 1);
        w.switch_tree(None);
        assert!(w.entries.is_empty());
        Ok(())
    }
}
