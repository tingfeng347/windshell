use crate::input::Input;
use crate::workspace::{Content, Session, Workspace};
use anyhow::{Context, Result};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::DefaultTerminal;
use std::{collections::HashMap, path::PathBuf};
use windshell_core::{Authentication, Credentials, Host, HostStore, Operation};
use zeroize::Zeroizing;

pub struct HostForm {
    pub index: Option<usize>,
    pub fields: [Input; 4],
    pub method: Authentication,
    pub credentials: CredentialForm,
    pub identity: String,
    pub focus: usize,
}

pub struct CredentialForm {
    pub method: Authentication,
    pub password: Input,
    pub key: Input,
    pub passphrase: Input,
    pub focus: usize,
    pub reveal: bool,
}
impl CredentialForm {
    pub fn new(method: Authentication, creds: &Credentials) -> Self {
        Self {
            method,
            password: Input::new(&creds.password),
            key: Input::new(&creds.private_key),
            passphrase: Input::new(&creds.passphrase),
            focus: 0,
            reveal: false,
        }
    }
    pub fn count(&self) -> usize {
        if self.method == Authentication::PrivateKey {
            2
        } else {
            1
        }
    }
    pub fn input(&mut self, index: usize) -> &mut Input {
        if self.method == Authentication::PrivateKey {
            if index == 0 {
                &mut self.key
            } else {
                &mut self.passphrase
            }
        } else {
            &mut self.password
        }
    }
    pub fn value(&self) -> Credentials {
        Credentials {
            password: if self.method == Authentication::Password {
                self.password.text.clone()
            } else {
                Default::default()
            },
            private_key: if self.method == Authentication::PrivateKey {
                self.key.text.clone()
            } else {
                Default::default()
            },
            passphrase: if self.method == Authentication::PrivateKey {
                self.passphrase.text.clone()
            } else {
                Default::default()
            },
        }
    }
}
impl HostForm {
    pub fn count(&self) -> usize {
        5 + match self.method {
            Authentication::Auto => 0,
            Authentication::Password => 1,
            Authentication::PrivateKey => 2,
        }
    }
    pub fn input(&mut self) -> Option<&mut Input> {
        match self.focus {
            0..=3 => Some(&mut self.fields[self.focus]),
            5..=6 if self.focus < self.count() => Some(self.credentials.input(self.focus - 5)),
            _ => None,
        }
    }
}

pub enum Mode {
    Browse,
    Manage,
    Settings,
    Edit(Box<HostForm>),
    Auth {
        operation: Operation,
        form: CredentialForm,
    },
    Transfer {
        upload: bool,
        fields: [Input; 2],
        focus: usize,
    },
    Delete {
        focus: usize,
    },
}

pub enum Action {
    None,
    Quit,
    Run(Operation),
}
#[derive(Clone, Copy)]
pub enum Target {
    Host(usize),
    Manage,
    Settings,
    ToggleFiles,
    NewTerminal,
    Tab(usize),
    CloseTab,
    File(usize),
    TerminalFocus,
    ToggleHidden,
    Refresh,
    New,
    Connect,
    Upload,
    Download,
    Edit,
    Delete,
    Field(usize),
    Button(usize),
    AuthCycle,
}

pub struct App {
    pub workspace: Workspace,
    pub store: HostStore,
    pub path: PathBuf,
    pub ssh_config: Option<PathBuf>,
    pub selected: usize,
    pub mode: Mode,
    pub message: String,
    pub error: bool,
    credentials: HashMap<String, Credentials>,
}

pub fn resume(terminal: &mut DefaultTerminal) -> Result<()> {
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::EnterAlternateScreen,
        event::EnableBracketedPaste,
        event::EnableMouseCapture
    )?;
    terminal.clear()?;
    Ok(())
}
pub fn suspend() -> Result<()> {
    crossterm::execute!(
        std::io::stdout(),
        event::DisableBracketedPaste,
        event::DisableMouseCapture
    )?;
    ratatui::try_restore()?;
    Ok(())
}

impl App {
    pub fn new(store: HostStore, path: PathBuf) -> Self {
        Self {
            workspace: Workspace::new(),
            store,
            path,
            ssh_config: None,
            selected: 0,
            mode: Mode::Browse,
            message: "选择一台主机开始，或新建连接。".into(),
            error: false,
            credentials: HashMap::new(),
        }
    }
    pub fn run(&mut self) -> Result<()> {
        let mut terminal = ratatui::try_init()?;
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = crossterm::execute!(
                std::io::stdout(),
                event::DisableBracketedPaste,
                event::DisableMouseCapture
            );
            previous_hook(info);
        }));
        let result = (|| -> Result<()> {
            resume(&mut terminal)?;
            if let Err(e) = self
                .workspace
                .refresh()
                .and_then(|_| self.workspace.local())
            {
                self.notice(format!("{e:#}"), true);
            }
            loop {
                self.tick(&mut terminal)?;
                let mut targets = Vec::new();
                terminal.draw(|f| targets = crate::ui::draw(f, self))?;
                if !event::poll(std::time::Duration::from_millis(30))? {
                    continue;
                }
                let action = match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => self.key(key),
                    Event::Paste(text) => {
                        self.paste(Zeroizing::new(text));
                        Action::None
                    }
                    Event::Mouse(mouse) => {
                        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                            targets
                                .iter()
                                .rev()
                                .find(|(rect, _)| rect.contains((mouse.column, mouse.row).into()))
                                .map(|(_, target)| self.click(*target))
                                .unwrap_or(Action::None)
                        } else {
                            if matches!(self.mode, Mode::Browse)
                                && matches!(
                                    mouse.kind,
                                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                                )
                            {
                                self.workspace.files_focused = self.workspace.sidebar
                                    && mouse.column
                                        < crate::ui::sidebar_width(terminal.size()?.width);
                                self.workspace
                                    .scroll(mouse.kind == MouseEventKind::ScrollUp, 3);
                            }
                            Action::None
                        }
                    }
                    _ => Action::None,
                };
                match action {
                    Action::None => {}
                    Action::Quit => break,
                    Action::Run(operation) => self.execute(&mut terminal, operation)?,
                }
            }
            Ok(())
        })();
        // Also clean up bracketed paste/mouse on propagated errors.
        let cleanup = suspend();
        result.and(cleanup)
    }

    fn execute(&mut self, terminal: &mut DefaultTerminal, operation: Operation) -> Result<()> {
        if matches!(operation, Operation::Shell) {
            let result = self.connect_tab();
            if let Err(e) = result {
                self.notice(format!("{e:#}"), true);
            }
            return Ok(());
        }
        let host = self.store.hosts[self.selected].clone();
        let mut creds = self
            .credentials
            .get(&host.name)
            .cloned()
            .unwrap_or_default();
        let prepared = creds.clone();
        let mut password_sent = false;
        let mut passphrase_sent = false;
        let helper = std::env::current_exe()?;
        suspend()?;
        let result = windshell_core::run(
            &host,
            &operation,
            self.ssh_config.as_deref(),
            &prepared,
            &helper,
            |prompt| {
                let lower = prompt.text.to_ascii_lowercase();
                if !prompt.confirm
                    && lower.contains("passphrase")
                    && !passphrase_sent
                    && !creds.passphrase.is_empty()
                {
                    passphrase_sent = true;
                    return Ok(Some(creds.passphrase.clone()));
                }
                if !prompt.confirm
                    && lower.contains("password")
                    && !password_sent
                    && !creds.password.is_empty()
                {
                    password_sent = true;
                    return Ok(Some(creds.password.clone()));
                }
                let reply = crate::ui::prompt(terminal, &host.name, prompt)?;
                if let Some(reply) = &reply {
                    if !prompt.confirm && lower.contains("passphrase") {
                        creds.passphrase = reply.clone();
                    } else if !prompt.confirm && lower.contains("password") {
                        creds.password = reply.clone();
                    }
                }
                Ok(reply)
            },
        );
        resume(terminal)?;
        self.mode = Mode::Manage;
        match result {
            Ok(status) if status.success() => {
                self.credentials.insert(host.name, creds);
                self.notice("操作完成，已返回连接列表", false);
            }
            Ok(status) => self.notice(
                format!("操作未成功（{status}）。请检查认证信息或终端中的 SSH 提示。"),
                true,
            ),
            Err(e) => self.notice(format!("{e:#}"), true),
        }
        Ok(())
    }

    fn connect_tab(&mut self) -> Result<()> {
        let host = &self.store.hosts[self.selected];
        let creds = self
            .credentials
            .get(&host.name)
            .cloned()
            .unwrap_or_default();
        let (command, bridge) = windshell_core::auth::AuthBridge::prepare(
            host,
            &Operation::Shell,
            self.ssh_config.as_deref(),
            &creds,
            &std::env::current_exe()?,
        )?;
        let mut cmd = portable_pty::CommandBuilder::new(command.get_program());
        let remote = crate::remote::Remote::new(host.name.clone())?;
        cmd.args(["-oControlMaster=yes", "-oControlPersist=no", "-o"]);
        cmd.arg(format!("ControlPath={}", remote.socket.display()));
        cmd.args(command.get_args());
        for (name, value) in command.get_envs() {
            if let Some(value) = value {
                cmd.env(name, value);
            }
        }
        cmd.env("TERM", "xterm-256color");
        let session = Session::spawn(cmd, Some(bridge), creds)?;
        self.workspace.push(host.name.clone(), session);
        self.workspace.attach(remote);
        self.mode = Mode::Browse;
        self.notice("已打开 SSH 标签页", false);
        Ok(())
    }
    fn tick(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let r = self.workspace.poll_remote();
        self.workspace_result(r);
        let size = terminal.size()?;
        let cols = size.width.saturating_sub(if self.workspace.sidebar {
            crate::ui::sidebar_width(size.width)
        } else {
            0
        });
        let mut failure = None;
        for tab in &mut self.workspace.tabs {
            if let Content::Terminal(session) = &mut tab.content {
                session.resize(size.height.saturating_sub(3), cols)?;
                session.tick()?;
                if let Some(bridge) = session.auth.take() {
                    let cached = self.credentials.get_mut(&tab.title);
                    let mut cached = cached;
                    let creds = &mut session.credentials;
                    let result = bridge.poll(|prompt| {
                        let lower = prompt.text.to_ascii_lowercase();
                        if !prompt.confirm
                            && lower.contains("password")
                            && !creds.password.is_empty()
                        {
                            return Ok(Some(std::mem::take(&mut creds.password)));
                        }
                        if !prompt.confirm
                            && lower.contains("passphrase")
                            && !creds.passphrase.is_empty()
                        {
                            return Ok(Some(std::mem::take(&mut creds.passphrase)));
                        }
                        let reply = crate::ui::prompt(terminal, &tab.title, prompt);
                        resume(terminal)?;
                        if let Ok(Some(value)) = &reply
                            && let Some(cached) = cached.as_mut()
                            && !prompt.confirm
                        {
                            if lower.contains("passphrase") {
                                cached.passphrase = value.clone();
                            } else if lower.contains("password") {
                                cached.password = value.clone();
                            }
                        }
                        reply
                    });
                    if let Err(e) = result {
                        failure = Some(format!("{e:#}"));
                        session.stop()?;
                    } else {
                        session.auth = Some(bridge);
                    }
                }
            }
        }
        if let Some(message) = failure {
            self.notice(message, true);
        }
        Ok(())
    }
    fn workspace_result(&mut self, result: Result<()>) {
        if let Err(e) = result {
            self.notice(format!("{e:#}"), true);
        }
    }
    fn workspace_key(&mut self, key: KeyEvent) -> Action {
        if key.modifiers.contains(KeyModifiers::SHIFT)
            && matches!(key.code, KeyCode::PageUp | KeyCode::PageDown)
        {
            self.workspace.scroll(key.code == KeyCode::PageUp, 20);
            return Action::None;
        }
        if key.modifiers.contains(KeyModifiers::ALT) {
            match key.code {
                KeyCode::Char('q') => return Action::Quit,
                KeyCode::Char('b') => {
                    self.workspace.sidebar = !self.workspace.sidebar;
                    self.workspace.files_focused = self.workspace.sidebar;
                }
                KeyCode::Char('t') => {
                    let r = self.workspace.local();
                    self.workspace_result(r);
                }
                KeyCode::Char('w') => self.workspace.close(),
                KeyCode::Char('s') => self.mode = Mode::Manage,
                KeyCode::Char(',') => self.mode = Mode::Settings,
                KeyCode::Right | KeyCode::Left if !self.workspace.tabs.is_empty() => {
                    let n = self.workspace.tabs.len();
                    self.workspace.active = (self.workspace.active
                        + if key.code == KeyCode::Right { 1 } else { n - 1 })
                        % n;
                    self.workspace.files_focused = false;
                }
                _ => {
                    if let Some(tab) = self.workspace.tabs.get_mut(self.workspace.active)
                        && let Content::Terminal(s) = &mut tab.content
                    {
                        let r = s.key(key);
                        self.workspace_result(r);
                    }
                }
            }
            return Action::None;
        }
        if self.workspace.files_focused {
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    self.workspace.selected = (self.workspace.selected + 1)
                        .min(self.workspace.entries.len().saturating_sub(1))
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.workspace.selected = self.workspace.selected.saturating_sub(1)
                }
                KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                    let r = self.workspace.open(self.workspace.selected);
                    self.workspace_result(r);
                }
                KeyCode::Left | KeyCode::Char('h') => self.workspace.collapse(),
                KeyCode::Backspace => {
                    let r = self.workspace.parent();
                    self.workspace_result(r);
                }
                KeyCode::Tab | KeyCode::Esc => self.workspace.files_focused = false,
                KeyCode::Char('r') => {
                    let r = self.workspace.refresh();
                    self.workspace_result(r);
                }
                _ => {}
            }
        } else if let Some(tab) = self.workspace.tabs.get_mut(self.workspace.active) {
            match &mut tab.content {
                Content::Terminal(session) => {
                    let r = session.key(key);
                    self.workspace_result(r);
                }
                Content::File { scroll, .. } => match key.code {
                    KeyCode::Down | KeyCode::Char('j') => *scroll = scroll.saturating_add(1),
                    KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                    KeyCode::PageDown => *scroll = scroll.saturating_add(20),
                    KeyCode::PageUp => *scroll = scroll.saturating_sub(20),
                    KeyCode::Home => *scroll = 0,
                    _ => {}
                },
            }
        }
        Action::None
    }

    fn notice(&mut self, text: impl Into<String>, error: bool) {
        self.message = text.into();
        self.error = error;
    }
    fn edit_host(&mut self, index: Option<usize>) {
        let host = index
            .map(|i| self.store.hosts[i].clone())
            .unwrap_or_default();
        let creds = self
            .credentials
            .get(&host.name)
            .cloned()
            .unwrap_or_default();
        self.mode = Mode::Edit(Box::new(HostForm {
            index,
            fields: [
                Input::new(&host.name),
                Input::new(&host.address),
                Input::new(&host.user),
                Input::new(&host.port.map(|p| p.to_string()).unwrap_or_default()),
            ],
            method: host.authentication,
            credentials: CredentialForm::new(host.authentication, &creds),
            identity: host.identity,
            focus: 0,
        }));
        self.notice(
            "填写主机信息，选择认证方式；凭据只保留在本次运行中。",
            false,
        );
    }
    fn request(&mut self, operation: Operation) -> Action {
        let host = &self.store.hosts[self.selected];
        let creds = self
            .credentials
            .get(&host.name)
            .cloned()
            .unwrap_or_default();
        if !creds.ready(host.authentication) {
            self.mode = Mode::Auth {
                operation,
                form: CredentialForm::new(host.authentication, &creds),
            };
            self.notice("输入本次连接的凭据；不会写入连接配置。", false);
            Action::None
        } else {
            self.mode = Mode::Browse;
            Action::Run(operation)
        }
    }
    fn save(&mut self, connect: bool) -> Result<Action> {
        let Mode::Edit(form) = &self.mode else {
            return Ok(Action::None);
        };
        let address = form.fields[1].text.trim().to_string();
        anyhow::ensure!(!address.is_empty(), "请输入主机地址或 SSH 别名");
        let name = form.fields[0].text.trim();
        let host = Host {
            name: if name.is_empty() {
                address.clone()
            } else {
                name.into()
            },
            address,
            user: form.fields[2].text.trim().into(),
            port: if form.fields[3].text.trim().is_empty() {
                None
            } else {
                Some(
                    form.fields[3]
                        .text
                        .trim()
                        .parse()
                        .context("端口须为 1–65535 的数字")?,
                )
            },
            identity: form.identity.clone(),
            authentication: form.method,
        };
        let creds = form.credentials.value();
        if creds.ready(form.method) {
            creds.validate(form.method)?;
        }
        let mut next = self.store.clone();
        let index = form.index.unwrap_or(next.hosts.len());
        let old_name = form.index.map(|i| next.hosts[i].name.clone());
        if form.index.is_some() {
            next.hosts[index] = host.clone();
        } else {
            next.hosts.push(host.clone());
        }
        next.save(&self.path)?;
        self.store = next;
        self.selected = index;
        if let Some(name) = old_name {
            self.credentials.remove(&name);
        }
        self.credentials.insert(host.name, creds);
        self.mode = Mode::Browse;
        if !connect {
            self.mode = Mode::Manage;
        }
        self.notice("配置已保存，凭据仅保留在本次运行中", false);
        Ok(if connect {
            self.request(Operation::Shell)
        } else {
            Action::None
        })
    }
    fn submit(&mut self, button: usize) -> Result<Action> {
        match &self.mode {
            Mode::Edit(_) => match button {
                0 => self.save(true),
                1 => self.save(false),
                _ => {
                    self.mode = Mode::Browse;
                    Ok(Action::None)
                }
            },
            Mode::Auth { operation, form } => {
                if button == 1 {
                    self.mode = Mode::Browse;
                    return Ok(Action::None);
                }
                let creds = form.value();
                creds.validate(form.method)?;
                self.credentials
                    .insert(self.store.hosts[self.selected].name.clone(), creds);
                let operation = operation.clone();
                self.mode = Mode::Browse;
                Ok(Action::Run(operation))
            }
            Mode::Transfer { upload, fields, .. } => {
                if button == 1 {
                    self.mode = Mode::Browse;
                    return Ok(Action::None);
                }
                anyhow::ensure!(!fields[0].text.is_empty(), "请选择或输入本地文件路径");
                let local = PathBuf::from(fields[0].text.as_str());
                let remote = fields[1].text.to_string();
                let op = if *upload {
                    Operation::Upload { local, remote }
                } else {
                    Operation::Download { remote, local }
                };
                windshell_core::command(
                    &self.store.hosts[self.selected],
                    &op,
                    self.ssh_config.as_deref(),
                )?;
                Ok(self.request(op))
            }
            Mode::Delete { .. } => {
                if button == 0 {
                    let mut next = self.store.clone();
                    let host = next.hosts.remove(self.selected);
                    next.save(&self.path)?;
                    self.store = next;
                    self.credentials.remove(&host.name);
                    self.selected = self.selected.min(self.store.hosts.len().saturating_sub(1));
                    self.notice("连接已删除，远端文件不受影响", false);
                }
                self.mode = Mode::Browse;
                Ok(Action::None)
            }
            _ => Ok(Action::None),
        }
    }
    fn activate(&mut self, button: usize) -> Action {
        match self.submit(button) {
            Ok(action) => action,
            Err(e) => {
                self.notice(format!("{e:#}"), true);
                Action::None
            }
        }
    }
    pub fn paste(&mut self, text: Zeroizing<String>) {
        if matches!(self.mode, Mode::Browse) {
            if !self.workspace.files_focused
                && let Some(tab) = self.workspace.tabs.get_mut(self.workspace.active)
                && let Content::Terminal(session) = &mut tab.content
            {
                let r = session.paste(&text);
                self.workspace_result(r);
            }
            return;
        }
        match &mut self.mode {
            Mode::Edit(form) => {
                let multiline = form.focus == 5 && form.method == Authentication::PrivateKey;
                if let Some(input) = form.input() {
                    input.insert(&text, multiline);
                }
            }
            Mode::Auth { form, .. } if form.focus < form.count() => {
                let multiline = form.focus == 0 && form.method == Authentication::PrivateKey;
                form.input(form.focus).insert(&text, multiline);
            }
            Mode::Transfer { fields, focus, .. } if *focus < 2 => {
                fields[*focus].insert(&text, false)
            }
            _ => {}
        }
    }
    pub fn click(&mut self, target: Target) -> Action {
        match target {
            Target::Manage => self.mode = Mode::Manage,
            Target::Settings => self.mode = Mode::Settings,
            Target::ToggleFiles => {
                self.workspace.sidebar = !self.workspace.sidebar;
                self.workspace.files_focused = self.workspace.sidebar;
            }
            Target::ToggleHidden => {
                self.workspace.hidden = !self.workspace.hidden;
                let r = self.workspace.refresh();
                self.workspace_result(r);
            }
            Target::Refresh => {
                let r = self.workspace.refresh();
                self.workspace_result(r);
            }
            Target::NewTerminal => {
                let r = self.workspace.local();
                self.workspace_result(r);
            }
            Target::CloseTab => self.workspace.close(),
            Target::Tab(i) => {
                self.workspace.active = i;
                self.workspace.files_focused = false;
            }
            Target::File(i) => {
                self.workspace.files_focused = true;
                let r = self.workspace.open(i);
                self.workspace_result(r);
            }
            Target::TerminalFocus => self.workspace.files_focused = false,
            Target::Host(index) => {
                self.selected = index;
            }
            Target::Field(index) => match &mut self.mode {
                Mode::Edit(form) => form.focus = index,
                Mode::Auth { form, .. } => form.focus = index,
                Mode::Transfer { focus, .. } => *focus = index,
                _ => {}
            },
            Target::AuthCycle => {
                if let Mode::Edit(form) = &mut self.mode {
                    form.focus = 4;
                    form.method = form.method.next();
                    form.credentials.method = form.method;
                }
            }
            Target::Button(button) => return self.activate(button),
            Target::New => self.edit_host(None),
            Target::Connect if !self.store.hosts.is_empty() => {
                return self.request(Operation::Shell);
            }
            Target::Edit if !self.store.hosts.is_empty() => self.edit_host(Some(self.selected)),
            Target::Delete if !self.store.hosts.is_empty() => {
                self.mode = Mode::Delete { focus: 1 };
                self.notice("只删除本地连接配置，不操作远端文件。", false);
            }
            Target::Upload | Target::Download if !self.store.hosts.is_empty() => {
                self.mode = Mode::Transfer {
                    upload: matches!(target, Target::Upload),
                    fields: Default::default(),
                    focus: 0,
                };
                self.notice(
                    "单文件传输；目标已存在时会覆盖。路径无需额外添加引号。",
                    false,
                );
            }
            _ => {}
        }
        Action::None
    }
    pub fn key(&mut self, mut key: KeyEvent) -> Action {
        if key.modifiers.contains(KeyModifiers::ALT) && key.code == KeyCode::Char('q') {
            return Action::Quit;
        }
        if matches!(self.mode, Mode::Browse) {
            return self.workspace_key(key);
        }
        if matches!(self.mode, Mode::Manage)
            && let KeyCode::Char(c) = key.code
        {
            key.code = KeyCode::Char(c.to_ascii_lowercase());
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Action::Quit;
        }
        if key.code == KeyCode::Esc {
            self.mode = Mode::Browse;
            self.notice("已取消", false);
            return Action::None;
        }
        match &mut self.mode {
            Mode::Browse => unreachable!(),
            Mode::Settings => match key.code {
                KeyCode::Char('b') => return self.click(Target::ToggleFiles),
                KeyCode::Char('h') => return self.click(Target::ToggleHidden),
                KeyCode::Char('r') => return self.click(Target::Refresh),
                KeyCode::Char('s') => self.mode = Mode::Manage,
                _ => {}
            },
            Mode::Manage => match key.code {
                KeyCode::Char('q') => return Action::Quit,
                KeyCode::Down | KeyCode::Char('j') if !self.store.hosts.is_empty() => {
                    self.selected = (self.selected + 1) % self.store.hosts.len()
                }
                KeyCode::Up | KeyCode::Char('k') if !self.store.hosts.is_empty() => {
                    self.selected =
                        (self.selected + self.store.hosts.len() - 1) % self.store.hosts.len()
                }
                KeyCode::Char(',') => return self.click(Target::Settings),
                KeyCode::Char('n') => return self.click(Target::New),
                KeyCode::Char('e') => return self.click(Target::Edit),
                KeyCode::Char('d') => return self.click(Target::Delete),
                KeyCode::Char('u') => return self.click(Target::Upload),
                KeyCode::Char('g') => return self.click(Target::Download),
                KeyCode::Enter => return self.click(Target::Connect),
                _ => {}
            },
            Mode::Edit(form) => {
                let count = form.count();
                if key.code == KeyCode::F(10) {
                    return self.activate(1);
                }
                if key.code == KeyCode::F(5) {
                    return self.activate(0);
                }
                if key.code == KeyCode::F(2) {
                    form.credentials.reveal = !form.credentials.reveal;
                } else if key.code == KeyCode::Enter && form.focus >= count {
                    let b = form.focus - count;
                    return self.activate(b);
                } else if navigate(&mut form.focus, count + 3, key) {
                } else if form.focus == 4
                    && matches!(
                        key.code,
                        KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')
                    )
                {
                    form.method = if key.code == KeyCode::Left {
                        form.method.next().next()
                    } else {
                        form.method.next()
                    };
                    form.credentials.method = form.method;
                } else if let Some(input) = form.input() {
                    input.key(key);
                }
            }
            Mode::Auth { form, .. } => {
                let count = form.count();
                if key.code == KeyCode::F(5) {
                    return self.activate(0);
                }
                if key.code == KeyCode::F(2) {
                    form.reveal = !form.reveal;
                } else if key.code == KeyCode::Enter && form.focus >= count {
                    let b = form.focus - count;
                    return self.activate(b);
                } else if !navigate(&mut form.focus, count + 2, key) && form.focus < count {
                    form.input(form.focus).key(key);
                }
            }
            Mode::Transfer { fields, focus, .. } => {
                if key.code == KeyCode::F(5) {
                    return self.activate(0);
                }
                if key.code == KeyCode::Enter && *focus >= 2 {
                    let b = *focus - 2;
                    return self.activate(b);
                }
                if !navigate(focus, 4, key) && *focus < 2 {
                    fields[*focus].key(key);
                }
            }
            Mode::Delete { focus } => {
                if key.code == KeyCode::Char('y') {
                    return self.activate(0);
                }
                if key.code == KeyCode::Char('n') {
                    return self.activate(1);
                }
                if key.code == KeyCode::Enter {
                    let b = *focus;
                    return self.activate(b);
                }
                if matches!(
                    key.code,
                    KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right
                ) {
                    *focus = 1 - *focus;
                }
            }
        }
        Action::None
    }
}
fn navigate(focus: &mut usize, count: usize, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Tab | KeyCode::Down | KeyCode::Enter => *focus = (*focus + 1) % count,
        KeyCode::BackTab | KeyCode::Up => *focus = (*focus + count - 1) % count,
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(app: &mut App, code: KeyCode) -> Action {
        app.key(KeyEvent::new(code, KeyModifiers::NONE))
    }
    fn password_form(app: &mut App) {
        app.click(Target::New);
        if let Mode::Edit(form) = &mut app.mode {
            form.fields[1] = Input::new("localhost");
            form.method = Authentication::Password;
            form.credentials.method = form.method;
            form.credentials.password = Input::new("test-session-only");
        }
    }
    #[test]
    fn save_password_metadata_and_reprompt_after_restart() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("hosts.toml");
        let mut app = App::new(HostStore::default(), path.clone());
        password_form(&mut app);
        key(&mut app, KeyCode::F(10));
        assert!(matches!(app.mode, Mode::Manage));
        assert_eq!(app.store.hosts[0].name, "localhost");
        assert!(!std::fs::read_to_string(&path)?.contains("test-session-only"));
        assert!(matches!(
            app.click(Target::Connect),
            Action::Run(Operation::Shell)
        ));
        let mut restarted = App::new(HostStore::load(&path)?, path);
        assert!(matches!(restarted.click(Target::Connect), Action::None));
        assert!(matches!(restarted.mode, Mode::Auth { .. }));
        Ok(())
    }
    #[test]
    fn failed_save_keeps_form_and_existing_state() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut app = App::new(HostStore::default(), dir.path().into());
        password_form(&mut app);
        key(&mut app, KeyCode::F(10));
        assert!(matches!(app.mode, Mode::Edit(_)));
        assert!(app.store.hosts.is_empty());
        assert!(app.error);
        Ok(())
    }
    #[test]
    fn pasted_multiline_key_does_not_trigger_navigation_or_submit() {
        let mut app = App::new(HostStore::default(), "unused".into());
        app.click(Target::New);
        if let Mode::Edit(form) = &mut app.mode {
            form.method = Authentication::PrivateKey;
            form.credentials.method = form.method;
            form.focus = 5;
        }
        let marker = "OPENSSH PRIVATE KEY";
        let key = format!("-----BEGIN {marker}-----\r\nTEST-DATA\r\n-----END {marker}-----\r\n");
        app.paste(Zeroizing::new(key.clone()));
        let Mode::Edit(form) = &app.mode else {
            panic!("paste submitted the form")
        };
        assert_eq!(form.focus, 5);
        assert_eq!(
            form.credentials.key.text.as_str(),
            key.replace("\r\n", "\n")
        );
    }
    #[test]
    fn enter_moves_to_next_field_and_delete_defaults_to_cancel() {
        let mut app = App::new(
            HostStore {
                hosts: vec![Host {
                    name: "test".into(),
                    address: "localhost".into(),
                    ..Default::default()
                }],
            },
            "unused".into(),
        );
        app.click(Target::Edit);
        key(&mut app, KeyCode::Enter);
        let Mode::Edit(form) = &app.mode else {
            panic!("unexpected save")
        };
        assert_eq!(form.focus, 1);
        key(&mut app, KeyCode::Esc);
        app.click(Target::Delete);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.store.hosts.len(), 1);
        assert!(matches!(app.mode, Mode::Browse));
    }
}
