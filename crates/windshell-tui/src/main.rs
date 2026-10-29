mod app;
mod highlight;
mod input;
mod remote;
mod ui;
mod workspace;

use anyhow::{Context, Result};
use clap::Parser;
use std::{io::IsTerminal, path::PathBuf};
use windshell_core::HostStore;

#[derive(Parser)]
#[command(version, about = "WindShell · SSH 连接与文件传输 TUI")]
struct Args {
    /// Override the platform-specific host configuration path
    #[arg(long)]
    config: Option<PathBuf>,
    /// Use an alternate OpenSSH config (passed to both ssh and scp)
    #[arg(long)]
    ssh_config: Option<PathBuf>,
    /// Validate configuration without opening the TUI
    #[arg(long)]
    check: bool,
}

fn main() -> Result<()> {
    if std::env::var_os("WINDSHELL_ASKPASS_ADDR").is_some() {
        // Askpass stdout is protocol data; errors must never print secrets.
        std::process::exit(if windshell_core::askpass_helper().is_ok() {
            0
        } else {
            1
        });
    }
    let args = Args::parse();
    let path = match args.config {
        Some(path) => path,
        None => directories::ProjectDirs::from("", "", "windshell")
            .context("无法定位用户配置目录，请使用 --config")?
            .config_dir()
            .join("hosts.toml"),
    };
    let store = HostStore::load(&path)?;
    if args.check {
        println!(
            "配置有效 · {} 个连接 · {}",
            store.hosts.len(),
            path.display()
        );
        return Ok(());
    }
    anyhow::ensure!(
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        "请在交互终端中启动 WindShell；检查配置可用 --check"
    );
    // The child and parent share a foreground terminal process group. Keep the
    // manager alive when Ctrl+C interrupts ssh/scp; raw-mode Ctrl+C is a key.
    ctrlc::set_handler(|| {}).context("安装终端中断处理失败")?;
    let mut app = app::App::new(store, path);
    app.ssh_config = args.ssh_config;
    app.run()
}
