#!/usr/bin/env python3
"""固定来源的真实文件事务回归；不把本文件断言称为原生 CLI 验收。"""

import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest

import codex_persistent_source as source
from apply_notification_patch import default_bundle


class SourceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.home = Path(self.temporary.name).resolve()
        self.bundle = default_bundle()

    def previous(self, revision=3):
        metadata, data = source.previous_revision(self.bundle, revision)
        root = source.source_path(self.home, metadata)
        shutil.copytree(self.bundle / 'codex/source', root)
        # 先恢复冻结的 rev6 整个插件，再应用更早版本的明确差异。
        for path in root.rglob('*'):
            if path.is_file() and path.relative_to(root).as_posix() not in metadata['files']:
                path.unlink()
        for name in metadata['files']:
            if name.startswith('plugins/warp/'):
                relative = name.removeprefix('plugins/warp/')
                original = self.bundle / 'codex/revisions/rev6' / relative
                historical = self.bundle / f'codex/revisions/rev{revision}' / relative
                selected = historical if historical.is_file() else original
                (root / name).write_bytes(selected.read_bytes())
        if revision < 6:
            (root / 'plugins/warp/scripts/build-payload.sh').write_bytes(
                (self.bundle / 'codex/revisions/rev5/scripts/build-payload.sh').read_bytes())
        (root.parent / 'SOURCE_METADATA.json').write_bytes(data)
        source.verify_previous(self.home, self.bundle, revision)
        cache = self.home / 'plugins/cache/codex-warp/warp/0.4.0'
        shutil.copytree(root / 'plugins/warp', cache)
        initial = ('[marketplaces.codex-warp]\nsource_type="local"\nsource=' + json.dumps(str(root)) +
                   '\n[plugins."warp@codex-warp"]\nenabled=true\n'
                   '[plugins."orchestration@codex-warp"]\nenabled=false\n'
                   '[hooks.state.user]\ntrusted_hash="user-owned"\n').encode()
        (self.home / 'config.toml').write_bytes(initial)
        return root, initial

    def revision_transaction(self, revision=3):
        previous, initial = self.previous(revision)
        root = source.materialize(self.home, self.bundle)
        stage = self.home / 'transaction'
        shutil.copytree(root / 'plugins/warp', stage / 'codex/plugins/cache/codex-warp/warp/0.4.0')
        final = initial.replace(json.dumps(str(previous)).encode(), json.dumps(str(root)).encode())
        return previous, stage, initial, final

    def test_exact_rev3_upgrade_keeps_old_tree_trust_and_disabled_orchestration(self):
        previous, stage, initial, final = self.revision_transaction()
        old = source.tree(previous)
        self.assertFalse(source.validate_source(self.home, source.config(self.home)[1], self.bundle))
        source.validate_caches(self.home, self.bundle)
        source.commit(self.home, stage, initial, final, self.bundle)
        self.assertEqual(source.config(self.home)[0], final)
        self.assertTrue(source.validate_source(self.home, source.config(self.home)[1], self.bundle))
        self.assertEqual(source.tree(previous), old)
        source.verify_previous_cache(stage / 'previous-cache/0.4.0', self.bundle)

    def test_rev3_failure_restores_original_pointer_and_cache(self):
        previous, stage, initial, final = self.revision_transaction()
        old = source.tree(previous)
        def fail_after_config(step):
            if step == 2:
                raise OSError('注入 rev3 配置提交后故障')
        with self.assertRaises(OSError):
            source.commit(self.home, stage, initial, final, self.bundle, fail_after_config)
        self.assertEqual(source.config(self.home)[0], initial)
        self.assertEqual(source.tree(previous), old)
        source.verify_previous_cache(self.home / 'plugins/cache/codex-warp/warp/0.4.0', self.bundle)

    def test_rev3_modified_source_and_mixed_cache_are_rejected_without_writes(self):
        root, initial = self.previous()
        path = root / 'plugins/warp/scripts/on-prompt-submit.sh'
        original = path.read_bytes()
        path.write_bytes(b'user modification')
        with self.assertRaises(ValueError):
            source.validate_source(self.home, source.config(self.home)[1], self.bundle)
        self.assertEqual(path.read_bytes(), b'user modification')
        path.write_bytes(original)
        (self.home / 'plugins/cache/codex-warp/warp/0.4.0/scripts/warp-notify.sh').write_bytes(
            (self.bundle / 'codex/scripts/warp-notify.sh').read_bytes())
        with self.assertRaises(ValueError):
            source.validate_caches(self.home, self.bundle)
        self.assertEqual(source.config(self.home)[0], initial)

    def prepared(self, existing=False):
        root = source.materialize(self.home, self.bundle)
        stage = self.home / 'transaction'
        staged_cache = stage / 'codex/plugins/cache/codex-warp/warp/0.4.0'
        staged_cache.parent.mkdir(parents=True)
        shutil.copytree(root / 'plugins/warp', staged_cache)
        initial = b'approval_policy="on-request"\n'
        (self.home / 'config.toml').write_bytes(initial)
        final = initial + ('[marketplaces.codex-warp]\nsource_type="local"\nsource=' + json.dumps(str(root)) +
                           '\n[plugins."warp@codex-warp"]\nenabled=true\n').encode()
        target = self.home / 'plugins/cache/codex-warp/warp'
        if existing:
            shutil.copytree(staged_cache.parent, target)
        return stage, initial, final, target

    def test_whole_bundle_and_reviewed_additions_are_pinned(self):
        metadata, _, expected = source.source_bundle(self.bundle)
        self.assertEqual(len(expected), 38)
        changed = {name for name, entry in metadata['files'].items() if entry['sha256'] != entry['upstream_sha256']}
        self.assertEqual(changed, {'plugins/warp/hooks/hooks.json', 'plugins/warp/scripts/build-payload.sh',
                                 'plugins/warp/scripts/on-prompt-submit.sh', 'plugins/warp/scripts/on-stop.sh', 'plugins/warp/scripts/warp-notify.sh',
                                 'plugins/warp/scripts/on-notification.ps1', 'plugins/warp/scripts/warp-notify.ps1'})
        root = source.materialize(self.home, self.bundle)
        previous = (root / 'plugins/warp/scripts/on-stop.sh').stat().st_mtime_ns
        self.assertEqual(source.materialize(self.home, self.bundle), root)
        self.assertEqual((root / 'plugins/warp/scripts/on-stop.sh').stat().st_mtime_ns, previous)

    def test_exact_rev6_upgrade_and_failures_preserve_source_cache_and_trust(self):
        for failing_step in (None, 1, 2):
            with self.subTest(failing_step=failing_step), tempfile.TemporaryDirectory() as temporary:
                self.home = Path(temporary).resolve()
                previous, stage, initial, final = self.revision_transaction(6)
                old = source.tree(previous)
                cache = self.home / 'plugins/cache/codex-warp/warp/0.4.0'
                old_cache = source.tree(cache)
                self.assertFalse(source.validate_source(self.home, source.config(self.home)[1], self.bundle))
                source.validate_caches(self.home, self.bundle)
                def maybe_fail(step):
                    if step == failing_step:
                        raise OSError('注入 rev6 升级提交故障')
                if failing_step is None:
                    source.commit(self.home, stage, initial, final, self.bundle, maybe_fail)
                    self.assertEqual(source.config(self.home)[0], final)
                    self.assertEqual(source.tree(stage / 'previous-cache/0.4.0'), old_cache)
                    self.assertTrue((cache / 'scripts/on-notification.ps1').is_file())
                    self.assertTrue((cache / 'scripts/warp-notify.ps1').is_file())
                else:
                    with self.assertRaises(OSError):
                        source.commit(self.home, stage, initial, final, self.bundle, maybe_fail)
                    self.assertEqual(source.config(self.home)[0], initial)
                    self.assertEqual(source.tree(cache), old_cache)
                self.assertEqual(source.tree(previous), old)
                self.assertEqual(source.config(self.home)[1]['hooks']['state']['user']['trusted_hash'], 'user-owned')
                self.assertFalse(source.config(self.home)[1]['plugins']['orchestration@codex-warp']['enabled'])

    def test_rev6_partial_addition_is_rejected_without_modifying_configuration(self):
        _, initial = self.previous(6)
        target = self.home / 'plugins/cache/codex-warp/warp/0.4.0/scripts/on-notification.ps1'
        target.write_bytes((self.bundle / 'codex/scripts/on-notification.ps1').read_bytes())
        with self.assertRaises(ValueError):
            source.validate_caches(self.home, self.bundle)
        self.assertEqual(source.config(self.home)[0], initial)

    def test_fixture_copy_checks_complete_tree_and_never_overwrites_existing_target(self):
        target = self.home / 'fixture'
        source.materialize_plugin_fixture(self.bundle, target)
        self.assertEqual(source.tree(target), source.tree(self.bundle / 'codex/source/plugins/warp'))
        with self.assertRaises(ValueError):
            source.materialize_plugin_fixture(self.bundle, target)
        for name in ('on-notification.ps1', 'warp-notify.ps1'):
            with self.subTest(name=name):
                original = (target / 'scripts' / name).read_bytes()
                (target / 'scripts' / name).unlink()
                metadata, _ = source.bundle_data(self.bundle, 'codex')
                with self.assertRaises(ValueError):
                    source.validate_tree(target, '0.4.0', metadata)
                (target / 'scripts' / name).write_bytes(original)

    def test_damaged_orchestration_source_is_never_overwritten(self):
        root = source.materialize(self.home, self.bundle)
        target = root / 'plugins/orchestration/scripts/on-stop.sh'
        target.write_text('用户编排逻辑')
        with self.assertRaises(ValueError):
            source.materialize(self.home, self.bundle)
        self.assertEqual(target.read_text(), '用户编排逻辑')

    def test_empty_marketplace_is_not_absent_and_config_bytes_distinguish_creation(self):
        self.assertFalse(source.validate_source(self.home, {}, self.bundle))
        with self.assertRaises(ValueError):
            source.validate_source(self.home, {'marketplaces': {'codex-warp': {}}}, self.bundle)
        original_bytes, original = source.config(self.home)
        self.assertIsNone(original_bytes)
        self.assertEqual(original, {})
        (self.home / 'config.toml').write_bytes(b'')
        created_bytes, created = source.config(self.home)
        self.assertEqual(created, original)
        self.assertNotEqual(created_bytes, original_bytes)

    def test_invalid_parent_and_target_types_fail_before_writing(self):
        for content in ("plugins='x'\n", "marketplaces='x'\n",
                        "[plugins]\n'warp@codex-warp'=true\n", "[marketplaces]\ncodex-warp=9\n",
                        "[plugins.'warp@codex-warp']\nenabled='false'\n"):
            with self.subTest(content=content):
                path = self.home / 'config.toml'
                path.write_text(content)
                with self.assertRaises(ValueError):
                    source.config(self.home)
                self.assertEqual(path.read_text(), content)

    def test_unknown_local_marketplace_rejected(self):
        settings = {'marketplaces': {'codex-warp': {'source_type': 'local', 'source': '/user-owned'}}}
        with self.assertRaises(ValueError):
            source.validate_source(self.home, settings, self.bundle)

    @unittest.skipUnless(os.name == 'posix', 'Windows 不使用 Unix 模式位契约')
    def test_owned_source_mode_change_is_rejected_and_preserved(self):
        root = source.materialize(self.home, self.bundle)
        target = root / 'plugins/orchestration/scripts/on-stop.sh'
        target.chmod(0o600)
        with self.assertRaises(ValueError):
            source.materialize(self.home, self.bundle)
        self.assertEqual(target.stat().st_mode & 0o777, 0o600)

    def test_concurrent_disable_before_write_keeps_disabled_and_previous_cache(self):
        stage, initial, final, target = self.prepared(existing=True)
        before = source.tree(target)
        disabled = initial + b'[plugins."warp@codex-warp"]\nenabled=false\n'
        def mutate(step):
            if step == 1:
                (self.home / 'config.toml').write_bytes(disabled)
        with self.assertRaises(ValueError):
            source.commit(self.home, stage, initial, final, self.bundle, mutate)
        self.assertEqual((self.home / 'config.toml').read_bytes(), disabled)
        self.assertEqual(source.tree(target), before)

    def test_after_commit_external_config_change_is_preserved_and_failure_is_explicit(self):
        stage, initial, final, target = self.prepared(existing=True)
        before = source.tree(target)
        concurrent = final + b'[marketplaces.other]\nsource_type="local"\nsource="/other"\n'
        def mutate(step):
            if step == 2:
                (self.home / 'config.toml').write_bytes(concurrent)
        with self.assertRaisesRegex(ValueError, '恢复未完成'):
            source.commit(self.home, stage, initial, final, self.bundle, mutate)
        self.assertEqual((self.home / 'config.toml').read_bytes(), concurrent)
        self.assertEqual(source.tree(target), before)
        self.assertTrue((stage / 'state.json').exists())

    def test_unknown_cache_change_keeps_backup_and_new_user_content(self):
        stage, initial, final, target = self.prepared(existing=True)
        before = source.tree(target)
        def mutate(step):
            if step == 2:
                (target / '0.4.0/scripts/on-stop.sh').write_text('并发内容')
        with self.assertRaisesRegex(ValueError, '恢复未完成'):
            source.commit(self.home, stage, initial, final, self.bundle, mutate)
        self.assertEqual((target / '0.4.0/scripts/on-stop.sh').read_text(), '并发内容')
        self.assertEqual(source.tree(stage / 'previous-cache'), before)
        self.assertEqual((self.home / 'config.toml').read_bytes(), initial)

    def test_scope_comparison_rejects_native_side_effects(self):
        before = {'hooks': {'state': {'x': {'trusted_hash': 'retained'}}},
                  'plugins': {'orchestration@codex-warp': {'enabled': False}}}
        after = json.loads(json.dumps(before))
        after['plugins'][source.PLUGIN] = {'enabled': True}
        after['marketplaces'] = {source.MARKETPLACE: {'source_type': 'local'}}
        self.assertTrue(source.unchanged_except_target(before, after))
        after['hooks']['state']['x']['trusted_hash'] = 'changed'
        self.assertFalse(source.unchanged_except_target(before, after))


if __name__ == '__main__':
    unittest.main()
