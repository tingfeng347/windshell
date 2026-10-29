#!/usr/bin/env python3
"""Opt-in auth integration: OpenSSH client + real SSH/SFTP protocol fixture.
Requires tmux, ssh-keygen and Paramiko (`pip install -r tests/requirements.txt`).
No system accounts/passwords or user SSH files are modified.
"""
import base64
import hashlib
import logging
import os
from pathlib import Path
import posixpath
import shlex
import socket
import subprocess
import tempfile
import threading
import time

import paramiko

ROOT = Path(__file__).resolve().parents[1]
EVIDENCE = ROOT / ".audit/ux-auth"
EVIDENCE.mkdir(parents=True, exist_ok=True)
# Disposable fixture credentials, never real account credentials.
PASSWORD = "fixture-session-secret"
PASSPHRASE = "fixture-key-unlock"
logging.basicConfig(filename=EVIDENCE / "paramiko.log", level=logging.WARNING)


def run(*args):
    return subprocess.run(args, check=True, text=True, capture_output=True).stdout


class Handle(paramiko.SFTPHandle):
    def stat(self):
        f = getattr(self, "readfile", None) or self.writefile
        return paramiko.SFTPAttributes.from_stat(os.fstat(f.fileno()))


    def chattr(self, attr):
        f = getattr(self, "readfile", None) or self.writefile
        f.flush()
        if attr.st_size is not None:
            os.ftruncate(f.fileno(), attr.st_size)
        if attr.st_mode is not None:
            os.fchmod(f.fileno(), attr.st_mode)
        return paramiko.SFTP_OK


class SFTP(paramiko.SFTPServerInterface):
    def __init__(self, server, *, root):
        super().__init__(server)
        self.root = root

    def path(self, path):
        result = (self.root / path.lstrip("/")).resolve()
        if not result.is_relative_to(self.root):
            raise PermissionError("outside fixture")
        return result

    def canonicalize(self, path):
        return posixpath.normpath("/" + path.lstrip("/"))

    def stat(self, path):
        try:
            return paramiko.SFTPAttributes.from_stat(self.path(path).stat())
        except OSError as e:
            return paramiko.SFTPServer.convert_errno(e.errno)

    lstat = stat

    def open(self, path, flags, attr):
        try:
            fd = os.open(self.path(path), flags, 0o600)
            handle = Handle(flags)
            if flags & os.O_RDWR:
                handle.readfile = handle.writefile = os.fdopen(fd, "r+b")
            elif flags & os.O_WRONLY:
                handle.writefile = os.fdopen(fd, "wb")
            else:
                handle.readfile = os.fdopen(fd, "rb")
            return handle
        except OSError as e:
            return paramiko.SFTPServer.convert_errno(e.errno)


class SFTPSubsystem(paramiko.SFTPServer):
    def finish_subsystem(self):
        # Match a real sftp-server process: EOF must carry a successful status.
        self.sock.send_exit_status(0)
        super().finish_subsystem()


class Server(paramiko.ServerInterface):
    def __init__(self, public_key, attempts):
        self.public_key = public_key
        self.attempts = attempts
        self.shell = threading.Event()
        self.method = ""

    def get_allowed_auths(self, username):
        return "password,publickey"

    def check_auth_password(self, username, password):
        accepted = username == "fixture" and password == PASSWORD
        self.attempts.append(("password", accepted))
        if accepted:
            self.method = "password"
        return paramiko.AUTH_SUCCESSFUL if accepted else paramiko.AUTH_FAILED

    def check_auth_publickey(self, username, key):
        accepted = username == "fixture" and key == self.public_key
        self.attempts.append(("publickey", accepted))
        if accepted:
            self.method = "publickey"
        return paramiko.AUTH_SUCCESSFUL if accepted else paramiko.AUTH_FAILED

    def check_channel_request(self, kind, chanid):
        return paramiko.OPEN_SUCCEEDED if kind == "session" else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED

    def check_channel_pty_request(self, *args):
        return True

    def check_channel_shell_request(self, channel):
        self.shell.set()
        return True


class Fixture:
    def __init__(self, root, public_key):
        self.root = root
        self.public_key = public_key
        self.key = paramiko.RSAKey.generate(2048)
        self.socket = socket.socket()
        self.socket.bind(("127.0.0.1", 0))
        self.socket.listen()
        self.socket.settimeout(.2)
        self.port = self.socket.getsockname()[1]
        self.stop = threading.Event()
        self.transports = []
        self.threads = []
        self.attempts = []
        self.thread = threading.Thread(target=self.accept, daemon=True)
        self.thread.start()

    def accept(self):
        while not self.stop.is_set():
            try:
                client, _ = self.socket.accept()
            except (TimeoutError, OSError):
                continue
            thread = threading.Thread(target=self.serve, args=(client,), daemon=True)
            self.threads.append(thread)
            thread.start()

    def serve(self, client):
        transport = paramiko.Transport(client)
        self.transports.append(transport)
        server = Server(self.public_key, self.attempts)
        try:
            transport.add_server_key(self.key)
            transport.set_subsystem_handler("sftp", SFTPSubsystem, SFTP, root=self.root)
            transport.start_server(server=server)
            channel = transport.accept(15)
            if channel is None:
                return
            # Keep both shell and SFTP channels alive until the real client exits.
            while transport.is_active() and not self.stop.is_set():
                if server.shell.wait(.05):
                    channel.sendall(f"AUTHENTICATED_{server.method}\r\nfixture> ".encode())
                    data = b""
                    while b"exit" not in data and transport.is_active():
                        block = channel.recv(1024)
                        if not block:
                            break
                        data += block
                    channel.send_exit_status(0)
                    channel.close()
                    break
        except (OSError, EOFError, paramiko.SSHException):
            pass
        finally:
            transport.close()

    def close(self):
        self.stop.set()
        self.socket.close()
        for transport in self.transports:
            transport.close()
        self.thread.join(timeout=2)
        for thread in self.threads:
            thread.join(timeout=2)


def main():
    binary = ROOT / "target/debug/windshell"
    captures = []
    with tempfile.TemporaryDirectory(prefix="windshell-auth-") as tmp:
        root = Path(tmp)
        keytmp = root / "temporary-keys"
        keytmp.mkdir()
        run("ssh-keygen", "-q", "-t", "ed25519", "-N", PASSPHRASE, "-f", str(root / "client"))
        public_key = paramiko.Ed25519Key.from_private_key_file(root / "client", password=PASSPHRASE)
        server = Fixture(root, public_key)
        fingerprint = "SHA256:" + base64.b64encode(hashlib.sha256(server.key.asbytes()).digest()).decode().rstrip("=")
        config = root / "ssh_config"
        hosts = root / "hosts.toml"
        config.write_text(f'''Host fixture
  HostName 127.0.0.1
  Port {server.port}
  User fixture
  IdentityFile none
  UserKnownHostsFile {root}/known_hosts
  StrictHostKeyChecking ask
''')
        mux = ["tmux", "-L", f"windshell-auth-{os.getpid()}", "-f", "/dev/null"]

        def tmux(*args):
            return run(*mux, *args)

        def screen():
            return tmux("capture-pane", "-p", "-t", "auth")

        def wait_for(text, timeout=12):
            until = time.monotonic() + timeout
            last = ""
            while time.monotonic() < until:
                last = screen()
                if text in last:
                    assert PASSWORD not in last and PASSPHRASE not in last
                    assert public_key.get_base64() not in last
                    captures.append(last)
                    return last
                time.sleep(.05)
            raise AssertionError(f"missing {text!r}:\n{last}")

        def key(value):
            tmux("send-keys", "-t", "auth", value)

        def literal(value):
            tmux("send-keys", "-t", "auth", "-l", value)

        def click(column, row):
            # SGR mouse press/release in the real terminal input stream.
            literal(f"\x1b[<0;{column + 1};{row + 1}M\x1b[<0;{column + 1};{row + 1}m")

        def paste_key():
            tmux("load-buffer", "-b", "test-key", str(root / "client"))
            tmux("paste-buffer", "-p", "-b", "test-key", "-t", "auth")
            wait_for("已输入")

        def start():
            command = shlex.join(["env", f"TMPDIR={keytmp}", str(binary), "--config", str(hosts), "--ssh-config", str(config)])
            tmux("new-session", "-d", "-s", "auth", "-x", "100", "-y", "30", command)

        def shell_exit(method):
            wait_for("AUTHENTICATED_" + method)
            if method == "publickey":
                files = [p for p in keytmp.iterdir() if p.is_file()]
                assert len(files) == 1, "private key lifetime not tied to session"
                assert files[0].stat().st_mode & 0o777 == 0o600
            literal("exit\n")
            wait_for("会话已结束")
            key("M-w")
            key("M-s")
            wait_for("连接详情")
            assert not list(keytmp.iterdir()), "temporary key not cleaned up"

        def transfer(upload, local, remote):
            key("u" if upload else "g")
            wait_for("本地路径")
            literal(str(local))
            key("Tab")
            literal(remote)
            key("F5")
            wait_for("操作完成")

        try:
            start()
            key("M-s")
            wait_for("还没有连接")
            click(6, 6)  # the visible New Connection button
            wait_for("新建连接")
            literal("密码测试")
            key("Tab")
            literal("fixture")
            for _ in range(3):
                key("Tab")
            key("Right")  # auto -> password
            key("Tab")
            literal(PASSWORD)
            wait_for("登录密码")
            assert PASSWORD not in screen()
            key("F5")
            trust = wait_for("验证主机指纹")
            assert fingerprint in trust, "wrong fingerprint shown"
            key("Enter")  # initial focus is Cancel, never implicit trust
            wait_for("已取消认证")
            assert not server.attempts, "credentials sent before host trust"
            key("M-w")
            key("M-s")
            key("Enter")
            wait_for("验证主机指纹")
            key("Tab")
            key("Enter")
            shell_exit("password")
            assert PASSWORD not in hosts.read_text()
            payload = bytes(range(256)) * 4096
            source = root / "本地 源.bin"
            source.write_bytes(payload)
            transfer(True, source, "/远端 文件.bin")
            assert (root / "远端 文件.bin").read_bytes() == payload
            downloaded = root / "下载.bin"
            transfer(False, downloaded, "/远端 文件.bin")
            assert downloaded.read_bytes() == payload
            # Wrong cached password must produce an editable in-TUI retry.
            key("e")
            for _ in range(5):
                key("Tab")
            key("C-u")
            literal("deliberately-wrong")
            key("F5")
            wait_for("SSH 认证")
            literal(PASSWORD)
            key("F5")
            shell_exit("password")
            assert ("password", False) in server.attempts
            # Replace password auth with an entire encrypted clipboard key.
            key("e")
            for _ in range(4):
                key("Tab")
            key("Right")
            key("Tab")
            paste_key()
            key("Tab")
            literal(PASSPHRASE)
            key("F5")
            shell_exit("publickey")
            saved = hosts.read_text()
            assert "private_key" in saved
            assert PASSWORD not in saved and PASSPHRASE not in saved
            assert "BEGIN" not in saved and public_key.get_base64() not in saved
            # Transfer uses the same in-memory clipboard credentials.
            transfer(True, source, "/私钥上传.bin")
            transfer(False, root / "私钥下载.bin", "/私钥上传.bin")
            assert (root / "私钥下载.bin").read_bytes() == payload
            key("q")
            time.sleep(.2)
            start()
            key("M-s")
            wait_for("连接详情")
            key("Enter")
            wait_for("登录认证")
            paste_key()
            key("F5")  # leave passphrase blank -> askpass TUI prompt
            wait_for("SSH 认证")
            literal(PASSPHRASE)
            key("F5")
            shell_exit("publickey")
            key("e")
            tmux("resize-window", "-t", "auth", "-x", "48", "-y", "18")
            for _ in range(6):
                key("Tab")
            wait_for("私钥口令")
            key("Escape")
            key("M-q")
            print("PASS: password auth/retry, explicit host fingerprint trust/cancel, encrypted key clipboard paste, passphrase prompt, password/key SFTP round trips, secret masking/non-persistence, temporary key permissions/cleanup, restart re-prompt, compact form")
        finally:
            (EVIDENCE / "auth-captures.txt").write_text("\n--- screen ---\n".join(captures))
            for flag, name in [((), "active"), (("-a",), "alternate")]:
                result = subprocess.run([*mux, "capture-pane", "-p", *flag, "-t", "auth"], text=True, capture_output=True)
                (EVIDENCE / f"auth-{name}.txt").write_text(result.stdout)
            subprocess.run([*mux, "kill-server"], capture_output=True)
            server.close()


if __name__ == "__main__":
    main()
