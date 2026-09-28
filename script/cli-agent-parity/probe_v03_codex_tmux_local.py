#!/usr/bin/env python3
"""无账号的 macOS 回环 SSH 验收：固定 Codex hook 在 tmux 内发送，外层接收零次。"""

import argparse
import base64
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import shlex
import socket
import stat
import subprocess
import sys
import tempfile
import threading
import time
import uuid

sys.dont_write_bytecode = True
import probe_codex_ssh_tmux as loopback
import probe_codex_remote_ssh_tmux as remote
import probe_v03_codex_tmux_receipts as v03


HOOK_SOURCE = r'''
import json,os,pathlib,subprocess,sys
payload=json.load(sys.stdin)
event=payload['hook_event_name']
expected={'session':'SessionStart','prompt':'UserPromptSubmit'}[sys.argv[1]]
if event!=expected: raise SystemExit('unexpected hook event')
if event=='UserPromptSubmit' and payload.get('prompt')!=__PROMPT__:
    raise SystemExit('unexpected prompt')
root=pathlib.Path(os.environ['PLUGIN_ROOT'])
script='on-session-start.sh' if event=='SessionStart' else 'on-prompt-submit.sh'
ancestors=[];pid=os.getpid()
for _ in range(8):
    value=subprocess.run(['/bin/ps','-p',str(pid),'-o','ppid=,comm='],
                         capture_output=True,text=True,timeout=3)
    if value.returncode or not value.stdout.strip(): break
    fields=value.stdout.strip().split(maxsplit=1)
    parent=int(fields[0]);ancestors.append({'pid':pid,'ppid':parent,'command':fields[1]})
    if parent<=1: break
    pid=parent
try:
    control=os.open('/dev/tty',os.O_WRONLY|os.O_NOCTTY)
except OSError:
    control_tty_opened=False
else:
    control_tty_opened=True;os.close(control)
result=subprocess.run(['bash',str(root/'reference/scripts'/script)],input=json.dumps(payload),
                      capture_output=True,text=True,timeout=8)
record={'input':payload,'pid':os.getpid(),'ancestry':ancestors,
        'hook_control_tty_opened':control_tty_opened,
        'reference_script':script,'reference_exit':result.returncode,
        'reference_stdout':result.stdout,'reference_stderr':result.stderr,
        'environment':{key:os.environ.get(key) for key in
            ('PLUGIN_ROOT','TMUX','TMUX_PANE','SSH_TTY','WARP_CLI_AGENT_PROTOCOL_VERSION',
             'WARP_CLIENT_VERSION','TERM_PROGRAM')}}
if event=='UserPromptSubmit':
    record['block_output']={'continue':False,'stopReason':os.environ['INFINISHELL_SSH_PROBE_TOKEN']}
output=pathlib.Path(os.environ['INFINISHELL_SSH_PROBE_REPORTS'])/(event+'.json')
output.write_text(json.dumps(record),encoding='utf-8')
if result.returncode: raise SystemExit(result.returncode)
if event=='UserPromptSubmit': print(json.dumps(record['block_output']),flush=True)
'''.replace('__PROMPT__', repr(loopback.PROMPT))


def instrument_remote():
    source = loopback.REMOTE_SOURCE
    source = v03.replace_once(source,
        'import fcntl,json,os,pathlib,shlex,signal,struct,subprocess,sys,termios,time',
        'import fcntl,json,os,pathlib,re,shlex,signal,struct,subprocess,sys,termios,time')
    native_start = "process=subprocess.Popen([config['codex'],'--no-alt-screen'," + repr(loopback.PROMPT) + "])"
    source = v03.replace_once(source, native_start, '''if mode=='tmux-off':
    pane=os.environ.get('TMUX_PANE','')
    if re.fullmatch(r'%[0-9]+',pane) is None: raise SystemExit('invalid tmux pane')
    option=subprocess.check_output([config['tmux'],'-S',entry['socket'],'show-option',
                                    '-gv','allow-passthrough'],text=True).strip()
    if option!='off': raise SystemExit('tmux passthrough is not off')
    pane_tty=subprocess.check_output([config['tmux'],'-S',entry['socket'],
        'display-message','-p','-t',pane,'#{pane_tty}'],text=True).strip()
    if pane_tty!=os.ttyname(0): raise SystemExit('pane tty mismatch')
    capture=report/'inner-pane.raw'
    subprocess.run([config['tmux'],'-S',entry['socket'],'pipe-pane','-O','-t',pane,
        '/bin/cat > '+shlex.quote(str(capture))],check=True,capture_output=True)
    (report/'inner-capture.json').write_text(json.dumps({'pane':pane,'pane_tty':pane_tty,
        'allow_passthrough':option,'installed_before_codex_start':True}))
''' + native_start)
    source = v03.replace_once(source,
        "(report/'native-exit.json').write_text(json.dumps({'pid':process.pid,'exit_code':code,'forced':forced}))",
        '''if mode=='tmux-off':
    subprocess.run([config['tmux'],'-S',entry['socket'],'pipe-pane','-t',pane],
                   check=True,capture_output=True)
    deadline=time.monotonic()+2
    while time.monotonic()<deadline:
        if capture.exists() and capture.read_bytes().count(b'\x1bPtmux;')>=2: break
        time.sleep(.02)
(report/'native-exit.json').write_text(json.dumps({'pid':process.pid,'exit_code':code,'forced':forced}))''')
    return source


def require_private_tmp():
    value = Path(os.environ['TMPDIR'])
    root = Path('/Users/zhishi/InfiniShell-Tests')
    loopback.require(value.parent == root and re.fullmatch(r'r-[0-9a-f]{8}', value.name),
                     'TMPDIR 必须是本轮短私有目录')
    for path in (Path('/Users'), Path('/Users/zhishi'), root, value):
        info = path.lstat()
        loopback.require(stat.S_ISDIR(info.st_mode) and not stat.S_ISLNK(info.st_mode)
                         and not info.st_mode & 0o022, '临时目录祖先不安全')
    info = value.stat()
    loopback.require(info.st_uid == os.getuid() and info.st_mode & 0o777 == 0o700
                     and info.st_dev == Path('/Users').stat().st_dev, '临时目录身份不符')
    return value


def verify_inputs(args, repository):
    manifest = json.loads((repository / 'script/cli-agent-parity/codex_0156_package_manifest.json').read_text())
    package = manifest['packages']['macos-arm64']
    loopback.require(manifest['version'] == '0.156.1'
        and hashlib.sha256(args.codex.read_bytes()).hexdigest() == package['files']['bin/codex'][1]
        and subprocess.check_output([str(args.codex),'--version'],text=True).strip() == 'codex-cli 0.156.1',
        '固定 Codex 完整包入口不符')
    tooling = json.loads((repository / 'specs/cli-agent-parity/fixtures/ssh-tmux-tooling.json').read_text())
    for item in tooling['sources']:
        source = args.libevent_source if item['name'].startswith('libevent') else args.tmux_source
        loopback.require(hashlib.sha256(source.read_bytes()).hexdigest() == item['sha256'],
                         '公开 tmux 依赖源码摘要不符')
    tmux_sha = hashlib.sha256(args.tmux.read_bytes()).hexdigest()
    loopback.require(tmux_sha == args.expected_tmux_sha256
        and subprocess.check_output([str(args.tmux),'-V'],text=True).strip() == 'tmux 3.7c',
        '本轮私有 tmux 二进制版本或摘要不符')
    return {'codex_version':'codex-cli 0.156.1','codex_sha256':package['files']['bin/codex'][1],
            'tmux_version':'tmux 3.7c','tmux_sha256':tmux_sha,
            'tooling_source_sha256':{item['name']:item['sha256'] for item in tooling['sources']}}


def validate_case(case, entry):
    reports = Path(entry['reports'])
    required = {'native-start','native-exit','SessionStart','UserPromptSubmit','inner-capture'}
    native = {path.stem: json.loads(path.read_text()) for path in reports.glob('*.json')}
    loopback.require(set(native) == required, '缺少原生 hook 或 pane 采集记录')
    loopback.require(case['ssh_exit'] == 0 and native['native-exit']['exit_code'] == 0
        and not native['native-exit']['forced'], '原生 Codex 或 SSH 未正常退出')
    submit = native['UserPromptSubmit']['input']
    session, turn = submit.get('session_id'), submit.get('turn_id')
    loopback.require(submit.get('prompt') == loopback.PROMPT and session and turn,
                     '原生提示或 session/turn 不符')
    codex_pid = native['native-start']['codex_pid']
    for name in ('SessionStart','UserPromptSubmit'):
        hook = native[name]
        loopback.require(hook['input']['session_id'] == session and hook['input']['cwd'] == entry['work']
            and hook['reference_exit'] == 0 and hook['reference_stdout'] == ''
            and hook['reference_stderr'] == '' and hook['environment']['PLUGIN_ROOT'] == case['native_cache']
            and hook['environment']['TMUX_PANE'] == native['native-start']['tmux_pane']
            and codex_pid in {item['pid'] for item in hook['ancestry']},
            '原生 Codex hook 身份或参考通知发送不符')
    raw = (reports/'inner-pane.raw').read_bytes()
    case['dual_end_receipt'] = v03.validate_inner_outer(case, native, native['inner-capture'], raw)
    plain = re.sub(rb'\x1b\[[0-?]*[ -/]*[@-~]', b'',
                   base64.b64decode(case['ssh_stdout_base64'])).decode(errors='replace')
    loopback.require(native['UserPromptSubmit']['block_output'] ==
        {'continue':False,'stopReason':entry['environment']['INFINISHELL_SSH_PROBE_TOKEN']}
        and entry['environment']['INFINISHELL_SSH_PROBE_TOKEN'] in plain,
        '未观察到 TUI 对本次原生 hook 阻断的回应')
    case['inner_pane_base64'] = base64.b64encode(raw).decode()
    case['native'] = native
    case['passed'] = True


def tmux_processes(socket_path, executable):
    output=subprocess.check_output(['/bin/ps','-axo','pid=,command='],text=True)
    return [int(fields[0]) for line in output.splitlines()
        if len(fields:=line.strip().split(maxsplit=1)) == 2
        and socket_path in fields[1] and str(executable) in fields[1]]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('codex','tmux','tmux-source','libevent-source','private-output','safe-output'):
        parser.add_argument('--'+name,required=True,type=Path)
    parser.add_argument('--expected-tmux-sha256',required=True)
    args=parser.parse_args()
    os.umask(0o077)
    tmp=require_private_tmp()
    repository=Path(__file__).resolve().parents[2]
    for name in ('codex','tmux','tmux_source','libevent_source'):
        setattr(args,name,getattr(args,name).resolve(strict=True))
    loopback.require(not args.private_output.exists() and not args.safe_output.exists(),
                     '收据目标已存在')
    inputs=verify_inputs(args,repository)
    model_requests=[]
    class RejectModel(BaseHTTPRequestHandler):
        def reject(self):
            model_requests.append({'method':self.command,'path':self.path})
            self.send_error(503,'No model is available')
        do_GET=do_POST=do_PUT=do_DELETE=do_PATCH=do_OPTIONS=do_HEAD=reject
        def log_message(self,*_args): pass
    provider=ThreadingHTTPServer(('127.0.0.1',0),RejectModel)
    thread=threading.Thread(target=provider.serve_forever,daemon=True);thread.start()
    report={'source_commit':subprocess.check_output(['git','-C',str(repository),'rev-parse','HEAD'],text=True).strip(),
        'source_tree_dirty':bool(subprocess.check_output(['git','-C',str(repository),'status','--porcelain'],text=True)),
        'target':'macos_loopback_ssh_codex_0.156.1_tmux_off','inputs':inputs,
        'credentials_provided':False,'model_lifecycle_verified':False,'product_ssh_ui_verified':False,
        'case':{'mode':'tmux-off','passed':False},'passed':False}
    with tempfile.TemporaryDirectory(prefix='v03-',dir=tmp) as temporary:
        root=Path(temporary);server=None
        entry=None
        try:
            home,work,reports=root/'home',root/'work',root/'reports'
            for path in (home/'.codex',work,reports): path.mkdir(parents=True)
            env={key:os.environ[key] for key in ('PATH','TMPDIR','LANG','LC_ALL') if key in os.environ}
            env['PATH']=str(args.tmux.parent)+os.pathsep+env.get('PATH','/usr/bin:/bin')
            env.update(HOME=str(home),CODEX_HOME=str(home/'.codex'),SHELL='/bin/bash',TERM='xterm-256color',
                WARP_CLI_AGENT_PROTOCOL_VERSION='1',WARP_CLIENT_VERSION='v03-loopback',
                TERM_PROGRAM='WarpTerminal',INFINISHELL_SSH_PROBE_PYTHON=str(Path(sys.executable).resolve()),
                INFINISHELL_SSH_PROBE_REPORTS=str(reports),INFINISHELL_SSH_PROBE_TOKEN=uuid.uuid4().hex,
                GIT_CONFIG_NOSYSTEM='1',GIT_TERMINAL_PROMPT='0')
            (home/'.codex/config.toml').write_text('cli_auth_credentials_store="file"\nmodel="gpt-6-luna"\n'
                'model_provider="isolated_ssh_probe"\napproval_policy="on-request"\nsandbox_mode="read-only"\n'
                '[model_providers.isolated_ssh_probe]\nname="Isolated SSH probe"\n'
                f'base_url="http://127.0.0.1:{provider.server_port}/v1"\nwire_api="responses"\n'
                'requires_openai_auth=false\nsupports_websockets=false\n')
            tmux_config=root/'tmux.conf';tmux_config.write_text('set -g status off\n'
                'set -g default-shell /bin/sh\nset -g allow-passthrough off\n')
            entry={'root':str(root),'work':str(work),'reports':str(reports),'environment':env,
                   'tmux_config':str(tmux_config),'socket':str(root/'tmux.sock')}
            loopback.HOOK_SOURCE=HOOK_SOURCE
            loopback.native_prepare(args,entry,report['case'])
            config=root/'probe.json';config.write_text(json.dumps({'cases':{'tmux-off':entry},
                'tmux':str(args.tmux),'codex':str(args.codex)}))
            remote_script=root/'remote.py';remote_script.write_text(instrument_remote())
            for name in ('host-key','client-key'):
                subprocess.run(['ssh-keygen','-q','-t','ed25519','-N','','-f',str(root/name)],check=True)
            authorized=root/'authorized_keys';authorized.write_text('restrict,pty '+(root/'client-key.pub').read_text());authorized.chmod(0o600)
            with socket.socket() as listener:
                listener.bind(('127.0.0.1',0));port=listener.getsockname()[1]
            known=root/'known_hosts';known.write_text(f'[127.0.0.1]:{port} '+(root/'host-key.pub').read_text())
            launcher=root/'force-command.sh';launcher.write_text('#!/bin/sh\nexec '+shlex.join([
                '/usr/bin/env','-i','PATH='+env['PATH'],'HOME='+str(home),str(Path(sys.executable).resolve()),
                str(remote_script),str(config)])+' "$SSH_ORIGINAL_COMMAND"\n');launcher.chmod(0o700)
            sshd=root/'sshd_config';sshd.write_text('\n'.join(['ListenAddress 127.0.0.1',f'Port {port}',
                f'HostKey {root}/host-key',f'PidFile {root}/sshd.pid',f'AuthorizedKeysFile {authorized}',
                'StrictModes yes','PasswordAuthentication no','KbdInteractiveAuthentication no','UsePAM no',
                'AllowUsers '+subprocess.check_output(['id','-un'],text=True).strip(),
                'PermitRootLogin no','AllowTcpForwarding no','X11Forwarding no','PermitTunnel no',
                'PrintMotd no','LogLevel ERROR',f'SetEnv HOME={home}',f'ForceCommand {launcher}'])+'\n')
            subprocess.run(['/usr/sbin/sshd','-t','-f',str(sshd)],check=True,capture_output=True)
            server=subprocess.Popen(['/usr/sbin/sshd','-D','-e','-f',str(sshd)],
                stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            deadline=time.monotonic()+4
            while True:
                loopback.require(server.poll() is None and time.monotonic()<deadline,'隔离 sshd 未就绪')
                try:
                    with socket.create_connection(('127.0.0.1',port),timeout=.1): break
                except OSError: time.sleep(.03)
            ssh=['ssh','-F','/dev/null','-o','BatchMode=yes','-o','IdentitiesOnly=yes',
                '-o','StrictHostKeyChecking=yes','-o','UserKnownHostsFile='+str(known),
                '-o','ConnectTimeout=3','-i',str(root/'client-key'),'-p',str(port),'-tt',
                subprocess.check_output(['id','-un'],text=True).strip()+'@127.0.0.1']
            report['sshd']={'pid':server.pid,'port':port,'isolated_identity':True,
                            'production_ssh_config_read':False,'production_credentials_read':False}
            case=report['case'];case.update(remote.capture_case([*ssh,'tmux-off'],env['INFINISHELL_SSH_PROBE_TOKEN']))
            loopback.require(not model_requests,'本机拒绝 provider 收到模型请求')
            validate_case(case,entry)
            report['passed']=case['passed']
        except Exception as error:
            report['error']={'type':type(error).__name__,'message':str(error)}
            if entry is not None:
                report['native_partial']={path.stem:json.loads(path.read_text())
                    for path in Path(entry['reports']).glob('*.json')}
        finally:
            if entry is not None:
                cleanup=subprocess.run([str(args.tmux),'-S',entry['socket'],'kill-server'],
                    capture_output=True,timeout=5)
                report['tmux_cleanup_command_exit']=cleanup.returncode
                deadline=time.monotonic()+2
                while (remaining:=tmux_processes(entry['socket'],args.tmux)) and time.monotonic()<deadline:
                    time.sleep(.02)
                report['tmux_server_pids_after_cleanup']=remaining
                report['tmux_server_gone']=not remaining
            if server is not None:
                if server.poll() is None: server.terminate()
                server.wait(timeout=5)
                report['isolated_sshd_exit']=server.returncode
            report['model_http_request_count']=len(model_requests)
            if entry is not None:
                native_path=Path(entry['reports'])/'native-start.json'
                if native_path.exists():
                    pid=json.loads(native_path.read_text())['codex_pid']
                    report['native_codex_pid']=pid
                    report['native_codex_process_gone']=subprocess.run(['/bin/ps','-p',str(pid)],
                        capture_output=True).returncode != 0
                    report['passed']=report['passed'] and report['native_codex_process_gone']
            report['passed']=report['passed'] and not model_requests and server is not None \
                and server.poll() is not None and report.get('tmux_server_gone') is True
            report=json.loads(json.dumps(report).replace(str(root),'<isolated-probe>'))
    report['temporary_directory_removed'] = not Path(temporary).exists()
    report['passed'] = report['passed'] and report['temporary_directory_removed']
    provider.shutdown();provider.server_close();thread.join(timeout=3)
    args.private_output.parent.mkdir(parents=True,exist_ok=True)
    with args.private_output.open('x') as target: json.dump(report,target,ensure_ascii=False,indent=2)
    safe={'source_commit':report['source_commit'],'source_tree_dirty':report['source_tree_dirty'],
          'target':report['target'],'inputs':report['inputs'],'passed':report['passed'],
          'credentials_provided':False,'model_http_request_count':report['model_http_request_count'],
          'isolated_sshd_exit':report.get('isolated_sshd_exit'),
          'native_codex_process_gone':report.get('native_codex_process_gone'),
          'tmux_cleanup_command_exit':report.get('tmux_cleanup_command_exit'),
          'tmux_server_gone':report.get('tmux_server_gone'),
          'dual_end_receipt':report['case'].get('dual_end_receipt'),
          'error':report.get('error'),'private_receipt_sha256':hashlib.sha256(args.private_output.read_bytes()).hexdigest(),
          'safe_receipt_contains_raw_terminal_bytes':False}
    if 'native' in report['case']:
        native=report['case']['native']
        reference_version=json.loads((repository/'app/assets/bundled/cli-agent-plugins/codex/source/plugins/warp'
            '/.codex-plugin/plugin.json').read_text())['version']
        safe['plugin']={'diagnostic_plugin_id':loopback.PLUGIN_ID,
            'reference_plugin_version':reference_version,'installed_cache_verified':True,
            'original_hook_exit_codes':{name:native[name]['reference_exit']
                for name in ('SessionStart','UserPromptSubmit')}}
        safe['pty']={'ssh_exit':report['case']['ssh_exit'],
            'pane_id':native['inner-capture']['pane'],
            'pane_tty':native['inner-capture']['pane_tty'],
            'capture_installed_before_codex_start':native['inner-capture']['installed_before_codex_start'],
            'allow_passthrough':native['inner-capture']['allow_passthrough']}
    with args.safe_output.open('x') as target: json.dump(safe,target,ensure_ascii=False,indent=2)
    print(json.dumps({'passed':report['passed'],'safe_output':str(args.safe_output),
                      'error':report.get('error')},ensure_ascii=False))
    if not report['passed']: raise SystemExit(1)


if __name__=='__main__': main()
