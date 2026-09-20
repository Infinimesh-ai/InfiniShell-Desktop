#!/usr/bin/env python3
"""仅显式 --execute 时强杀编译后的 Rust 文件事务测试；不运行 Codex CLI。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time

PREFIX = 'terminal::cli_agent_sessions::plugin_manager::codex_source::tests::recovery_tests::'
STEPS = (0, 3, 4, 1, 5, 2, 6)


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def save(stream, value):
    stream.seek(0);json.dump(value,stream,ensure_ascii=False,indent=2);stream.write('\n');stream.truncate();stream.flush();os.fsync(stream.fileno())


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--test-binary',type=Path)
    parser.add_argument('--test-binary-sha256')
    parser.add_argument('--source-manifest',type=Path)
    parser.add_argument('--source-manifest-sha256')
    parser.add_argument('--output',type=Path)
    parser.add_argument('--execute',action='store_true')
    args=parser.parse_args()
    if not args.execute:
        print(json.dumps({'status':'prepared','execute_requested':False,'failure_windows':len(STEPS),'native_cli_inputs':0,'rust_executed':False}));return 0
    if not all((args.test_binary,args.test_binary_sha256,args.source_manifest,args.source_manifest_sha256,args.output)):
        parser.error('执行时必须指定已编译测试二进制与 source manifest 的实际路径和 SHA256，以及唯一输出路径')
    for path,expected in ((args.test_binary,args.test_binary_sha256),(args.source_manifest,args.source_manifest_sha256)):
        if path.is_symlink() or not path.is_file() or sha(path)!=expected:raise ValueError('bound_material_mismatch')
    descriptor=os.open(args.output,os.O_CREAT|os.O_EXCL|os.O_RDWR|getattr(os,'O_NOFOLLOW',0),0o600)
    report={'status':'reserved','passed':False,'test_binary_sha256':args.test_binary_sha256,
            'source_manifest_sha256':args.source_manifest_sha256,'platform':os.name,'rows':[],
            'native_cli_executed':False,'credentials_read':False,'power_loss_verified':False}
    with os.fdopen(descriptor,'w+') as out:
        save(out,report)
        try:
            for step in STEPS:
                root=Path(tempfile.mkdtemp(prefix='infinishell-codex-journal-crash-')).resolve();root.chmod(0o700)
                (root/'.fixture-owner').write_bytes(b'Codex journal SIGKILL fixture v1\n')
                (root/'.fixture-owner').chmod(0o600)
                env={'PATH':os.defpath,'HOME':str(root),'USERPROFILE':str(root),'TMPDIR':str(root),
                     'TMP':str(root),'TEMP':str(root),'LANG':'en_US.UTF-8','LC_ALL':'en_US.UTF-8',
                     'CODEX_HOME':str(root/'unused-auth-home'),'INFINISHELL_CODEX_JOURNAL_CRASH_ROOT':str(root),
                     'INFINISHELL_CODEX_JOURNAL_CRASH_STEP':str(step)}
                if os.name=='nt':
                    for key in ('SystemRoot','WINDIR'):
                        if key in os.environ:env[key]=os.environ[key]
                row={'step':step,'workspace':str(root),'passed':False};report['rows'].append(row);save(out,report)
                for name in ('interrupted_install_process_fixture','recover_interrupted_install_process_fixture'):
                    listed=subprocess.run([str(args.test_binary),PREFIX+name,'--exact','--ignored','--list'],cwd=root,env=env,capture_output=True,timeout=30)
                    if listed.returncode!=0 or listed.stdout.decode().splitlines()!=[PREFIX+name+': test', '', '1 test, 0 benchmarks']:raise ValueError('test_entry_missing')
                with os.fdopen(os.open(root/'private-child.log',os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600),'wb') as log:
                    child=subprocess.Popen([str(args.test_binary),PREFIX+'interrupted_install_process_fixture','--exact','--ignored','--nocapture','--test-threads=1'],cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT)
                    try:
                        deadline=time.monotonic()+45
                        while not (root/'checkpoint').is_file() and child.poll() is None and time.monotonic()<deadline:time.sleep(.02)
                        if child.poll() is not None or not (root/'checkpoint').is_file() or (root/'checkpoint').read_text()!=str(step):raise ValueError('checkpoint_not_observed')
                        child.kill();code=child.wait(timeout=20)
                        row['termination_method']='TerminateProcess' if os.name=='nt' else 'SIGKILL'
                        row['killed_exit_code']=code
                        if code!=(1 if os.name=='nt' else -signal.SIGKILL):raise ValueError('kill_not_confirmed')
                    finally:
                        if child.poll() is None:child.kill();child.wait(timeout=20)
                with os.fdopen(os.open(root/'private-recovery.log',os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600),'wb') as log:
                    result=subprocess.run([str(args.test_binary),PREFIX+'recover_interrupted_install_process_fixture','--exact','--ignored','--nocapture','--test-threads=1'],cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=30)
                row['recovery_exit_code']=result.returncode;row['passed']=result.returncode==0;save(out,report)
                if not row['passed']:raise ValueError('rust_recovery_failed')
            for path,expected in ((args.test_binary,args.test_binary_sha256),(args.source_manifest,args.source_manifest_sha256)):
                if sha(path)!=expected:raise ValueError('bound_material_changed')
            report.update(status='completed',passed=True,bound_materials_unchanged=True);save(out,report)
        except BaseException as error:
            report.update(status='failed',passed=False,error_type=type(error).__name__);save(out,report)
            print(json.dumps({'passed':False,'report':str(args.output),'error_type':type(error).__name__}));return 1
    print(json.dumps({'passed':True,'report':str(args.output),'failure_windows':len(STEPS),'native_cli_executed':False}));return 0


if __name__=='__main__':raise SystemExit(main())
