#!/usr/bin/env python3
"""运行真实 DEB 渲染及维护脚本，只把系统路径映射到临时目录。"""

import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest


REPO = Path(__file__).resolve().parents[2]


def executable(path, contents):
    path.write_text(contents)
    path.chmod(0o755)


class DebianMaintainerTests(unittest.TestCase):
    def test_generated_package_lifecycle(self):
        root = Path(tempfile.mkdtemp(prefix="deb-maintainers-"))
        fixture = root / "workspace"
        (fixture / "script/linux").mkdir(parents=True)
        shutil.copy2(REPO / "script/linux/bundle_deb", fixture / "script/linux/bundle_deb")
        shutil.copytree(REPO / "resources/linux/debian", fixture / "resources/linux/debian")
        # 不编译应用；真实打包脚本仍负责选择模板、变量替换及设置维护脚本权限。
        (fixture / "script/linux/bundle_install").write_text(":\n")
        tools = root / "tools"
        tools.mkdir()
        executable(tools / "fakeroot", '#!/usr/bin/env bash\nexec "$@"\n')
        native_dpkg = shutil.which("dpkg-deb")
        if sys.platform.startswith("linux"):
            self.assertIsNotNone(native_dpkg, "Linux 门禁必须使用真实 dpkg-deb")
        if native_dpkg is None:
            # macOS 只验证生成的脚本；Linux 必须从实际 DEB 重新提取维护脚本。
            executable(
                tools / "dpkg-deb",
                f"#!{sys.executable}\nimport pathlib, shutil, sys\n"
                "assert sys.argv[1] == '-b'\n"
                "shutil.copytree(pathlib.Path(sys.argv[2]) / 'DEBIAN', "
                "pathlib.Path(sys.argv[3]) / 'DEBIAN')\n",
            )
        variants = [
            ("app", "oss", "infinishell", "infinishell", "/opt/infinishell"),
            ("app", "stable", "warp", "warp-terminal", "/opt/warpdotdev/warp-terminal"),
            ("app", "preview", "warp-preview", "warp-terminal-preview", "/opt/warpdotdev/warp-terminal-preview"),
            ("cli", "oss", "infinishell", "infinishell-cli", "/opt/infinishell-cli"),
            ("cli", "stable", "oz", "oz-stable", "/opt/warpdotdev/oz-stable"),
            ("cli", "preview", "oz-preview", "oz-preview", "/opt/warpdotdev/oz-preview"),
        ]
        actions = ("remove", "purge", "upgrade", "failed-upgrade", "abort-install", "abort-upgrade", "disappear")
        states = ("owned-live", "owned-dangling", "foreign-link", "regular-file", "directory", "missing")
        checks = 0
        for artifact, channel, binary, package, opt in variants:
            variant = root / f"{artifact}-{channel}"
            output = variant / "output"
            output.mkdir(parents=True)
            environment = dict(os.environ, PATH=f"{tools}{os.pathsep}{os.environ['PATH']}")
            environment.update(
                WORKSPACE_ROOT_DIR=str(fixture), DIST_DIR=str(variant / "dist"),
                OUT_DIR=str(output), RELEASE_CHANNEL=channel, ARTIFACT=artifact,
                BINARY_NAME=binary, BUILD_ARCH="x86_64", GIT_RELEASE_TAG="v0.2026.10.14.13.00.oss_00",
            )
            bundled = subprocess.run(["bash", str(fixture / "script/linux/bundle_deb")], env=environment, capture_output=True, text=True)
            self.assertEqual(bundled.returncode, 0, bundled.stdout + bundled.stderr)
            control = output / "DEBIAN"
            if native_dpkg is not None:
                packages = list(output.glob("*.deb"))
                self.assertEqual(len(packages), 1)
                subprocess.run([native_dpkg, "--info", str(packages[0])], check=True, capture_output=True, text=True)
                subprocess.run([native_dpkg, "--control", str(packages[0]), str(control)], check=True)
            for name in ("control", "postinst", "postrm"):
                self.assertIsNone(re.search(r"@@[A-Z_]+@@", (control / name).read_text()), f"{artifact}/{channel}/{name}")
            self.assertIn(f"Package: {package}\n", (control / "control").read_text())
            for name in ("postinst", "postrm"):
                self.assertEqual((control / name).stat().st_mode & 0o777, 0o755)
                subprocess.run(["bash", "-n", str(control / name)], check=True)
            for action in actions:
                for state in states:
                    with self.subTest(artifact=artifact, channel=channel, action=action, state=state):
                        sandbox = variant / f"{action}-{state}"
                        (sandbox / "usr/bin").mkdir(parents=True)
                        stub = sandbox / "tools"
                        stub.mkdir()
                        # 实际脚本只改路径命名空间，逻辑和 shell 命令保持不变。
                        for name in ("postinst", "postrm"):
                            body = (control / name).read_text()
                            body = body.replace("/usr/bin/", f"{sandbox}/usr/bin/").replace("/opt/", f"{sandbox}/opt/")
                            (sandbox / name).write_text(body)
                        executable(stub / "update-desktop-database", "#!/usr/bin/env bash\nexit 0\n")
                        apt_dirs = {"APT_TRUSTED_KEYRING_DIR": sandbox / "apt/trusted", "APT_SOURCE_LIST_DIR": sandbox / "apt/sources"}
                        apt_body = "#!/usr/bin/env bash\ncase \"$2\" in\n"
                        for variable, directory in apt_dirs.items():
                            directory.mkdir(parents=True)
                            apt_body += f"{variable}) printf '%s\\n' {shlex.quote(variable + '=' + shlex.quote(str(directory) + '/'))} ;;\n"
                        apt_body += "*) exit 1 ;;\nesac\n"
                        executable(stub / "apt-config", apt_body)
                        execution_env = dict(os.environ, PATH=f"{stub}{os.pathsep}{os.environ['PATH']}")
                        subprocess.run(["bash", str(sandbox / "postinst"), "configure"], env=execution_env, check=True)
                        link = sandbox / "usr/bin" / (package if artifact == "app" else binary)
                        target = sandbox / opt.lstrip("/") / binary
                        self.assertTrue(link.is_symlink())
                        self.assertEqual(os.readlink(link), str(target))
                        if state == "owned-live":
                            target.parent.mkdir(parents=True)
                            target.write_text("package executable")
                        elif state not in ("owned-dangling",):
                            link.unlink()
                            if state == "foreign-link":
                                link.symlink_to(sandbox / "foreign-executable")
                            elif state == "regular-file":
                                link.write_text("administrator file")
                            elif state == "directory":
                                link.mkdir()
                                (link / "sentinel").write_text("administrator directory")
                        expected_link = os.readlink(link) if link.is_symlink() else None
                        subprocess.run(["bash", str(sandbox / "postrm"), action, "0.1"], env=execution_env, check=True)
                        removed = state == "missing" or (action in ("remove", "purge") and state.startswith("owned-"))
                        self.assertEqual(os.path.lexists(link), not removed)
                        if not removed and expected_link is not None:
                            self.assertEqual(os.readlink(link), expected_link)
                        if state == "regular-file":
                            self.assertEqual(link.read_text(), "administrator file")
                        if state == "directory":
                            self.assertEqual((link / "sentinel").read_text(), "administrator directory")
                        if state == "owned-live":
                            self.assertEqual(target.read_text(), "package executable")
                        # remove/purge 重入必须成功，不能把已消失的链接视为错误。
                        if action in ("remove", "purge"):
                            subprocess.run(["bash", str(sandbox / "postrm"), action], env=execution_env, check=True)
                            self.assertEqual(os.path.lexists(link), not removed)
                        # 释放本场景已核对内容的链接夹具，便于外层按身份检查临时目录。
                        if expected_link is not None and not removed:
                            link.unlink()
                        checks += 1
        mode = "真实 dpkg-deb 构建并提取" if native_dpkg else "macOS 渲染检查，未构建 DEB"
        print(f"通过：{len(variants)} 种包命名，{checks} 个维护脚本场景；{mode}；系统目录未写入。")


if __name__ == "__main__":
    unittest.main()
