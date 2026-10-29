"""Real PTY workspace interaction test, isolated tmux and temporary files."""
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import time

ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'.audit/workspace'
OUT.mkdir(parents=True,exist_ok=True)
MUX=['tmux','-L',f'windshell-workspace-{os.getpid()}','-f','/dev/null']
def mux(*args):
    return subprocess.check_output([*MUX,*args],text=True)
def key(*keys): mux('send-keys','-t','test',*keys)
def literal(text): mux('send-keys','-t','test','-l',text)
def screen(): return mux('capture-pane','-p','-t','test')
def wait(text):
    until=time.monotonic()+8
    while time.monotonic()<until:
        current=screen()
        if text in current: return current
        time.sleep(.05)
    raise AssertionError(f'Missing {text!r}:\n{current}')
def capture(name): (OUT/f'{name}.ansi').write_text(mux('capture-pane','-p','-e','-t','test'))
with tempfile.TemporaryDirectory(prefix='windshell-workspace-') as temp:
    root=Path(temp)
    (root/'folder').mkdir()
    (root/'folder'/'sample.txt').write_text('PREVIEW_CONTENT 中文\nsecond line\n')
    command=shlex.join(['env','-u','NO_COLOR',str(ROOT/'target/debug/windshell'),'--config',str(root/'hosts.toml')])
    try:
        mux('new-session','-d','-s','test','-x','120','-y','36','-c',str(root),command)
        wait('Alt+B')
        literal("printf 'FIRST_%s\\n' SESSION; sleep 2; printf 'BACKGROUND_%s\\n' DONE")
        key('Enter'); wait('FIRST_SESSION')
        key('M-t'); literal("printf 'SECOND_%s\\n' SESSION"); key('Enter'); wait('SECOND_SESSION')
        assert 'FIRST_SESSION' not in screen()
        key('M-Left'); wait('BACKGROUND_DONE')
        literal("printf 'SCROLL_LINE_%s\n' {1..80}"); key('Enter'); wait('SCROLL_LINE_80')
        key('S-PageUp'); time.sleep(.1); assert 'SCROLL_LINE_80' not in screen()
        key('S-PageDown'); wait('SCROLL_LINE_80')
        literal('sleep 60'); key('Enter'); time.sleep(.1); key('C-c')
        literal("printf 'INTERRUPT_%s\\n' OK"); key('Enter'); wait('INTERRUPT_OK')
        capture('terminal')
        # File pane is remote-only until an SSH tab is connected.
        wait('连接 SSH 后显示远端文件')
        assert 'sample.txt' not in screen()
        key('M-Right'); wait('SECOND_SESSION')
        key('M-s'); wait('新增连接'); capture('ssh-panel')
        # Outside the modal must not change tabs or create a terminal.
        literal('\x1b[<0;110;1M\x1b[<0;110;1m')
        wait('新增连接')
        key(','); wait('显示隐藏文件'); capture('settings')
        key('Escape'); wait('SECOND_SESSION')
        key('M-w'); wait('INTERRUPT_OK')
        mux('resize-window','-t','test','-x','48','-y','18')
        key('M-s'); key('n'); wait('主机 / SSH 别名'); capture('compact')
        key('Escape'); key('M-q')
        time.sleep(.2)
        assert subprocess.run([*MUX,'has-session','-t','test'],capture_output=True).returncode!=0
        print('PASS: independent PTYs, background output, Ctrl+C routing, remote-only empty file pane, terminal tabs, modal click isolation, settings, compact layout, cleanup')
    finally:
        subprocess.run([*MUX,'kill-server'],capture_output=True)
