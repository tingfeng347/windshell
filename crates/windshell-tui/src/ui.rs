use crate::{
    app::{App, CredentialForm, Mode, Target, resume, suspend},
    input::Input,
};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, Paragraph, Wrap},
};
use windshell_core::{Authentication, Prompt};
use zeroize::Zeroizing;

pub type Targets = Vec<(Rect, Target)>;

// Catppuccin Mocha. Every surface below is picked from this small, fixed set so
// the chrome (top bar / sidebar / status) reads as one palette instead of ad
// hoc RGB values.
const CRUST: Color = Color::Rgb(17, 17, 27);
const MANTLE: Color = Color::Rgb(24, 24, 37);
const BASE: Color = Color::Rgb(30, 30, 46);
const SURFACE0: Color = Color::Rgb(49, 50, 68);
const SURFACE1: Color = Color::Rgb(69, 71, 90);
const OVERLAY0: Color = Color::Rgb(108, 112, 134);
const OVERLAY1: Color = Color::Rgb(147, 153, 178);
const SUBTEXT0: Color = Color::Rgb(166, 173, 200);
const TEXT: Color = Color::Rgb(205, 214, 244);
const ACCENT: Color = Color::Rgb(137, 180, 250);
const LAVENDER: Color = Color::Rgb(180, 190, 254);
const GREEN: Color = Color::Rgb(166, 227, 161);
const RED: Color = Color::Rgb(243, 139, 168);
const PEACH: Color = Color::Rgb(250, 179, 135);

const MUTED: Color = OVERLAY1;

fn accent() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}
fn muted() -> Style {
    Style::default().fg(OVERLAY1)
}
fn dim_style() -> Style {
    Style::default().fg(OVERLAY0)
}
fn selected_style() -> Style {
    Style::default()
        .bg(SURFACE0)
        .fg(LAVENDER)
        .add_modifier(Modifier::BOLD)
}
fn panel(title: &str) -> Block<'_> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(SURFACE1))
        .title_style(Style::default().fg(OVERLAY1))
        .title(title)
}
/// Padded, focus-aware field/panel frame used by every form surface.
fn field(title: &str, focused: bool) -> Block<'_> {
    let block = panel(&format!(" {title} "));
    if focused {
        block.border_style(accent())
    } else {
        block
    }
}
/// Display width aware padding: CJK names keep the columns aligned.
fn pad(text: &str, width: usize) -> String {
    let used = Line::raw(text).width();
    if used >= width {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(width - used))
    }
}
/// Section rule drawn as a one-cell surface band (no box-drawing glyphs, so it
/// stays a single column in CJK-wide terminals too).
fn rule(frame: &mut Frame, rect: Rect, color: Color) {
    frame.render_widget(Block::default().style(Style::default().bg(color)), rect);
}

pub fn sidebar_width(width: u16) -> u16 {
    if width < 70 {
        20.min(width / 3)
    } else {
        (width / 4).clamp(24, 34)
    }
}
fn color(c: vt100::Color, default: Color) -> Color {
    match c {
        vt100::Color::Default => default,
        vt100::Color::Idx(n) => Color::Indexed(n),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}
pub fn draw(frame: &mut Frame, app: &App) -> Targets {
    use crate::workspace::Content;
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(Color::Black).fg(TEXT)),
        area,
    );
    if area.width < 48 || area.height < 18 {
        frame.render_widget(
            Paragraph::new("WindShell · 请调整到至少 48×18\nAlt+Q 退出"),
            area,
        );
        return vec![];
    }
    let w = &app.workspace;
    let side = if w.sidebar {
        sidebar_width(area.width)
    } else {
        0
    };
    let mut hits = vec![(
        Rect::new(area.x, area.y, side.max(11), 1),
        Target::ToggleFiles,
    )];
    let body = Rect::new(
        area.x + side,
        area.y + 1,
        area.width - side,
        area.height - 3,
    );
    frame.render_widget(
        Paragraph::new(" WindShell").style(accent()),
        Rect::new(area.x, area.y, side.max(11), 1),
    );
    if w.sidebar {
        let rect = Rect::new(area.x, area.y + 1, side, area.height - 3);
        frame.render_widget(
            Block::default()
                .borders(ratatui::widgets::Borders::RIGHT)
                .border_style(Style::default().fg(MUTED)),
            rect,
        );
        frame.render_widget(
            Paragraph::new(format!(
                " {}",
                w.root.file_name().unwrap_or_default().to_string_lossy()
            ))
            .style(accent()),
            Rect::new(rect.x, rect.y, side - 1, 1),
        );
        frame.render_widget(
            Paragraph::new(w.remote_status())
                .style(Style::default().fg(MUTED))
                .wrap(Wrap { trim: false }),
            Rect::new(rect.x, rect.y + 1, side - 1, 2),
        );
        let height = rect.height.saturating_sub(3) as usize;
        let offset = w.selected.saturating_add(1).saturating_sub(height);
        for (row, (i, entry)) in w
            .entries
            .iter()
            .enumerate()
            .skip(offset)
            .take(height)
            .enumerate()
        {
            let r = Rect::new(rect.x, rect.y + 3 + row as u16, side - 1, 1);
            let label = format!(
                " {}{} {}",
                "  ".repeat(entry.depth),
                if entry.directory {
                    if entry.expanded { "▾" } else { "▸" }
                } else {
                    "·"
                },
                entry.path.file_name().unwrap_or_default().to_string_lossy()
            );
            let style = if i == w.selected && w.files_focused {
                Style::default().bg(Color::Rgb(49, 50, 68)).fg(ACCENT)
            } else {
                Style::default().fg(if entry.directory { ACCENT } else { TEXT })
            };
            frame.render_widget(Paragraph::new(label).style(style), r);
            hits.push((r, Target::File(i)));
        }
    }
    let start = area.x + side.max(if w.sidebar { 0 } else { 11 });
    let controls = 12u16;
    let available = area.right().saturating_sub(start + controls);
    let tab_width = 20u16.min(available).max(1);
    let count = (available / tab_width).max(1) as usize;
    let offset = w.active.saturating_add(1).saturating_sub(count);
    for (col, (i, tab)) in w
        .tabs
        .iter()
        .enumerate()
        .skip(offset)
        .take(count)
        .enumerate()
    {
        let r = Rect::new(start + col as u16 * tab_width, area.y, tab_width, 1);
        let label = format!(
            " {} {}",
            if matches!(tab.content, Content::File { .. }) {
                "·"
            } else {
                ">"
            },
            tab.title
        );
        frame.render_widget(
            Paragraph::new(label).style(if i == w.active {
                accent()
            } else {
                Style::default().fg(MUTED)
            }),
            r,
        );
        hits.push((r, Target::Tab(i)));
    }
    for (i, (label, target)) in [
        (" + ", Target::NewTerminal),
        (" × ", Target::CloseTab),
        ("SSH", Target::Manage),
        (" ⚙ ", Target::Settings),
    ]
    .into_iter()
    .enumerate()
    {
        let r = Rect::new(area.right() - controls + i as u16 * 3, area.y, 3, 1);
        frame.render_widget(Paragraph::new(label).style(accent()), r);
        hits.push((r, target));
    }
    hits.push((body, Target::TerminalFocus));
    match w.tabs.get(w.active).map(|t| &t.content) {
        Some(Content::Terminal(session)) => render_session(frame, body, session, matches!(app.mode, Mode::Browse) && !w.files_focused),
        Some(Content::File {lines,scroll,..}) => {
            let lines:Vec<Line>=lines.iter().enumerate().skip(*scroll as usize).take(body.height as usize).map(|(n,line)| {let mut line=line.clone();line.spans.insert(0,ratatui::text::Span::styled(format!("{:>4}  ",n+1),Style::default().fg(MUTED)));line}).collect();
            frame.render_widget(Paragraph::new(lines),body);
        }
        None => frame.render_widget(Paragraph::new("\n  WindShell\n\n  Alt+T  打开本地终端\n  Alt+S  连接 SSH\n  Alt+B  展开文件栏\n\n  从左侧打开文件，在这里预览。").style(Style::default().fg(MUTED)),body),
    }
    let status = Rect::new(area.x, area.bottom() - 2, area.width, 1);
    let location = match w.tabs.get(w.active).map(|t| &t.content) {
        Some(Content::File { path, .. }) => format!("只读  {}", path.display()),
        _ => format!(
            "{}  {}",
            if w.files_focused { "文件" } else { "终端" },
            w.root.display()
        ),
    };
    frame.render_widget(
        Paragraph::new(format!(" {}", location))
            .style(Style::default().bg(Color::Rgb(30, 30, 46)).fg(ACCENT)),
        status,
    );
    let hint = if app.error {
        app.message.clone()
    } else {
        " Alt+B 文件栏  Alt+T 新终端  Alt+←/→ 标签  Alt+S SSH  Alt+, 设置  Alt+Q 退出".into()
    };
    frame.render_widget(
        Paragraph::new(hint).style(Style::default().fg(if app.error {
            Color::Rgb(243, 139, 168)
        } else {
            MUTED
        })),
        Rect::new(area.x, area.bottom() - 1, area.width, 1),
    );
    if !matches!(app.mode, Mode::Browse) {
        if area.width < 56 || area.height < 24 {
            frame.render_widget(Clear, area);
            return draw_panel(frame, app);
        }
        // Render existing form widgets into a bounded surface, then translate its
        // cursor and hit targets. No background target can receive modal clicks.
        let rect = centered(
            area,
            (area.width.saturating_sub(6)).min(110),
            (area.height.saturating_sub(4)).max(18),
        );
        frame.render_widget(Clear, rect);
        frame.render_widget(
            panel(" WindShell  ·  SSH / 设置                         Esc 关闭 ")
                .style(Style::default().bg(Color::Black).fg(TEXT))
                .border_style(Style::default().fg(MUTED)),
            rect,
        );
        let inner = panel("").inner(rect);
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(
            inner.width,
            inner.height,
        ))
        .expect("memory terminal");
        let mut inner_hits = vec![];
        let rendered = terminal
            .draw(|f| {
                f.render_widget(
                    Block::default().style(Style::default().bg(Color::Black).fg(TEXT)),
                    f.area(),
                );
                inner_hits = draw_panel(f, app);
            })
            .expect("memory draw");
        for y in 0..inner.height {
            for x in 0..inner.width {
                frame.buffer_mut()[(inner.x + x, inner.y + y)] = rendered.buffer[(x, y)].clone();
            }
        }
        {
            let cursor = terminal.get_cursor_position().unwrap();
            frame.set_cursor_position((inner.x + cursor.x, inner.y + cursor.y));
        }
        hits = inner_hits
            .into_iter()
            .map(|(mut r, t)| {
                r.x += inner.x;
                r.y += inner.y;
                (r, t)
            })
            .collect();
    }
    hits
}

fn render_session(
    frame: &mut Frame,
    body: Rect,
    session: &crate::workspace::Session,
    focused: bool,
) {
    let screen = session.parser.screen();
    for row in 0..body.height {
        for col in 0..body.width {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            let mut style = Style::default()
                .fg(color(cell.fgcolor(), TEXT))
                .bg(color(cell.bgcolor(), Color::Black));
            for (enabled, modifier) in [
                (cell.bold(), Modifier::BOLD),
                (cell.italic(), Modifier::ITALIC),
                (cell.underline(), Modifier::UNDERLINED),
                (cell.inverse(), Modifier::REVERSED),
            ] {
                if enabled {
                    style = style.add_modifier(modifier);
                }
            }
            let symbol = cell.contents();
            frame.buffer_mut()[(body.x + col, body.y + row)]
                .set_symbol(if symbol.is_empty() { " " } else { symbol })
                .set_style(style);
        }
    }
    let (row, col) = screen.cursor_position();
    if focused
        && screen.scrollback() == 0
        && !screen.hide_cursor()
        && row < body.height
        && col < body.width
    {
        frame.set_cursor_position((body.x + col, body.y + row));
    }
    if session.exited {
        frame.render_widget(
            Paragraph::new(" 会话已结束 · Alt+W 关闭 · Alt+T 新建")
                .style(Style::default().fg(MUTED)),
            Rect::new(body.x, body.bottom() - 1, body.width, 1),
        );
    }
}

fn draw_panel(frame: &mut Frame, app: &App) -> Targets {
    let area = frame.area();
    let mut hits = Vec::new();
    if area.width < 48 || area.height < 18 {
        frame.render_widget(
            Paragraph::new("WindShell\n请调整到至少 48 列 × 18 行\nEsc 返回 · Ctrl+C 退出")
                .wrap(Wrap { trim: false }),
            area,
        );
        return hits;
    }
    match &app.mode {
        Mode::Browse | Mode::Manage | Mode::Delete { .. } => browse(frame, app, &mut hits),
        Mode::Settings => {
            heading(frame, "设置", "工作区偏好 · 本次运行生效");
            let rows = [
                (
                    format!(
                        "[{}] 文件栏   B",
                        if app.workspace.sidebar { "开" } else { "关" }
                    ),
                    Target::ToggleFiles,
                ),
                (
                    format!(
                        "[{}] 显示隐藏文件   H",
                        if app.workspace.hidden { "开" } else { "关" }
                    ),
                    Target::ToggleHidden,
                ),
                ("SSH 连接管理   S".into(), Target::Manage),
                ("刷新文件栏 R".into(), Target::Refresh),
            ];
            for (i, (label, target)) in rows.into_iter().enumerate() {
                let r = Rect::new(area.x + 2, area.y + 4 + i as u16 * 2, area.width - 4, 1);
                frame.render_widget(Paragraph::new(label).style(accent()), r);
                hits.push((r, target));
            }
            frame.render_widget(Paragraph::new("Alt+B 文件栏 · Alt+T 新终端 · Alt+W 关闭标签\nAlt+←/→ 切换标签 · Alt+S SSH · Alt+, 设置\nAlt+Q 退出 · Esc 关闭面板\n\n远端文件：只读 UTF-8 高亮预览，最大 1 MiB\n终端：Ctrl+C、Tab、Esc 均传递给当前会话").wrap(Wrap {trim:false}),Rect::new(area.x+2,area.y+11,area.width-4,area.height.saturating_sub(12)));
        }
        Mode::Edit(form) => {
            heading(
                frame,
                if form.index.is_some() {
                    "编辑连接"
                } else {
                    "新建连接"
                },
                "主机信息 / 登录认证",
            );
            let mut groups = if area.width >= 78 {
                vec![vec![0, 1], vec![2, 3], vec![4]]
            } else {
                vec![vec![0], vec![1], vec![2, 3], vec![4]]
            };
            if form.method != Authentication::Auto {
                groups.push(vec![5]);
            }
            if form.method == Authentication::PrivateKey {
                groups.push(vec![6]);
            }
            let body = body(area);
            let focus_group = groups
                .iter()
                .position(|g| g.contains(&form.focus))
                .unwrap_or(groups.len() - 1);
            let offset = ((focus_group + 1) as u16 * 3).saturating_sub(body.height);
            for (group, indices) in groups.iter().enumerate() {
                let top = group as i32 * 3 - i32::from(offset);
                if top < 0 || top + 3 > i32::from(body.height) {
                    continue;
                }
                let row = Rect::new(body.x, body.y + top as u16, body.width, 3);
                let cells = Layout::horizontal(vec![
                    Constraint::Ratio(1, indices.len() as u32);
                    indices.len()
                ])
                .split(row);
                for (column, &index) in indices.iter().enumerate() {
                    let rect = cells[column];
                    if index < 4 {
                        input(
                            frame,
                            rect,
                            [
                                "名称（可选）",
                                "主机 / SSH 别名",
                                "用户名（可空）",
                                "端口（可空）",
                            ][index],
                            &form.fields[index],
                            [
                                "默认使用主机名",
                                "例如 192.168.1.10 或 prod",
                                "SSH 默认用户",
                                "SSH 默认端口",
                            ][index],
                            form.focus == index,
                            false,
                        );
                        hits.push((rect, Target::Field(index)));
                    } else if index == 4 {
                        let selected = form.focus == 4;
                        let text = format!("  {}   ← / → 切换", form.method.label());
                        frame.render_widget(
                            Paragraph::new(text)
                                .style(if selected { accent() } else { Style::default() })
                                .block(panel(" 登录方式 ").border_style(if selected {
                                    accent()
                                } else {
                                    Style::default()
                                })),
                            rect,
                        );
                        hits.push((rect, Target::AuthCycle));
                    } else {
                        credential(
                            frame,
                            rect,
                            &form.credentials,
                            index - 5,
                            form.focus == index,
                        );
                        hits.push((rect, Target::Field(index)));
                    }
                }
            }
            footer(
                frame,
                app,
                &["保存并连接 F5", "保存 F10", "取消 Esc"],
                form.focus.checked_sub(form.count()),
                &mut hits,
            );
        }
        Mode::Auth { form, .. } => {
            heading(
                frame,
                "登录认证",
                &format!(
                    "{} · {}",
                    app.store.hosts[app.selected].name,
                    form.method.label()
                ),
            );
            let body = body(area);
            for i in 0..form.count() {
                let row = Rect::new(body.x, body.y + i as u16 * 3, body.width, 3);
                credential(frame, row, form, i, form.focus == i);
                hits.push((row, Target::Field(i)));
            }
            let hint = Rect::new(body.x, body.y + form.count() as u16 * 3 + 1, body.width, 2);
            frame.render_widget(
                Paragraph::new(if form.method == Authentication::PrivateKey {
                    "直接粘贴完整私钥，包含 BEGIN 和 END。加密私钥可填写口令。"
                } else {
                    "输入 SSH 登录密码。关闭 WindShell 后，下次连接会重新询问。"
                })
                .wrap(Wrap { trim: false }),
                hint,
            );
            footer(
                frame,
                app,
                &["继续连接 F5", "取消 Esc"],
                form.focus.checked_sub(form.count()),
                &mut hits,
            );
        }
        Mode::Transfer {
            upload,
            fields,
            focus,
        } => {
            heading(
                frame,
                if *upload {
                    "上传文件"
                } else {
                    "下载文件"
                },
                &app.store.hosts[app.selected].name,
            );
            let body = body(area);
            for i in 0..2 {
                let rect = Rect::new(body.x, body.y + i * 3, body.width, 3);
                input(
                    frame,
                    rect,
                    if i == 0 {
                        "本地路径"
                    } else {
                        "远端路径"
                    },
                    &fields[i as usize],
                    if i == 0 {
                        "输入本地文件或目录的完整路径"
                    } else {
                        "输入远端文件或目录，不额外加引号"
                    },
                    *focus == i as usize,
                    false,
                );
                hits.push((rect, Target::Field(i as usize)));
            }
            frame.render_widget(
                Paragraph::new("目标已存在时会覆盖。暂不支持目录递归、通配符或 ~ 前缀。")
                    .wrap(Wrap { trim: false }),
                Rect::new(body.x, body.y + 7, body.width, 2),
            );
            footer(
                frame,
                app,
                &["开始传输 F5", "取消 Esc"],
                focus.checked_sub(2),
                &mut hits,
            );
        }
    }
    if let Mode::Delete { focus } = &app.mode {
        // Protected confirmation: cancel has initial focus.
        hits.clear();
        let rect = centered(area, 64, 10);
        frame.render_widget(Clear, rect);
        frame.render_widget(panel(" 删除连接 "), rect);
        frame.render_widget(
            Paragraph::new(format!(
                "删除「{}」？\n只删除本地连接配置，远端文件不受影响。",
                app.store.hosts[app.selected].name
            ))
            .wrap(Wrap { trim: false }),
            Rect::new(rect.x + 2, rect.y + 2, rect.width - 4, 3),
        );
        buttons(
            frame,
            Rect::new(rect.x + 2, rect.y + 6, rect.width - 4, 3),
            &["删除", "取消"],
            Some(*focus),
            &mut hits,
        );
    }
    hits
}

fn body(area: Rect) -> Rect {
    Rect::new(
        area.x + 2,
        area.y + 3,
        area.width.saturating_sub(4),
        area.height.saturating_sub(9),
    )
}
fn heading(frame: &mut Frame, title: &str, subtitle: &str) {
    let area = frame.area();
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(format!("  WindShell / {title}"), accent()),
            Line::raw(format!("  {subtitle}")),
        ]),
        Rect::new(area.x, area.y, area.width, 2),
    );
}
fn footer(frame: &mut Frame, app: &App, labels: &[&str], focus: Option<usize>, hits: &mut Targets) {
    let area = frame.area();
    frame.render_widget(
        Paragraph::new(&*app.message)
            .style(if app.error {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            })
            .wrap(Wrap { trim: false }),
        Rect::new(area.x + 2, area.bottom() - 6, area.width - 4, 2),
    );
    buttons(
        frame,
        Rect::new(area.x + 2, area.bottom() - 4, area.width - 4, 3),
        labels,
        focus,
        hits,
    );
    frame.render_widget(
        Paragraph::new(" Tab 切换 · Enter 下一项/按钮 · F2 显隐密码 · Esc 返回"),
        Rect::new(area.x + 1, area.bottom() - 1, area.width - 2, 1),
    );
}
fn buttons(
    frame: &mut Frame,
    rect: Rect,
    labels: &[&str],
    focus: Option<usize>,
    hits: &mut Targets,
) {
    let cells = Layout::horizontal(vec![
        Constraint::Ratio(1, labels.len() as u32);
        labels.len()
    ])
    .split(rect);
    for (i, label) in labels.iter().enumerate() {
        let focused = focus == Some(i);
        let style = if focused {
            accent().add_modifier(Modifier::REVERSED)
        } else if i == 0 {
            accent()
        } else {
            Style::default()
        };
        frame.render_widget(
            Paragraph::new(*label)
                .centered()
                .style(style)
                .block(panel(if focused { " Enter " } else { "" })),
            cells[i],
        );
        hits.push((cells[i], Target::Button(i)));
    }
}
fn browse(frame: &mut Frame, app: &App, hits: &mut Targets) {
    let area = frame.area();
    heading(frame, "SSH 连接", "选择连接 · 凭据只保留在本次运行");
    let row = Rect::new(2, 3, area.width - 4, 1);
    let cells = Layout::horizontal([
        Constraint::Length(18),
        Constraint::Length(16),
        Constraint::Min(1),
    ])
    .split(row);
    for (i, (label, target)) in [
        ("新增连接 (N)", Target::New),
        ("设置 (,)", Target::Settings),
        ("连接 (Enter)", Target::Connect),
    ]
    .into_iter()
    .enumerate()
    {
        frame.render_widget(Paragraph::new(label).style(accent()), cells[i]);
        hits.push((cells[i], target));
    }
    let height = area.height.saturating_sub(12) as usize;
    let offset = app.selected.saturating_add(1).saturating_sub(height);
    if app.store.hosts.is_empty() {
        frame.render_widget(
            Paragraph::new("还没有连接\n\n按 N 添加 SSH，支持密码、私钥和 SSH Agent。"),
            Rect::new(2, 5, area.width - 4, height as u16),
        );
    }
    for (row, (i, host)) in app
        .store
        .hosts
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .enumerate()
    {
        let rect = Rect::new(2, 5 + row as u16, area.width - 4, 1);
        frame.render_widget(
            Paragraph::new(format!(
                "{} {:<18}  {}{}{}  ·  {}",
                if i == app.selected { ">" } else { " " },
                host.name,
                host.user,
                if host.user.is_empty() { "" } else { "@" },
                host.address,
                host.authentication.label()
            ))
            .style(if i == app.selected {
                accent().bg(Color::Rgb(49, 50, 68))
            } else {
                Style::default()
            }),
            rect,
        );
        hits.push((rect, Target::Host(i)));
    }
    if let Some(host) = app.store.hosts.get(app.selected) {
        frame.render_widget(
            Paragraph::new(format!(
                "连接详情 · {} · 端口 {}",
                host.address,
                host.port
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "SSH 默认".into())
            ))
            .style(Style::default().fg(MUTED)),
            Rect::new(2, area.height - 6, area.width - 4, 1),
        );
    }
    let actions = [
        ("编辑 E", Target::Edit),
        ("删除 D", Target::Delete),
        ("上传 U", Target::Upload),
        ("下载 G", Target::Download),
    ];
    let cells = Layout::horizontal([Constraint::Ratio(1, 4); 4]).split(Rect::new(
        2,
        area.height - 4,
        area.width - 4,
        1,
    ));
    for (i, (label, target)) in actions.into_iter().enumerate() {
        frame.render_widget(Paragraph::new(label).style(accent()), cells[i]);
        hits.push((cells[i], target));
    }
    frame.render_widget(
        Paragraph::new(app.message.as_str()).style(Style::default().fg(if app.error {
            Color::Red
        } else {
            MUTED
        })),
        Rect::new(2, area.height - 2, area.width - 4, 1),
    );
}

fn credential(frame: &mut Frame, rect: Rect, form: &CredentialForm, index: usize, focused: bool) {
    if form.method == Authentication::PrivateKey && index == 0 {
        let text = if form.key.text.is_empty() {
            "粘贴包含 BEGIN / END 的完整私钥".into()
        } else {
            format!(
                "已输入 {} 行私钥 · 内容已隐藏",
                form.key.text.lines().count()
            )
        };
        frame.render_widget(
            Paragraph::new(text).block(
                panel(" 私钥内容（直接粘贴，不填路径） ").border_style(if focused {
                    accent()
                } else {
                    Style::default()
                }),
            ),
            rect,
        );
    } else {
        let (label, value, hint) = if form.method == Authentication::PrivateKey {
            ("私钥口令（可选）", &form.passphrase, "未加密的私钥可留空")
        } else {
            ("登录密码", &form.password, "输入密码；仅在本次运行中保留")
        };
        input(frame, rect, label, value, hint, focused, !form.reveal);
    }
}
fn input(
    frame: &mut Frame,
    rect: Rect,
    label: &str,
    input: &Input,
    hint: &str,
    focused: bool,
    hidden: bool,
) {
    let block = panel(label).border_style(if focused { accent() } else { Style::default() });
    let inner = block.inner(rect);
    let text = if input.text.is_empty() {
        hint.to_string()
    } else if hidden {
        "*".repeat(input.text.chars().count())
    } else {
        input.text.to_string()
    };
    let cursor = if hidden {
        input.text[..input.cursor].chars().count()
    } else {
        Line::raw(&input.text[..input.cursor]).width()
    };
    let offset = if focused && !input.text.is_empty() {
        cursor
            .saturating_add(1)
            .saturating_sub(inner.width as usize)
    } else {
        0
    };
    frame.render_widget(
        Paragraph::new(text)
            .scroll((0, offset as u16))
            .style(if input.text.is_empty() {
                Style::default().add_modifier(Modifier::DIM)
            } else {
                Style::default()
            })
            .block(block),
        rect,
    );
    if focused {
        frame.set_cursor_position((
            inner.x
                + cursor
                    .saturating_sub(offset)
                    .min(inner.width.saturating_sub(1) as usize) as u16,
            inner.y,
        ));
    }
}
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

/// Authentication owns the TUI only while OpenSSH's askpass helper is blocked.
/// No event reader competes with the remote shell once authentication finishes.
pub fn prompt(
    terminal: &mut DefaultTerminal,
    host: &str,
    prompt: &Prompt,
) -> Result<Option<Zeroizing<String>>> {
    resume(terminal)?;
    let result = (|| {
        let mut value = Input::default();
        let mut focus = if prompt.confirm { 1 } else { 0 };
        let text: String = prompt
            .text
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .collect();
        let mut scroll = 0u16;
        loop {
            let mut hits = Vec::new();
            terminal.draw(|f| {
                let a = f.area();
                if a.width < 48 || a.height < 18 {
                    f.render_widget(Paragraph::new("请调整到 48×18 以上；Esc 取消认证"), a);
                    return;
                }
                heading(
                    f,
                    if prompt.confirm {
                        "验证主机指纹"
                    } else {
                        "SSH 认证"
                    },
                    host,
                );
                let bottom = a.bottom() - 6;
                let height = bottom.saturating_sub(a.y + 4 + if prompt.confirm { 0 } else { 3 });
                f.render_widget(
                    Paragraph::new(text.as_str())
                        .wrap(Wrap { trim: false })
                        .scroll((scroll, 0))
                        .block(panel(" OpenSSH 请求 ")),
                    Rect::new(a.x + 2, a.y + 3, a.width - 4, height),
                );
                if !prompt.confirm {
                    let rect = Rect::new(a.x + 2, bottom - 3, a.width - 4, 3);
                    input(
                        f,
                        rect,
                        "认证内容",
                        &value,
                        "输入后点击继续，或按 F5",
                        focus == 0,
                        true,
                    );
                    hits.push((rect, Target::Field(0)));
                }
                f.render_widget(
                    Paragraph::new(if prompt.confirm {
                        "请核对上方指纹。信任后 OpenSSH 将记录此主机。"
                    } else {
                        "输入内容不会显示，也不会写入连接配置。"
                    })
                    .wrap(Wrap { trim: false }),
                    Rect::new(a.x + 2, bottom, a.width - 4, 2),
                );
                buttons(
                    f,
                    Rect::new(a.x + 2, a.bottom() - 4, a.width - 4, 3),
                    if prompt.confirm {
                        &["信任并继续", "取消连接"]
                    } else {
                        &["继续 F5", "取消 Esc"]
                    },
                    if prompt.confirm {
                        Some(focus)
                    } else {
                        focus.checked_sub(1)
                    },
                    &mut hits,
                );
                f.render_widget(
                    Paragraph::new(" Tab 切换 · Enter 确认 · PageUp/PageDown 查看请求 · Esc 取消"),
                    Rect::new(a.x + 1, a.bottom() - 1, a.width - 2, 1),
                );
            })?;
            let mut submit = None;
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Esc => return Ok(None),
                    KeyCode::Char('c')
                        if k.modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL) =>
                    {
                        return Ok(None);
                    }
                    KeyCode::PageDown => scroll = scroll.saturating_add(3),
                    KeyCode::PageUp => scroll = scroll.saturating_sub(3),
                    KeyCode::Tab | KeyCode::BackTab => {
                        focus = (focus + 1) % if prompt.confirm { 2 } else { 3 }
                    }
                    KeyCode::Enter if prompt.confirm => submit = Some(focus),
                    KeyCode::Enter if focus > 0 => submit = Some(focus - 1),
                    KeyCode::Enter => focus = 1,
                    KeyCode::F(5) if !prompt.confirm => submit = Some(0),
                    _ if !prompt.confirm && focus == 0 => value.key(k),
                    _ => {}
                },
                Event::Paste(text) if !prompt.confirm && focus == 0 => {
                    let text = Zeroizing::new(text);
                    value.insert(&text, false);
                }
                Event::Mouse(m) if m.kind == MouseEventKind::Down(MouseButton::Left) => {
                    if let Some((_, target)) = hits
                        .iter()
                        .rev()
                        .find(|(r, _)| r.contains((m.column, m.row).into()))
                    {
                        match target {
                            Target::Button(i) => submit = Some(*i),
                            Target::Field(_) => focus = 0,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            if let Some(button) = submit {
                if button == 1 {
                    return Ok(None);
                }
                return Ok(Some(if prompt.confirm {
                    Zeroizing::new("yes".into())
                } else {
                    value.text
                }));
            }
        }
    })();
    let cleanup = suspend();
    match result {
        Ok(value) => {
            cleanup?;
            Ok(value)
        }
        Err(e) => {
            let _ = cleanup;
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use windshell_core::{Host, HostStore};
    #[test]
    fn secrets_are_masked_and_focused_fields_fit_compact_terminal() -> Result<()> {
        let host = Host {
            name: "主机".into(),
            address: "localhost".into(),
            ..Default::default()
        };
        let mut app = App::new(HostStore { hosts: vec![host] }, "unused".into());
        app.click(Target::Edit);
        if let Mode::Edit(form) = &mut app.mode {
            form.method = Authentication::PrivateKey;
            form.credentials.method = form.method;
            form.credentials.key = Input::new("PRIVATE_KEY_CONTENT\nSECOND_LINE");
            form.credentials.passphrase = Input::new("PASSPHRASE_CONTENT");
            form.focus = 6;
        }
        for (width, height) in [(100, 30), (80, 24), (48, 18), (20, 5), (0, 0)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            terminal.draw(|f| {
                draw(f, &app);
            })?;
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect();
            assert!(!text.contains("PRIVATE_KEY_CONTENT"));
            assert!(!text.contains("PASSPHRASE_CONTENT"));
            if width >= 48 {
                assert!(text.replace(' ', "").contains("私钥口令"));
                assert!(text.replace(' ', "").contains("保存并连接"));
            }
        }
        Ok(())
    }
}
