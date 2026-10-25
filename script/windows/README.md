# Inno Setup installer script

## What is `windows-installer.iss`?

On Windows, programs are conventionally installed using an installer, also known as an installation wizard.
The installer is a single executable that takes care of:
* Creating a directory to store the program's files
* Downloading assets
* Initializing registry entries
* Creating a desktop icon
* ... and more, depending on the application's needs.


`windows-installer.iss` is an **Inno Setup script**:
a configuration file for building the InfiniShell installer.
The Inno Setup Compiler takes a script file and generates an installer executable.
This is roughly equivalent to the bundling process on MacOS.


## How to edit the installer

See the Inno Setup documentation: [Inno Setup Help](https://jrsoftware.org/ishelp/).
This script can be edited manually using any code editor.
However, it requires the Inno Setup compiler to be turned into a `.exe` file.


## Building the InfiniShell installer

Use `bundle.ps1` from the repository root. Install the Windows build dependencies from the development guide and add Inno Setup 6.7 or later `ISCC.exe` to PATH; the installer uses the `RedirectionGuard` option introduced in 6.7.

Replace the placeholder with the actual candidate tag:

```powershell
.\script\windows\bundle.ps1 -CHANNEL oss -ARTIFACT app -ARCH x64 -RELEASE_TAG '<candidate-tag>'
```

The script selects the Cargo profile, builds the application and bundled tools, expands the PowerShell bootstrap, prepares resources, and supplies Inno definitions including `Arch`, `OutputName`, `MyAppExeName`, and `TargetProfileDir`. The OSS x64 GUI installer is written to `script\windows\Output\InfiniShellSetup.exe`. Without signing configured, the result is unsigned; a successful build does not establish signing or installation lifecycle acceptance.

Use `-SKIP_BUILD_BINARY` only when all binaries and resources are ready for the same candidate, architecture, and profile. A standalone `cargo build` does not prepare every required installer input.

When debugging the `.iss` script, inspect the current `$ISCC_ARGS` in `bundle.ps1` and keep application identity, architecture, output name, and resource paths consistent. Do not invoke bare `iscc windows-installer.iss` or rely on old executable/profile defaults. Compiling through the Inno GUI requires the same complete definitions and resources.


## Using icons

Windows has its own icon file format that bundles together multiple icon sizes.
App icons are located in `app/channels/<channel_name>/icon/padded` (with approximately 10% safe area, shared by the macOS Dock and Linux/Windows).
The `.ico` files are generated using imagemagick:

```shell
convert 16x16.png 32x32.png 48x48.png 64x64.png 256x256.png icon.ico
```

Note that sizes above 256x256 are not supported.
See the [Inno Setup docs](https://jrsoftware.org/ishelp/index.php?topic=setup_setupiconfile).
