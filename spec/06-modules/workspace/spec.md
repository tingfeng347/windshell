# Windvim 风格工作区

2026-10-28。**以下首轮交付为历史基线，当前文件栏与预览行为以文末「远端目录与代码高亮」为准。**

取代旧版单会话主机首页；此前 auth/mvp 的界面与单会话描述保留为历史验收，不代表当前行为。任务开始无 Git 仓库，修改前代码/规格已备份到 `.audit/before-workspace/`。

## 目标与边界

- 沿用本地 windvim 的 Catppuccin Mocha 前景色、黑色背景、顶部标签、左侧文件树和 Lazy 风格浮层。保留 Rust TUI。
- 启动本地终端；本地和 SSH 会话各自拥有 PTY，切换标签不退出进程。文件也占用右侧标签，重复打开复用已有标签。
- （历史基线）首版曾浏览启动工作目录；当前行为已由文末“远端目录与代码高亮”取代。
- 设置与新增 SSH 均位于弹出面板；面板阻止点击穿透。48×18 紧凑窗口中面板占满窗口，避免输入区域被压缩。
- 设置支持文件栏和隐藏文件显示，当前仅本次运行有效。主机仍写原有 hosts.toml；凭据不持久化。
- 文件传输仍走现有前台 scp 流程，非后台队列。

## 状态与不变量

- Alt 快捷键由工作区处理；普通字符、Ctrl+C、Tab、Esc 传给获得焦点的终端，不再被主机首页截获。
- PTY 输出通过有界通道消费，隐藏标签继续刷新；窗口变化更新所有 PTY 大小。每个标签独立 VT 状态、5000 行滚动历史。
- 关闭标签/退出销毁子进程并回收；SSH 退出后释放认证桥与临时私钥，保留终端末屏供查看。
- AuthBridge 复用原 OpenSSH 参数、带令牌的 loopback askpass 通道和私钥文件生命周期。首次指纹默认取消，重试支持重新输入，不自动信任。
- 配置保存失败保持原连接状态，新增终端失败在状态栏显示原因。

## 任务与验收

- [x] PTY 终端、输入/粘贴、resize、后台输出、关闭、混合标签。
- [x] 文件树展开/收起、隐藏项开关、文本预览、拒绝二进制、重复打开复用标签。
- [x] 主机列表/添加/设置浮层，鼠标目标隔离，紧凑布局。
- [x] 复用 SSH 认证/传输与会话临时凭据回收。
- [x] 更新 README、旧集成测试操作入口，增加工作区真实交互测试。

验证入口：`cargo test --locked --workspace`、fmt、Clippy；`tests/workspace_smoke.py`、`tests/remote_smoke.py`、`tests/ssh_smoke.py`、`tests/auth_smoke.py`。实际结果见下方交付记录。

## 验证限制

仅 Linux 实测。未宣称完整实现 xterminal：不含远端文件编辑、后台传输队列、终端内鼠标协议透传或持久工作区。设置为会话级。全屏终端程序可通过键盘操作；外层终端快捷键冲突需由用户终端配置调整。

采用 wind-vibe 规范/规格双视角自审；工作区没有 Git 基线，未声称执行基于 commit 的独立 code-review。impeccable 只借鉴设计与检查方法，未执行其网页生成和代理流水线。当前项目未建立 CodeGraph 索引，直接读取源码；需要时用户可在仓库运行 `codegraph init`。

## 本轮交付记录

- `cargo test --locked --workspace`：14 项通过（新增文件树/重复预览/二进制拒绝行为）。
- `cargo fmt --all --check`、`cargo clippy --locked --workspace --all-targets -- -D warnings`：通过。
- debug 与 release 构建通过，`target/release/windshell` 已更新。
- `python3 tests/workspace_smoke.py`：真实 tmux/PTY 验证独立标签、后台输出、Ctrl+C 路由、滚动历史、目录展开、中文只读预览、混合标签切换/关闭、浮层点击隔离、设置、48×18 输入与退出。
- `python3 tests/ssh_smoke.py`：真实系统 sshd 的 shell、1 MiB 字节往返、缺失远端错误、传输 Ctrl+C、主机密钥不匹配拒绝和紧凑表单通过。
- `.audit/ux-auth/venv/bin/python tests/auth_smoke.py`：OpenSSH + Paramiko 服务端的密码/重输、指纹确认/取消、加密私钥/口令、SFTP、临时密钥权限/回收和重启询问通过。系统 Python 未安装 Paramiko，使用项目已有测试虚拟环境。
- `.audit/workspace/*.ansi` 保存实际终端捕获，配套 PNG 由 ANSI 字符网格渲染用于视觉检查（非桌面截图）。已检查文件预览、SSH 浮层、设置与紧凑表单。环境含 `NO_COLOR=1`，视觉捕获时显式移除以验证主题；应用尊重用户的 NO_COLOR 设置。
- 规范自审：保留认证校验/秘密边界与旧配置；PTY 有界输出通道、会话回收、错误可见。规格自审：每项工作区要求映射到实现及交互测试；限制如上，无未解决的交付阻断项。不声明完整 xterminal 功能对等或跨平台运行已通过。


## 远端目录与代码高亮（当前行为，2026-10-28）

用户澄清左栏应当属于 SSH 远端，本轮取代本地文件树，原本本地浏览的实现与测试同步替换。

- 每次 SSH 连接建立独立的私有 ControlMaster socket；左栏通过 SFTP v3 从登录目录开始读取。切换标签按会话 ID 隔离文件树、选择状态和预览；本地终端显示连接提示。
- 目录展开/收起，Backspace 上级目录，R 刷新；隐藏文件开关有效。仅列出普通文件和目录，暂不遍历符号链接或特殊设备。
- 读取通过独立工作线程中的 SFTP 短通道完成，复用已认证主连接；禁止主连接消失后退回新 SSH 认证。每次完成即关闭通道，避免阻止主终端 exit。20 秒超时可见；关闭窗口/会话释放子进程和 socket 目录。
- 不经过远端 shell 拼接路径，不往交互终端注入文件命令。目录最多 20000 项，协议包最大 2 MiB，预览文件最大 1 MiB；文件类型、UTF-8、二进制及长度校验保留。
- Syntect 根据扩展名/shebang 高亮，保留跨行注释状态，预先缓存样式行，滚动时只渲染视口；未知语法使用纯文本。只读，不提供编辑保存。
- 服务器须启用 SFTP。当前远端浏览依赖 Unix OpenSSH ControlMaster，Linux 实测，macOS 未实测；Windows 不支持本轮远端浏览。

测试入口：`tests/remote_smoke.py` 使用两个隔离的真实 sshd/SFTP 服务，验证目录展开、源文件预览、高亮样式、二进制/大文件拒绝、双会话隔离、本地空态、终端不受干扰和 exit；同时检查每台 sshd 只有一次认证。单元测试覆盖隔离、预览复用、语法颜色与畸形协议包。原 auth/ssh/workspace 回归继续运行。
