"""验证真实归档目录、路径边界及候选包独立桌面验收要求。"""
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
path = Path(__file__).with_name('reissue_release_installer.py')
spec = importlib.util.spec_from_file_location('reissue_controller', path)
controller = importlib.util.module_from_spec(spec)
spec.loader.exec_module(controller)


class ArchiveListingTests(unittest.TestCase):
    def test_actual_compiler_listing_header_and_unselected_members(self):
        # 首次远端失败使用的实际格式；-h 不隐藏这两行说明。
        text = r'''
Inno Setup archive:           compiler-distribution.exe
Inno Setup version detected:  6.7.0 (Unicode)

  {tmp}\isdonate.bmp
  {tmp}\isdonate_dark.bmp
  {app}\license.txt
  {app}\ISCC.exe
  {app}\Examples\MyDll\C#\MyDll.cs
  install_script.iss
'''
        self.assertEqual(controller.application_members(text), [
            r'{app}\license.txt', r'{app}\ISCC.exe', r'{app}\Examples\MyDll\C#\MyDll.cs'])

    def test_product_header_and_payload(self):
        text = 'Inno Setup archive: InfiniShellSetup.exe\nInno Setup version detected: 6.7.0 (Unicode)\n  {app}\\infinishell.exe\n'
        self.assertEqual(controller.application_members(text), [r'{app}\infinishell.exe'])

    def test_reject_unsafe_or_ambiguous_members(self):
        for text in [
            r'{app}\..\escape.exe', r'C:\escape.exe', r'{app}\file:stream',
            r'{tmp}\..\escape.exe', r'{commonappdata}\unexpected.exe',
            r'{app}\trailing.\file', '{app}\\ISCC.exe\n{app}\\iscc.exe',
            '{app}\\ISCC.exe\nunknown tool warning', '', r'{tmp}\only.bmp',
        ]:
            with self.subTest(text=text), self.assertRaises(RuntimeError):
                controller.application_members(text)


@unittest.skipUnless(os.name == 'nt', '原生文件身份检查仅用于 Windows')
class CandidateAcceptanceTests(unittest.TestCase):
    def run_candidate(self, same_installer, changed_payload=False):
        with tempfile.TemporaryDirectory(prefix='infinishell-reissue-test-') as directory:
            root = Path(directory).resolve()
            self.assertEqual(root.parent, Path(tempfile.gettempdir()).resolve())
            for name in ('compiler', 'evidence', 'candidate', 'original-payload/{app}', 'rebuilt-payload/{app}'):
                (root / name).mkdir(parents=True)
            for number in range(150):
                for folder in ('original-payload', 'rebuilt-payload'):
                    (root / folder / '{app}' / f'file{number}').write_bytes(b'unchanged application bytes')
            files = controller.inventory(root / 'original-payload/{app}')
            setup = root / 'candidate/InfiniShellSetup.exe'
            setup.write_bytes(b'installer fixture; never executed')
            actual = controller.sha(setup)
            accepted = actual if same_installer else '0' * 64
            records = {
                'source-inputs.json': {'files': []},
                'source-binding.json': {},
                'compiler-files.json': {'files': {}},
                'compiler-signature.json': {},
                'original-payload.json': {'files': files},
                'rebuilt-members.json': {'members': ['{app}/' + name for name in files]},
                'installer-signature.json': {'authenticode_status': 'NotSigned', 'installer_sha256': actual},
                'path-regression-binding.json': {'status': 'passed'},
            }
            for name, value in records.items():
                (root / 'evidence' / name).write_text(json.dumps(value), encoding='utf-8')
            if changed_payload:
                (root / 'rebuilt-payload/{app}/file0').write_bytes(b'changed payload')
            identity = {'release_run': 38537451605, 'tag_object': '1' * 40,
                        'sources': {'windows': {'artifact': {}}}, 'ci_manifest_sha256': '2' * 64}
            with patch.object(controller, 'verify_source', return_value=identity), patch.dict(os.environ, {'GITHUB_SHA': '3' * 40}):
                args = SimpleNamespace(output=root, verified=root, repo=root, accepted=accepted)
                if changed_payload:
                    with self.assertRaisesRegex(RuntimeError, '150'):
                        controller.finish(args)
                    self.assertFalse((root / 'delivery').exists())
                    return
                controller.finish(args)
            result = controller.read(root / 'delivery/reissue-binding.json')
            self.assertEqual(result['status'], 'remote_reissue_build_and_regression_passed')
            self.assertEqual(result['installer_matches_previous_acceptance'], same_installer)
            self.assertEqual(result['requires_new_desktop_acceptance'], not same_installer)
            self.assertEqual(result['desktop_six_stage_acceptance'], 'not_performed_by_this_job')
            self.assertEqual((root / 'delivery/InfiniShellSetup.exe').read_bytes(), setup.read_bytes())

    def test_new_installer_requires_its_own_desktop_acceptance(self):
        self.run_candidate(False)

    def test_identical_installer_does_not_fabricate_a_desktop_run(self):
        self.run_candidate(True)

    def test_changed_application_payload_blocks_delivery(self):
        self.run_candidate(False, changed_payload=True)


if __name__ == '__main__':
    unittest.main(verbosity=2)
