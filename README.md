# WindShell

windvim 风格的 Rust 终端工作区。左侧展开当前 SSH 会话的远端文件树，右侧切换本地/SSH 终端和文件预览标签；设置与 SSH 管理在浮动面板中完成。

## 启动

需要 Rust stable 和 PATH 中的 **OpenSSH 9+**（`ssh`、`scp`）。在支持 UTF-8、ANSI 的交互终端中运行：

```sh
cargo run -p windshell --release
# 独立连接列表 / 独立 OpenSSH 配置
cargo run -p windshell -- --config ./hosts.toml --ssh-config ./ssh_config
# 仅检查连接列表，不打开界面
cargo run -p windshell -- --config ./hosts.toml --check
```

Linux 默认配置为 `~/.config/windshell/hosts.toml`，macOS 为 `~/Library/Application Support/windshell/hosts.toml`，Windows 为 `%APPDATA%\windshell\config\hosts.toml`。具体位置遵循 directories 的平台规则。

## 创建连接

按 Alt+S 或点击顶部 SSH 打开管理面板，再点击「新增连接」或按 N。填写主机（IP、域名或 SSH 别名），名称可空，用户名和端口留空时继承 OpenSSH 设置。

登录方式可以选择：

- **SSH 配置 / Agent**：使用已有 SSH config、系统密钥和 Agent。
- **密码**：在遮挡的密码框中输入登录密码。
- **粘贴私钥**：整段粘贴私钥，包含 BEGIN / END，无需填写私钥路径。加密私钥可填写口令，也可以在连接时按提示输入。

Tab 切换字段，在登录方式上用 ← / → 或空格切换。使用终端的粘贴菜单/快捷键（通常 Ctrl+Shift+V）粘贴多行私钥，终端须支持 bracketed paste。私钥内容始终隐藏，只显示行数。F2 可切换密码/口令显隐。

「保存并连接」或 F5 开始连接，「保存」或 F10 只保存配置。也可先不填凭据，等连接时再输入。**密码、粘贴私钥和口令只保留在本次运行中，不写入连接配置；重启后重新输入。**

首次连接会在 TUI 中展示主机指纹，默认取消；核对后明确选择「信任并继续」。主机密钥变化仍由 OpenSSH 拒绝。应用不绕过主机校验。

## 操作

| 按键 | 操作 |
| --- | --- |
| Alt+B / 点击 WindShell | 展开/收起文件栏并聚焦 |
| Alt+T / 顶部 + | 新建本地终端标签 |
| Alt+← / Alt+→ | 切换终端或文件标签 |
| Alt+W / 顶部 × | 关闭当前标签（终止对应会话） |
| Alt+S / Alt+, | SSH 管理 / 设置面板 |
| Alt+Q | 退出工作区 |
| Shift+PageUp/PageDown / 滚轮 | 滚动终端历史或文件预览 |
| ↑↓ / J K（文件栏） | 选择文件；Enter/→ 展开或打开，← 收起，R 刷新 |
| ↑↓ / J K（SSH 面板） | 选择主机 |
| N / E / D | 新建 / 编辑 / 删除连接 |
| Enter（主机列表） | 连接 |
| U / G | 上传 / 下载文件 |
| Tab / Shift+Tab / ↑↓（表单） | 切换字段或按钮 |
| Enter（表单字段） | 前进到下一项；聚焦按钮时执行操作 |
| ←→ / Home / End / Delete / Backspace | 编辑文本 |
| Ctrl+U | 清空当前字段 |
| F5 | 保存并连接 / 继续认证 / 开始传输 |
| F10（连接表单） | 只保存 |
| F2 | 显隐密码/口令 |
| Esc | 取消并返回 |
| Q（SSH 面板） | 退出 |

支持鼠标点击主机、字段、认证方式和操作按钮。48×18 起可用；紧凑终端中的表单随焦点滚动。

启动即打开本地 shell。SSH 连接打开独立 PTY 标签，切换标签时后台继续运行。Ctrl+C、Tab、Esc 直接传给当前终端；远端 `exit` 后保留末屏，Alt+W 关闭。行首输入 OpenSSH 的 `~.` 可断开无响应会话。

文件栏只显示当前 SSH 会话的远端文件，连接后从远端登录目录开始；切换 SSH 或文件标签会切换到所属会话的目录树。本地终端标签显示连接提示，不再浏览本地文件。目录点击展开/收起，文件点击在右侧只读预览（UTF-8、最大 1 MiB），根据扩展名或 shebang 自动语法高亮，未知类型按纯文本显示。重复打开同一会话的文件会回到已有标签。文件栏中 Backspace 返回上级目录，R 刷新。

远端浏览通过 SFTP，复用当前 OpenSSH 认证连接，每次操作后关闭文件通道，不往终端注入命令；服务器需要启用 SFTP。连接断开、权限不足、二进制和大文件会提示错误。当前远端浏览依赖 Unix OpenSSH 的 ControlMaster，Linux 已实测；Windows 远端浏览暂不支持，macOS 未实测。设置面板可切换文件栏和隐藏文件，本次运行有效。暂不提供编辑保存或终端内鼠标协议透传。

## 文件传输

在 SSH 管理面板中，U 上传，G 下载。输入本地与远端路径后，点击「开始传输」或按 F5。复用当前连接的认证方式，进度由前台 OpenSSH 显示。

路径无需额外加引号，空格按原文处理；远端路径相对于远端登录目录。暂不支持目录递归、通配符、远端反斜杠或 `~` 前缀。**目标已存在时会覆盖**。Ctrl+C 可取消传输，可能留下部分文件。远端文件栏用于预览，传输仍通过路径表单。

## 配置与凭据

```toml
[[hosts]]
name = "开发服务器"
address = "dev"
user = "deploy"
port = 22
# auto / password / private_key；仅记录认证方式，不记录凭据
authentication = "password"
```

旧版 `identity` 配置仍由 Auto 模式使用，界面不再要求填写密钥路径。用户 SSH 配置应可信。

为了让系统 OpenSSH 使用粘贴私钥，运行期间会创建私有临时文件（Unix 0600），会话/传输正常结束、取消或普通错误后删除。密码和口令不写临时文件。强制杀死进程或系统崩溃可能留下临时私钥文件；本版本不提供持久凭据库。

## 开发与验证

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build -p windshell
# 可选：真实 sshd 回归，需 tmux、sshd、ssh-keygen
python3 tests/workspace_smoke.py
python3 tests/remote_smoke.py
python3 tests/ssh_smoke.py
# 可选：密码/私钥/指纹交互，需 tmux、ssh-keygen
# 在测试用 Python 虚拟环境中安装 tests/requirements.txt 后运行
python3 tests/auth_smoke.py
```

- `windshell-core`：配置、运行时凭据、OpenSSH 操作及认证请求回调，无 TUI 依赖。
- `windshell`：Ratatui/Crossterm 界面、输入编辑和终端生命周期。
- [规格入口](spec/README.md)与[当前工作区验收](spec/06-modules/workspace/spec.md)：实际验证范围和限制。

支持多标签会话，暂无后台传输队列或桌面端。CI 配置覆盖 Linux/macOS/Windows，实际执行结果以验收记录为准；跨平台运行时认证仍需各平台实测。
