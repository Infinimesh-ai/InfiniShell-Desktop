"""验证真实 innounp 目录格式及拒绝路径逃逸的边界。"""
import importlib.util
from pathlib import Path
import sys
import unittest

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


if __name__ == '__main__':
    unittest.main(verbosity=2)
