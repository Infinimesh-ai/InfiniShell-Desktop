#!/usr/bin/env python3
"""在本机私有短目录运行固定 Claude 父子 Skill 在线验收。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import uuid

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
import run_claude_adapter_live as base
from prepare_claude_cli import current_platform, verify_binary, verify_version
from run_claude_v2_write_cancel_live import verify_workdir

TEST = ('ai::cli_agent_runtime::coordinator::g10_fixed_policy_live_tests::'
        'real_claude_g10_fixed_skill_parent_child')
SCOPE = 'real_claude_g10_fixed_skill_parent_child'
VERSION = '2.1.280'
MODEL = 'claude-opus-5-5'


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def one(events, kind):
    found = [row for row in events if row.get('event') == kind]
    if len(found) != 1:
        raise ValueError(f'{kind} 数量不符')
    return found[0]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--workdir', type=Path, required=True)
    parser.add_argument('--test-binary', type=Path, required=True)
    parser.add_argument('--claude', type=Path, required=True)
    parser.add_argument('--supervisor', type=Path, required=True)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    if sys.platform != 'darwin':
        raise ValueError('仅支持本机 macOS 验收')
    workdir = verify_workdir(args.workdir)
    output_receipt = args.output or (workdir / 'g10-skill.safe.json')
    if output_receipt.exists() or output_receipt.parent != workdir:
        raise ValueError('安全收据必须是本轮目录内的新文件')
    sources = (args.test_binary, args.claude, args.supervisor)
    if any(path.is_symlink() or not path.is_file() for path in sources):
        raise ValueError('二进制来源不允许符号链接')
    inputs = workdir / 'inputs'
    inputs.mkdir(mode=0o700)
    test, claude = (inputs / name for name in ('warp-libtest', 'claude'))
    for source, target in zip(sources[:2], (test, claude)):
        shutil.copy2(source, target)
        target.chmod(0o700)
    source_app = args.supervisor.parents[2]
    if source_app.suffix != '.app' or args.supervisor != source_app / 'Contents/MacOS/infinishell':
        raise ValueError('宿主 worker 必须来自已签名应用包')
    private_app = inputs / 'InfiniShellParity.app'
    shutil.copytree(source_app, private_app)
    supervisor = private_app / 'Contents/MacOS/infinishell'
    subprocess.run(['/usr/bin/codesign', '-s', '-', '--force', '--timestamp=none', str(test)],
                   check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for binary in (test, claude):
        subprocess.run(['/usr/bin/codesign', '--verify', '--strict', str(binary)], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    subprocess.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', str(private_app)],
                   check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    original_claude_sha = sha(args.claude)
    if sha(claude) != original_claude_sha:
        raise ValueError('固定 Claude 副本摘要不符')
    native = verify_binary(claude, current_platform(), VERSION)
    version = verify_version(claude, workdir, VERSION)
    root = workdir / 'g10-skill'
    root.mkdir(mode=0o700)
    (root / 'project' / '.claude').mkdir(parents=True, mode=0o700)
    (root / '.infinishell-claude-g10-skill-probe').write_text(SCOPE)
    artifact = root / 'g10-skill.raw.ndjson'
    artifact.touch(mode=0o600, exist_ok=False)
    environment = base.authorized_default_account_environment(root)
    environment['TMPDIR'] = str(workdir)
    environment['TMP'] = str(workdir)
    environment['TEMP'] = str(workdir)
    auth = base.probe_authorized_default_account(claude, environment.copy(), root / 'project')
    profile = 'claude-g10-skill-' + uuid.uuid4().hex
    environment.update({
        'WARP_DATA_PROFILE': profile,
        'INFINISHELL_CLAUDE_LIVE_ROOT': str(root),
        'INFINISHELL_CLAUDE_LIVE_AUTH_MODE': 'authorized_default_account',
        'INFINISHELL_CLAUDE_LIVE_EXECUTABLE': str(claude),
        'INFINISHELL_CLAUDE_LIVE_EXPECTED_VERSION': VERSION,
        'INFINISHELL_CLAUDE_LIVE_STATE_PROFILE': profile,
        'INFINISHELL_CLAUDE_LIVE_ARTIFACT': str(artifact),
        'INFINISHELL_CLAUDE_LIVE_MODEL': MODEL,
        'INFINISHELL_CLI_SUPERVISOR_EXECUTABLE': str(supervisor),
    })
    output = root / 'libtest.raw.log'
    with output.open('wb') as stream:
        process = subprocess.Popen([str(test), TEST, '--exact', '--ignored', '--nocapture',
                                    '--test-threads=1'], cwd=REPO, env=environment,
                                   stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            returncode = process.wait(timeout=750)
            timed_out = False
        except subprocess.TimeoutExpired:
            timed_out = True
            process.kill()
            returncode = process.wait(timeout=20)
    events = [json.loads(line) for line in artifact.read_text().splitlines() if line.strip()]
    failure = None
    try:
        if timed_out or returncode or not re.search(
                r'test result: ok\. 1 passed; 0 failed; 0 ignored;',
                output.read_text(errors='replace')):
            raise ValueError('真实 libtest 未成功')
        if one(events, 'acceptance_started')['scope'] != SCOPE:
            raise ValueError('开始范围不符')
        done = one(events, 'acceptance_passed')
        if any(row.get('event') == 'acceptance_failed' for row in events):
            raise ValueError('真实链存在失败状态')
        for key in ('parent_child_ceiling_verified', 'native_skill_deny_verified',
                    'native_skill_allow_verified', 'parent_ceiling_derive_rejected',
                    'real_parent_overbound_run_agents_verified', 'all_runtime_hosts_cleaned'):
            if done.get(key) is not True:
                raise ValueError(f'{key} 缺失')
        decisions = [row.get('decision') for row in events
                     if row.get('event') == 'native_approval_decision']
        if decisions != ['AllowOnce', 'DenyOnce', 'AllowOnce']:
            raise ValueError('原生审批顺序不符')
        saved = one(events, 'g10_saved_chain_verified')
        if not all(saved.get(key) is True for key in
                   ('parent_ceiling_saved', 'child_profile_subset', 'skill_deny_resolved',
                    'skill_allow_resolved', 'child_result_contains_marker',
                    'parent_ceiling_derive_rejected')):
            raise ValueError('持久父子结果不符')
        submitted = one(events, 'overbound_native_submit_accepted')
        approved = one(events, 'overbound_native_approval_allowed')
        requested = one(events, 'overbound_native_tool_requested')
        rejected = one(events, 'overbound_native_rejection_verified')
        if not (events.index(saved) < events.index(submitted) < events.index(approved)
                < events.index(requested) < events.index(rejected)
                and submitted.get('parent_task_id') == rejected.get('parent_task_id')
                and submitted.get('generation') == rejected.get('generation') == 2
                and submitted.get('unselected_skill') == 'beta'
                and approved.get('exact_scope_verified') is True
                and rejected.get('native_request_verified') is True
                and rejected.get('persisted_error_verified') is True
                and rejected.get('new_child_count') == 0):
            raise ValueError('原生越界请求或持久拒绝证据不符')
        cleanup = [row for row in events if row.get('event') == 'cleanup_confirmed']
        if len(cleanup) != 2 or len({row.get('task_id') for row in cleanup}) != 2:
            raise ValueError('父子清理收据数量不符')
        if any(row['receipt'].get('native_process') != 'exited'
               or not row['receipt'].get('native_cleanup_sha256')
               or not row['receipt'].get('adapter_task_terminated')
               or not row['receipt'].get('adapter_succeeded')
               or not row['receipt'].get('event_journal_completed') for row in cleanup):
            raise ValueError('父子原生退出未确认')
    except (ValueError, KeyError, TypeError) as error:
        failure = str(error)
    summary = {
        'scope': SCOPE, 'passed': failure is None, 'failure': failure,
        'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO,
                                                  text=True).strip(),
        'source_dirty': bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=REPO)),
        'state_profile': profile, 'test_pid': process.pid,
        'cli_version': version, 'cli_binary_sha256': native['sha256'],
        'model': MODEL, 'authorized_account_logged_in': auth['loggedIn'],
        'test_binary_sha256': sha(test), 'supervisor_sha256': sha(supervisor),
        'raw_event_sha256': sha(artifact), 'raw_log_sha256': sha(output),
        'event_count': len(events), 'test_exit_code': returncode, 'timed_out': timed_out,
        'native_decisions': [str(value) for value in decisions] if failure is None else [],
        'parent_child_results_verified': failure is None,
        'native_exit_receipts': len(cleanup) if failure is None else 0,
        'real_gui_verified': False, 'real_parent_overbound_run_agents_verified': failure is None,
        'http_request_count_verified': False,
    }
    output_receipt.write_text(json.dumps(summary, indent=2, ensure_ascii=False) + '\n')
    output_receipt.chmod(0o600)
    print('G10 固定技能父子真实链：' + ('通过' if failure is None else '未通过'))
    print('安全收据：' + str(output_receipt))
    return 0 if failure is None else 1


if __name__ == '__main__':
    try:
        sys.exit(main())
    except Exception as error:
        print('G10 运行前置或执行器失败：' + type(error).__name__)
        sys.exit(2)
