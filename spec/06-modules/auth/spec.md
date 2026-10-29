# 连接表单与认证

规格有效；本轮实现完成；Linux 本地验收通过，跨平台运行时仍待实测。用户反馈见 [原始输入](../../01-inputs/user-prompts.md)。初始代码/规格快照：`.audit/ux-auth/before.tar.gz`；本轮范围相对该快照，不包含第一版已有功能的独立开发声明。

## 交互

连接管理采用 GUI 常见的主机列表、详情区、操作按钮与分组表单，在 TUI 中实现键盘与鼠标操作。保持终端配色，不假定用户使用深色终端；青色标识焦点/主操作，错误有文字原因。

- 新建/编辑连接 → 填写名称、主机、用户、端口 → 选择「SSH 配置 / Agent」「密码」「粘贴私钥」。名称留空时使用主机名，用户/端口留空沿用 SSH 默认值。
- 输入框支持左右光标、Home/End、Delete/Backspace、Ctrl+U 清空。Tab/Shift+Tab 或上下切换字段；Enter 在字段间前进，只有聚焦按钮时执行操作。F5 保存并连接，F10 只保存，Esc 放弃表单。
- 密码默认遮挡，F2 可显隐；私钥始终只显示已输入行数，支持完整多行粘贴，无需路径。接受 LF、CRLF 和终端粘贴产生的 CR 换行，粘贴不触发提交/导航。粘贴使用终端的粘贴菜单/快捷键（常见 Ctrl+Shift+V），要求支持 bracketed paste 的终端；不读取全局剪贴板。
- 新建/编辑可只保存连接元数据，凭据为空时连接前再询问。凭据保留在当前进程内，关闭后重新输入；不增加密码库/操作系统 keyring，持久化凭据属于下一阶段选择。
- 支持加密私钥的口令，未预填或首次输入错误时，在 TUI 接受 OpenSSH 的认证请求。缓存密码错误时后续询问可重新输入；不把同一错误密码无限重发。
- 首次主机指纹以 OpenSSH 给出的真实提示展示，默认焦点为取消。明确选择信任才继续；不关闭主机密钥校验，密钥变化仍由 OpenSSH 拒绝。
- 主机支持鼠标选择，按钮/字段支持点击；删除默认取消。48×18 起可使用，表单按焦点滚动，小窗口也可访问最后一个字段和操作按钮。
- 上传下载沿用单文件传输表单，Enter 先前进到按钮，F5 明确提交。传输仍显示 OpenSSH 原生进度。此轮「不填路径」指登录私钥，文件传输仍需本地/远端路径，不声称实现了文件选择器。

## 实现边界与不变量

`windshell-core::Authentication` 持久化认证方式；`Credentials` 独立且不实现 Debug/Serialize，用于运行时凭据，不进入 HostStore。旧配置缺少认证方式时默认 Auto；已有 `identity` 配置继续被 Auto 使用，避免破坏第一版用户保存的配置，TUI 不再要求填写此字段。

SSH_ASKPASS 使用当前可执行程序的内部 helper。OpenSSH 与父进程通过仅绑定回环地址的通道请求认证，使用随机令牌校验，密码不放进命令参数或环境变量；环境只传本地通道地址与令牌。父进程只在 OpenSSH 等待认证时取得终端输入；认证结束后把完整终端交还给远端会话，避免两个事件读取器争抢输入。实现依据：[OpenSSH askpass 官方约定](https://man.openbsd.org/ssh.1#SSH_ASKPASS_REQUIRE)、[Crossterm bracketed paste](https://docs.rs/crossterm/0.29.0/crossterm/event/struct.EnableBracketedPaste.html)。

粘贴私钥交给 OpenSSH 前，使用私有临时文件（Unix 0600）；文件在会话/传输期间存在，正常退出、认证取消和普通错误均删除。不宣称私钥从不落盘；SIGKILL、系统崩溃不能保证删除。密码和口令不写临时文件。应用退出时移除 bracketed paste/鼠标捕获；panic hook 也清理这些终端模式。

核心沿用同步单会话，不引入 GUI 框架或空认证抽象。桌面端将来可复用认证请求回调，不需要理解 TUI 焦点模型。

## 验证入口与结果

| 入口 | 覆盖 | 状态 |
| --- | --- | --- |
| `cargo test --locked --workspace` | 配置保全、参数隔离、认证元数据与凭据分离、失败保存、重启重新询问、多行粘贴、CR 换行、UTF-8 编辑、默认删除取消、遮挡及紧凑表单 | 通过 |
| `python3 tests/ssh_smoke.py` | 真 OpenSSH sshd：交互 shell、单文件字节往返、缺失文件、Ctrl+C 后恢复、主机密钥变化拒绝、48×18 | 通过 |
| `python3 tests/auth_smoke.py`（需 tests/requirements.txt） | OpenSSH 客户端 + 隔离 Paramiko SSH/SFTP 服务端：密码登录/重输、首次指纹取消/信任、加密私钥完整粘贴、口令询问、密码与私钥两种方式各 1 MiB 字节往返、配置不含凭据、临时文件权限及删除、重启凭据询问、点击新建按钮 | 通过 |
| fmt / Clippy | 格式与警告 | 通过 |

Paramiko 是独立的协议测试服务端，不是生产后端或纯函数替身；真实系统 sshd 回归另外运行。不修改系统用户密码、不触碰用户 SSH 文件。测试夹具曾缺少 fsetstat 和子系统退出状态，独立 scp 已复现后修正；应用的多行 CR 粘贴问题有先失败、后通过的回归测试。不要把夹具修复误报为产品认证缺陷。

原始证据位于 `.audit/ux-auth/`。Windows/macOS 运行时认证、硬件密钥、MFA、跳板机组合仍未实测；CI 仅配置，不声称已运行。工作区无 Git 基线且无可用 OMX 原生角色配置，本轮采用规范/规格双视角自审，不声称独立代理审查。


## 本轮最终证据

- `.audit/ux-auth/fmt.log`、`clippy.log`、`tests.log`、`build.log`：格式检查、Clippy 零警告、13 个测试和 debug 构建通过。
- `.audit/ux-auth/ssh-smoke.log`：真实系统 sshd 回归通过。
- `.audit/ux-auth/auth-smoke.log`：密码/加密私钥、认证重输、指纹信任与取消、SFTP 字节比对、点击新建及凭据生命周期全部通过。
- `.audit/ux-auth/tui-captures.txt`、`auth-captures.txt`：已查看实际 100×30 与 48×18 终端文本；紧凑布局最后一个认证字段和操作按钮可见，密码遮挡，私钥仅显示行数。
- `.audit/ux-auth/baseline.sha256`：最终交付文件校验清单，无 Git commit；首版文件与本轮基线由 before.tar.gz 保全。

审查结论：规范视角核对了凭据不序列化/不进入命令参数或环境、临时私钥生命周期、终端模式恢复与旧配置继承；规格视角核对了密码输入、完整私钥粘贴、GUI 式按钮与焦点、错误重输及原有传输。未发现阻断本轮交付的问题。采用本轮自审，独立性限制如前。

本轮未新增桌面端、远端文件浏览器或持久凭据库；终端会话与传输进度仍由系统 OpenSSH 接管。Windows/macOS 尚未运行，不能把 Linux 通过等同跨平台认证通过。
