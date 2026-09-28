#!/usr/bin/env python3
"""本机 V03 回环脚本的采集时序和私有进程身份回归。"""

import unittest
from unittest.mock import patch

import probe_v03_codex_tmux_local as probe


class LocalV03CodexTests(unittest.TestCase):
    def test_hook_and_remote_source_compile_with_capture_before_codex(self):
        compile(probe.HOOK_SOURCE, 'native_hook.py', 'exec')
        source=probe.instrument_remote()
        compile(source, 'remote.py', 'exec')
        self.assertLess(source.index("'pipe-pane','-O'"),
                        source.index("process=subprocess.Popen([config['codex']"))
        self.assertIn("option!='off'", source)
        self.assertIn("'inner-pane.raw'", source)

    def test_tmux_cleanup_selects_only_exact_private_executable_and_socket(self):
        binary='/private/tooling/bin/tmux'
        socket='/private/run/tmux.sock'
        listing='100 '+binary+' -S '+socket+' new-session\n'
        listing+='101 /usr/bin/tmux -S '+socket+' new-session\n'
        listing+='102 '+binary+' -S /other/run/tmux.sock new-session\n'
        with patch.object(probe.subprocess,'check_output',return_value=listing) as output:
            self.assertEqual(probe.tmux_processes(socket,binary),[100])
        output.assert_called_once_with(['/bin/ps','-axo','pid=,command='],text=True)

    def test_private_short_tmpdir_is_required(self):
        with patch.dict(probe.os.environ,{'TMPDIR':'/tmp/ordinary'},clear=False):
            with self.assertRaises(RuntimeError):
                probe.require_private_tmp()


if __name__=='__main__':
    unittest.main()
