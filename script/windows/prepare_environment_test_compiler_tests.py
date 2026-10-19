"""固定编译器准备的离线安全边界测试；不下载或运行原生工具。"""
import ast
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import stat
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import zipfile

sys.dont_write_bytecode = True
_path = Path(__file__).with_name('prepare_environment_test_compiler.py')
_spec = importlib.util.spec_from_file_location('environment_compiler', _path)
controller = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(controller)


class CompilerContractTests(unittest.TestCase):
    def test_frozen_tool_matches_existing_release_verifier(self):
        text = _path.with_name('verify_release_installer.ps1').read_text(encoding='utf-8-sig')
        source = text.split("$acquirer = @'\n", 1)[1].split("\n'@", 1)[0]
        assignments = [node for node in ast.parse(source).body if isinstance(node, ast.Assign)]
        tool = next(ast.literal_eval(node.value) for node in assignments
                    if any(isinstance(target, ast.Name) and target.id == 'TOOL' for target in node.targets))
        self.assertEqual(controller.TOOL, tool)
        self.assertIs(controller.COMPILER, controller._reissue.COMPILER)

    def test_actual_listing_selects_only_application_members(self):
        text = '\n'.join([
            'Inno Setup archive: compiler-distribution.exe',
            'Inno Setup version detected: 6.7.0 (Unicode)',
            r'{tmp}\isdonate.bmp', r'{app}\ISCC.exe',
            r'{app}\Examples\MyDll\C#\MyDll.cs', 'install_script.iss',
        ])
        self.assertEqual(controller.compiler_members(text),
                         {'{app}/ISCC.exe', '{app}/Examples/MyDll/C#/MyDll.cs'})

    def test_rejects_unsafe_or_ambiguous_compiler_members(self):
        for member in [r'{app}\..\outside', r'{app}\NUL.txt', r'{app}\file:stream',
                       r'{app}\trailing.\file', r'{app}\wild*.txt', r'C:\outside',
                       r'{app}\COM1', r'{app}\iscc.exe', '{app}/ISCC.exe']:
            with self.subTest(member=member), self.assertRaises(RuntimeError):
                controller.compiler_members('{app}\\ISCC.exe\n' + member)
        with self.assertRaises(RuntimeError):
            controller.compiler_members(r'{app}\license.txt')

    def test_rejects_http_redirect(self):
        with self.assertRaisesRegex(RuntimeError, 'HTTPS'):
            controller.HttpsRedirect().redirect_request(None, None, 302, '', {}, 'http://example.test/tool')


class CompilerFilesystemTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='infinishell-compiler-test-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()

    def test_existing_output_is_preserved(self):
        marker = self.root / 'keep.txt'
        marker.write_bytes(b'keep')
        with patch.object(controller, 'download') as download, self.assertRaises(RuntimeError):
            controller.prepare(self.root)
        download.assert_not_called()
        self.assertEqual(marker.read_bytes(), b'keep')

    def test_rejects_parent_traversal(self):
        with self.assertRaises(RuntimeError):
            controller.physical(self.root / '..' / 'elsewhere')

    def test_rejects_reparse_attribute(self):
        original = Path.lstat
        def lstat(path):
            if path == self.root:
                return SimpleNamespace(st_mode=stat.S_IFDIR | 0o700, st_file_attributes=0x400)
            return original(path)
        with patch.object(Path, 'lstat', lstat), self.assertRaises(RuntimeError):
            controller.physical(self.root / 'new')

    def test_rejects_symbolic_link_in_ancestor(self):
        link = self.root / 'link'
        try:
            link.symlink_to(self.root, target_is_directory=True)
        except OSError:
            self.skipTest('当前测试账号不能创建符号链接')
        with self.assertRaises(RuntimeError):
            controller.physical(link / 'new')

    def test_download_bounds_and_digest(self):
        for payload, size, digest in [(b'too long', 2, '0' * 64), (b'short', 8, '0' * 64),
                                      (b'ab', 2, '0' * 64)]:
            target = self.root / ('download-' + str(size) + '-' + str(len(payload)))
            response = io.BytesIO(payload)
            response.url = 'https://example.test/fixed'
            opener = SimpleNamespace(open=lambda *args, **kwargs: response)
            with self.subTest(payload=payload), patch.object(controller.urllib.request, 'build_opener', return_value=opener):
                with self.assertRaises(RuntimeError):
                    controller.download(response.url, target, size, digest)
                self.assertLessEqual(target.stat().st_size, size)

    def tool_zip(self, filename, extra=None, special=None):
        target = self.root / filename
        with zipfile.ZipFile(target, 'w') as archive:
            for name in sorted(controller.TOOL_MEMBERS):
                info = zipfile.ZipInfo(name)
                if special == name:
                    info.create_system = 3
                    info.external_attr = (stat.S_IFLNK | 0o777) << 16
                archive.writestr(info, b'fixture')
            if extra:
                archive.writestr(extra, b'extra')
        return target

    def test_zip_rejects_extra_traversal_case_alias_and_link(self):
        for index, extra in enumerate(['../outside', 'INNOUNP.EXE', 'unexpected.txt']):
            archive = self.tool_zip(f'bad-{index}.zip', extra=extra)
            destination = self.root / f'bad-{index}'
            with self.subTest(extra=extra), self.assertRaises(RuntimeError):
                controller.unpack_tool(archive, destination)
            self.assertEqual(list(destination.iterdir()), [])
        with self.assertRaises(RuntimeError):
            controller.unpack_tool(self.tool_zip('link.zip', special='innounp.exe'), self.root / 'link-extract')

    def test_zip_rejects_wrong_executable_before_any_execution(self):
        with self.assertRaisesRegex(RuntimeError, 'SHA-256'):
            controller.unpack_tool(self.tool_zip('fixed-names.zip'), self.root / 'tool')

    def test_inventory_rejects_unlisted_files_and_directories(self):
        directory = self.root / 'compiler'
        (directory / '{app}').mkdir(parents=True)
        (directory / '{app}/ISCC.exe').write_bytes(b'fixture')
        expected = {'{app}/ISCC.exe'}
        self.assertEqual(set(controller.inventory(directory, expected)), expected)
        extra = directory / 'unexpected'
        extra.mkdir()
        with self.assertRaises(RuntimeError):
            controller.inventory(directory, expected)
        extra.rmdir()
        (directory / '{app}/extra.dll').write_bytes(b'extra')
        with self.assertRaises(RuntimeError):
            controller.inventory(directory, expected)

    def test_failed_preparation_has_no_success_path_and_preserves_evidence(self):
        output = self.root / 'attempt'
        with patch.object(controller, 'download', side_effect=RuntimeError('fixture failure')):
            with self.assertRaises(RuntimeError):
                controller.prepare(output)
        result = json.loads((output / 'result.json').read_text(encoding='utf-8'))
        self.assertEqual(result['status'], 'failed')
        self.assertNotIn('compiler_path', result)
        self.assertFalse(result['product_execution'])
        self.assertFalse(result['registry_modified'])

    def test_success_only_returns_verified_compiler_and_small_manifest(self):
        output = self.root / 'attempt'
        binary = b'compiler fixture; never executed'
        expected_hash = hashlib.sha256(binary).hexdigest()

        def download(url, path, size, digest):
            path.write_bytes(b'archive fixture; never executed')

        def unpack(archive, directory):
            directory.mkdir()

        def run(root, label, arguments):
            self.assertEqual(arguments[-1], r'{app}\*')
            if label == 'compiler-members':
                listing = root / 'listing.txt'
                listing.write_text(r'{app}\ISCC.exe', encoding='utf-8')
                return listing
            self.assertEqual(label, 'compiler-extract')
            self.assertIn('-d' + str(root / 'compiler'), arguments)
            app = root / 'compiler/{app}'
            app.mkdir()
            (app / 'ISCC.exe').write_bytes(binary)

        with patch.object(controller, 'download', side_effect=download), \
                patch.object(controller, 'unpack_tool', side_effect=unpack), \
                patch.object(controller, 'run_tool', side_effect=run) as runner, \
                patch.object(controller, 'ISCC_SIZE', len(binary)), \
                patch.dict(controller.COMPILER, {'iscc_sha256': expected_hash}):
            compiler = controller.prepare(output)
        self.assertEqual(runner.call_count, 2)
        self.assertEqual(compiler, output / 'compiler/{app}/ISCC.exe')
        result = json.loads((output / 'result.json').read_text(encoding='utf-8'))
        manifest = json.loads((output / 'compiler-files.json').read_text(encoding='utf-8'))
        self.assertEqual(result['status'], 'passed')
        self.assertEqual(result['compiler_path'], str(compiler))
        self.assertEqual(result['compiler_files_sha256'], controller.sha(output / 'compiler-files.json'))
        self.assertEqual(manifest['files']['{app}/ISCC.exe']['sha256'], expected_hash)

    def test_unverified_tool_is_never_executed(self):
        tool = self.root / 'tool'
        tool.mkdir()
        (tool / 'innounp.exe').write_bytes(b'untrusted')
        with patch.object(controller.subprocess, 'run') as run, self.assertRaises(RuntimeError):
            controller.run_tool(self.root, 'listing', [])
        run.assert_not_called()


if __name__ == '__main__':
    unittest.main(verbosity=2)
