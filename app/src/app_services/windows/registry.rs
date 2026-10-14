use std::{ffi::OsString, path::Path};

use anyhow::{ensure, Context, Result};
use warp_core::channel::ChannelState;
use windows_registry::{Key, CURRENT_USER, HSTRING};

const OWNER_PATH: &str = "InfiniShellOwnerPath";

pub(super) fn register_uri_handler() -> Result<()> {
    // 独立开发 profile 不覆盖当前用户的系统 URI 关联。
    if ChannelState::data_profile().is_some() {
        return Ok(());
    }

    let executable =
        std::env::current_exe().context("Could not get current executable for URI registration")?;
    let app_id = ChannelState::app_id();
    register_uri_handler_at(
        CURRENT_USER,
        "Software\\Classes",
        ChannelState::url_scheme(),
        app_id.application_name(),
        &app_id.to_string(),
        &executable,
    )
}

// 根键和 Classes 路径由调用方提供，让原生回归仅访问隔离的 HKCU 子树。
fn register_uri_handler_at(
    root: &Key,
    classes_path: &str,
    uri_scheme: &str,
    application_name: &str,
    app_id: &str,
    executable: &Path,
) -> Result<()> {
    // 缺失的 Classes、协议和命令键都要恢复，旧失效命令同样由本实例接管。
    let classes_key = root
        .create(classes_path)
        .context("Could not create or open URI classes key")?;
    let parent_key = classes_key
        .create(uri_scheme)
        .context("Could not create or open URI scheme key")?;
    parent_key
        .set_string("", application_name)
        .context("Could not set URI scheme display name")?;
    parent_key
        .set_string("URL Protocol", "")
        .context("Could not set URI protocol marker")?;
    let command_key = parent_key
        .create("shell\\open\\command")
        .context("Could not create or open URI command key")?;

    let mut command = OsString::from("\"");
    command.push(executable.as_os_str());
    command.push("\" \"%1\"");
    let expected_command = HSTRING::from(command.as_os_str());
    // windows-registry 0.2 的 set_hstring 不把终止 NUL 计入写入长度。
    // REG_SZ 的原始字节必须包含它，不能只依赖回读 API 自动补齐。
    command.push("\0");
    command_key
        .set_hstring("", &HSTRING::from(command.as_os_str()))
        .context("Could not set URI command")?;
    ensure!(
        command_key
            .get_hstring("")
            .context("Could not read back URI command")?
            == expected_command
            && parent_key
                .get_string("")
                .context("Could not read back URI scheme display name")?
                == application_name
            && parent_key
                .get_string("URL Protocol")
                .context("Could not read back URI protocol marker")?
                .is_empty(),
        "URI registration did not match after writing"
    );

    // 与安装器共用归属字段；命令成功后才登记所有者。
    let expected_owner = HSTRING::from(executable.as_os_str());
    let mut owner = executable.as_os_str().to_os_string();
    owner.push("\0");
    let owner = HSTRING::from(owner.as_os_str());
    parent_key
        .set_hstring(OWNER_PATH, &owner)
        .context("Could not set URI registration owner")?;
    ensure!(
        parent_key
            .get_hstring(OWNER_PATH)
            .context("Could not read back URI registration owner")?
            == expected_owner,
        "URI registration owner did not match after writing"
    );

    // 与 WarpUI 的显示名算法一致，并向安装器标明本实例的 AUMID 归属。
    let aumid_key = classes_key
        .create(format!("AppUserModelId\\{app_id}"))
        .context("Could not create or open AUMID key")?;
    let display_name = app_id.rsplit('.').next().unwrap_or(app_id);
    aumid_key
        .set_string("DisplayName", display_name)
        .context("Could not set AUMID display name")?;
    ensure!(
        aumid_key
            .get_string("DisplayName")
            .context("Could not read back AUMID display name")?
            == display_name,
        "AUMID display name did not match after writing"
    );
    aumid_key
        .set_hstring(OWNER_PATH, &owner)
        .context("Could not set AUMID registration owner")?;
    ensure!(
        aumid_key
            .get_hstring(OWNER_PATH)
            .context("Could not read back AUMID registration owner")?
            == expected_owner,
        "AUMID registration owner did not match after writing"
    );
    Ok(())
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
