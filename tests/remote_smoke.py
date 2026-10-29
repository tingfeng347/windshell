"""Real OpenSSH/SFTP tests for remote trees and highlighted previews."""
import getpass, os, shlex, shutil, socket, subprocess, tempfile, time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'.audit/remote'; OUT.mkdir(parents=True,exist_ok=True)
def run(*args): return subprocess.check_output(args,text=True,stderr=subprocess.STDOUT)
with tempfile.TemporaryDirectory(prefix='ws-remote-') as tmp:
    d=Path(tmp); servers=[]; logs=[]
    mux=['tmux','-L',f'ws-remote-{os.getpid()}','-f','/dev/null']
    def tmux(*args):return run(*mux,*args)
    def key(*keys):tmux('send-keys','-t','test',*keys)
    def literal(s):tmux('send-keys','-t','test','-l',s)
    def screen():return tmux('capture-pane','-p','-t','test')
    def wait(s):
        end=time.monotonic()+12
        while time.monotonic()<end:
            t=screen()
            if s in t:return t
            time.sleep(.05)
        raise AssertionError(f'Missing {s}:\n{t}')
    def wait_sidebar(text):
        end=time.monotonic()+10
        while time.monotonic()<end:
            if text in '\n'.join(line[:29] for line in screen().splitlines()[1:]):return
            time.sleep(.05)
        raise AssertionError(screen())
    def click(row):literal(f'\x1b[<0;7;{row+1}M\x1b[<0;7;{row+1}m')
    def capture(n):(OUT/f'{n}.ansi').write_text(tmux('capture-pane','-p','-e','-t','test'))
    try:
        for name in ['host','client']:run('ssh-keygen','-q','-t','ed25519','-N','','-f',str(d/name))
        (d/'authorized_keys').write_text((d/'client.pub').read_text())
        configs=[]; hosts=[]; known=[]
        for name in ['alpha','beta']:
            root=d/name;root.mkdir();(root/'src').mkdir()
            (root/'src'/'main.rs').write_text(f'// REMOTE_{name.upper()} 中文\nfn main() {{\n    let answer = "hello";\n    println!("{{}}", answer);\n}}\n')
            (root/'binary').write_bytes(b'\x00\xff');(root/'large.txt').write_bytes(b'x'*(1024*1024+1));(root/'.hidden').write_text('hidden')
            with socket.socket() as sock:sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
            cfg=d/f'{name}.sshd';cfg.write_text(f'Port {port}\nListenAddress 127.0.0.1\nHostKey {d}/host\nAuthorizedKeysFile {d}/authorized_keys\nPidFile {d}/{name}.pid\nStrictModes no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nSubsystem sftp internal-sftp -d {root}\n')
            log=(OUT/f'{name}.log').open('w');logs.append(log)
            servers.append(subprocess.Popen([shutil.which('sshd'),'-D','-e','-f',str(cfg)],stdout=log,stderr=log))
            pub=(d/'host.pub').read_text().split();known.append(f'[127.0.0.1]:{port} {pub[0]} {pub[1]}\n')
            configs.append(f'Host {name}\n HostName 127.0.0.1\n Port {port}\n User {getpass.getuser()}\n IdentityFile {d}/client\n IdentitiesOnly yes\n UserKnownHostsFile {d}/known_hosts\n StrictHostKeyChecking yes\n')
            hosts.append(f'[[hosts]]\nname="{name}"\naddress="{name}"\n')
        (d/'known_hosts').write_text(''.join(known));(d/'config').write_text(''.join(configs));(d/'hosts.toml').write_text(''.join(hosts))
        cmd=shlex.join(['env','-u','NO_COLOR',str(ROOT/'target/debug/windshell'),'--config',str(d/'hosts.toml'),'--ssh-config',str(d/'config')])
        tmux('new-session','-d','-s','test','-x','120','-y','36',cmd)
        wait('连接 SSH 后显示远端文件');key('M-s');key('Enter');wait('src');wait('alpha · SFTP')
        click(4);wait_sidebar('main.rs');click(5);wait('REMOTE_ALPHA');capture('highlight')
        ansi=tmux('capture-pane','-p','-e','-t','test');assert '\x1b[38;' in ansi
        # The same path in a session reuses its existing preview tab.
        click(5);wait('REMOTE_ALPHA')
        key('M-Left');literal("printf 'SHELL_%s\\n' ALIVE");key('Enter');wait('SHELL_ALIVE')
        # Preview errors stay visible without harming shell or tree.
        click(6);wait('二进制文件');click(7);wait('超过 1 MiB')
        key('M-s');key('Down');key('Enter');wait('beta · SFTP');assert 'main.rs' not in '\n'.join(line[:29] for line in screen().splitlines()[1:])
        click(4);wait_sidebar('main.rs');click(5);wait('REMOTE_BETA');capture('beta')
        key('M-Left');wait('beta · SFTP');key('M-Left');wait('REMOTE_ALPHA');wait('alpha ·')
        # Local terminal never presents a remote tree as local files.
        key('M-Left');key('M-Left');wait('连接 SSH 后显示远端文件');assert 'main.rs' not in '\n'.join(line[:29] for line in screen().splitlines()[1:])
        key('M-Right');literal('exit');key('Enter');wait('会话已结束')
        key('M-q');
        for name in ['alpha','beta']: assert (OUT/f'{name}.log').read_text().count('Accepted publickey')==1, 'SFTP unexpectedly reauthenticated'
        print('PASS: real SFTP listing/expansion/preview, syntax styles, session isolation, shell unaffected, binary/size rejection, local empty state, SSH exit')
    finally:
        try:(OUT/'last.txt').write_text(screen())
        except subprocess.CalledProcessError:pass
        subprocess.run([*mux,'kill-server'],capture_output=True)
        for server in servers:server.terminate();server.wait(timeout=5)
        for log in logs:log.close()
