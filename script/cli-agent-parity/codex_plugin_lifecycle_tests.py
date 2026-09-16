"""注册表探测的文件保真与操作边界回归；不将替身当作原生生命周期证据。"""

from pathlib import Path
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
from probe_codex_plugin_lifecycle import (INITIAL_CONFIG, registry_entry, require_same_tree,
                                         safe_command, tree_hashes, user_settings)


class LifecycleGuardTests(unittest.TestCase):
    def test_changed_missing_or_added_original_files_fail(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            original = root / 'plugin.json'
            original.write_bytes(b'original')
            before = tree_hashes(root)
            require_same_tree(root, before)
            original.write_bytes(b'changed')
            with self.assertRaises(ValueError):
                require_same_tree(root, before)
            original.write_bytes(b'original')
            extra = root / 'unexpected'
            extra.touch()
            with self.assertRaises(ValueError):
                require_same_tree(root, before)
            extra.unlink()
            original.unlink()
            with self.assertRaises(ValueError):
                require_same_tree(root, before)

    def test_settings_comparison_only_excludes_native_registry_fields(self):
        with tempfile.TemporaryDirectory() as temporary:
            config = Path(temporary) / 'config.toml'
            config.write_text(INITIAL_CONFIG)
            before = user_settings(config)
            config.write_text(INITIAL_CONFIG + '\n[plugins."warp@codex-warp"]\nenabled = false\n')
            self.assertEqual(before, user_settings(config))
            config.write_text(INITIAL_CONFIG.replace('on-request', 'never'))
            self.assertNotEqual(before, user_settings(config))

    def test_non_registry_entrypoints_are_rejected(self):
        for arguments in (['exec', 'hello'], ['app-server'], ['thread/start'], ['turn/start'],
                          ['login'], ['plugin', 'update'], ['plugin', 'disable', 'warp']):
            with self.subTest(arguments=arguments), self.assertRaises(ValueError):
                safe_command(arguments)
        safe_command(['plugin', 'add', 'warp@codex-warp', '--json'])
        safe_command(['plugin', 'disable', '--help'])

    def test_duplicate_registry_records_fail(self):
        entry = {'pluginId': 'warp@codex-warp'}
        self.assertIsNone(registry_entry({'installed': []}))
        self.assertEqual(registry_entry({'installed': [entry]}), entry)
        with self.assertRaises(ValueError):
            registry_entry({'installed': [entry, entry]})


if __name__ == '__main__':
    unittest.main()
