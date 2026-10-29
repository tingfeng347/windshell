#!/usr/bin/env python3
"""Linux/macOS opt-in real TUI + OpenSSH test. Requires tmux, sshd and ssh-keygen.
Uses an isolated tmux server, ephemeral localhost SSH server and pinned host key.
Run after `cargo build -p windshell`: python3 tests/ssh_smoke.py
"""
import getpass
import os
from pathlib import Path
import shlex
import shutil
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
EVIDENCE = ROOT / ".audit/ux-auth"
EVIDENCE.mkdir(parents=True, exist_ok=True)


def run(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, **kwargs).stdout


def main():
    binary = ROOT / "target/debug/windshell"
    for tool in ("tmux", "sshd", "ssh-keygen"):
        if not shutil.which(tool):
            raise RuntimeError(f"missing prerequisite: {tool}")
    if not binary.is_file():
        raise RuntimeError("run cargo build -p windshell first")
    with tempfile.TemporaryDirectory(prefix="windshell-smoke-") as tmp:
        d = Path(tmp)
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            port = s.getsockname()[1]
        for key in ("host", "client"):
            run("ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(d / key))
        host_key = (d / "host.pub").read_text().split()
        (d / "known_hosts").write_text(f"[127.0.0.1]:{port} {host_key[0]} {host_key[1]}\n")
        (d / "authorized_keys").write_text((d / "client.pub").read_text())
        wrong_key = (d / "client.pub").read_text().split()
        (d / "wrong_hosts").write_text(f"[127.0.0.1]:{port} {wrong_key[0]} {wrong_key[1]}\n")
        (d / "sshd_config").write_text(f'''Port {port}
ListenAddress 127.0.0.1
HostKey {d}/host
AuthorizedKeysFile {d}/authorized_keys
PidFile {d}/sshd.pid
StrictModes no
PasswordAuthentication no
KbdInteractiveAuthentication no
UsePAM no
Subsystem sftp internal-sftp
''')
        (d / "ssh_config").write_text(f'''Host wrong-key
  UserKnownHostsFile {d}/wrong_hosts
Host smoke slow wrong-key
  HostName 127.0.0.1
  Port {port}
  User {getpass.getuser()}
  IdentityFile {d}/client
  IdentitiesOnly yes
  UserKnownHostsFile {d}/known_hosts
  StrictHostKeyChecking yes
Host slow
  ProxyCommand sleep 20
''')
        mux = ["tmux", "-L", f"windshell-test-{os.getpid()}", "-f", "/dev/null"]
        log = (EVIDENCE / "sshd.log").open("w")
        server = subprocess.Popen([shutil.which("sshd"), "-D", "-e", "-f", str(d / "sshd_config")], stdout=log, stderr=log)
        captures = []

        def tmux(*args):
            return run(*mux, *args)

        def screen():
            return tmux("capture-pane", "-p", "-t", "test")

        def wait_for(text, timeout=10):
            until = time.monotonic() + timeout
            last = ""
            while time.monotonic() < until:
                if server.poll() is not None:
                    raise RuntimeError((EVIDENCE / "sshd.log").read_text())
                last = screen()
                if text in last:
                    captures.append(last)
                    return
                time.sleep(0.05)
            raise AssertionError(f"missing {text!r}:\n{last}")

        def key(value):
            tmux("send-keys", "-t", "test", value)

        def literal(value):
            tmux("send-keys", "-t", "test", "-l", value)

        def transfer(upload, local, remote):
            key("u" if upload else "g")
            wait_for("本地路径")
            literal(str(local))
            key("Tab")
            literal(str(remote))
            key("F5")

        try:
            until = time.monotonic() + 5
            while True:
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=.2):
                        break
                except OSError:
                    if time.monotonic() > until:
                        raise RuntimeError((EVIDENCE / "sshd.log").read_text())
                    time.sleep(.05)
            command = shlex.join([str(binary), "--config", str(d / "hosts.toml"), "--ssh-config", str(d / "ssh_config")])
            tmux("new-session", "-d", "-s", "test", "-x", "100", "-y", "30", command)
            key("M-s")
            wait_for("还没有连接")
            key("n")
            literal("本机测试")
            key("Tab")
            literal("smoke")
            key("F10")
            wait_for("配置已保存")
            key("Enter")
            time.sleep(.4)
            literal("printf 'WINDSHELL_%s\\n' SSH_OK")
            key("Enter")
            wait_for("WINDSHELL_SSH_OK")
            literal("exit")
            key("Enter")
            wait_for("会话已结束")
            key("M-w")
            key("M-s")
            source = d / "源 文件.bin"
            payload = bytes(range(256)) * 4096
            source.write_bytes(payload)
            remote = d / "远端 文件;literal.bin"
            transfer(True, source, remote)
            wait_for("操作完成")
            assert remote.read_bytes() == payload, "upload bytes differ"
            target = d / "下载 文件.bin"
            transfer(False, target, remote)
            wait_for("操作完成")
            assert target.read_bytes() == payload, "download bytes differ"
            transfer(False, d / "missing-out", d / "does-not-exist")
            wait_for("操作未成功")
            # A real scp process waiting in ProxyCommand exercises SIGINT in the
            # shared terminal process group, without a huge transfer fixture.
            key("e")
            key("Tab")
            key("C-u")
            literal("slow")
            key("F10")
            wait_for("配置已保存")
            transfer(True, source, remote)
            time.sleep(.4)
            key("C-c")
            wait_for("操作未成功")
            # Mismatched pinned host keys must remain an SSH failure.
            key("e")
            key("Tab")
            key("C-u")
            literal("wrong-key")
            key("F10")
            wait_for("配置已保存")
            key("Enter")
            wait_for("Host key verification failed")
            key("M-w")
            key("M-s")
            key("e")
            tmux("resize-window", "-t", "test", "-x", "48", "-y", "18")
            key("Tab")
            key("Tab")
            key("Tab")
            key("Tab")
            wait_for("登录方式")
            key("Escape")
            key("M-q")
            time.sleep(.2)
            result = subprocess.run([*mux, "has-session", "-t", "test"], capture_output=True)
            assert result.returncode != 0, "TUI did not exit"
            check = run(str(binary), "--config", str(d / "hosts.toml"), "--check")
            print(check.strip())
            print("PASS: create, SSH shell, upload/download 1 MiB exact bytes, missing remote failure, Ctrl+C returns to TUI, host-key mismatch rejected, 48x18 form, clean quit")
        finally:
            (EVIDENCE / "tui-captures.txt").write_text("\n--- screen ---\n".join(captures))
            subprocess.run([*mux, "kill-server"], capture_output=True)
            server.terminate()
            try:
                server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait()
            log.close()


if __name__ == "__main__":
    main()
