use anyhow::{Result, anyhow};

const MANAGED_FILE_PREFIX: &str = "infinishell-v";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InstalledVersion {
    pub(crate) file_name: String,
    pub(crate) size_bytes: u64,
    pub(crate) is_current: bool,
    pub(crate) is_running: bool,
}

impl InstalledVersion {
    pub(crate) fn can_remove(&self) -> bool {
        !self.is_current && !self.is_running
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct CleanupResult {
    pub(crate) removed: Vec<String>,
    pub(crate) skipped_current: Vec<String>,
    pub(crate) skipped_running: Vec<String>,
    pub(crate) missing: Vec<String>,
    pub(crate) failed: Vec<String>,
}

/// 扫描脚本只读取当前 SSH 账号的固定安装目录，并以完整命令路径判断版本是否仍在运行。
pub(crate) fn scan_script() -> &'static str {
    r#"install_dir="$HOME/.infinishell/remote-server"
is_infinishell_running() {
  target_path=$1
  daemon_prefix="$target_path remote-server-daemon"
  proxy_prefix="$target_path remote-server-proxy"
  process_list=$(ps -axo command= 2>/dev/null) || return 2
  printf '%s\n' "$process_list" | (
    while IFS= read -r command_line; do
      case "$command_line" in
        "$daemon_prefix"|"$daemon_prefix "*|"$proxy_prefix"|"$proxy_prefix "*) exit 0 ;;
      esac
    done
    exit 1
  )
}
for path in "$install_dir"/infinishell-v*; do
  [ -f "$path" ] || continue
  file_name=${path##*/}
  size_bytes=$(wc -c < "$path" | tr -d '[:space:]')
  is_infinishell_running "$path"
  running_status=$?
  if [ "$running_status" -eq 0 ]; then
    is_running=1
  elif [ "$running_status" -eq 1 ]; then
    is_running=0
  else
    exit 2
  fi
  printf 'I\t%s\t%s\t%s\n' "$file_name" "$size_bytes" "$is_running"
done"#
}

/// 只接收扫描输出中符合固定前缀与版本字符白名单的文件名。
pub(crate) fn parse_scan_output(stdout: &str, current_file_name: &str) -> Vec<InstalledVersion> {
    let mut versions = stdout
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            if fields.next()? != "I" {
                return None;
            }
            let file_name = fields.next()?;
            let size_bytes = fields.next()?.parse().ok()?;
            let is_running = match fields.next()? {
                "0" => false,
                "1" => true,
                _ => return None,
            };
            if fields.next().is_some() || !is_managed_file_name(file_name) {
                return None;
            }
            Some(InstalledVersion {
                file_name: file_name.to_string(),
                size_bytes,
                is_current: file_name == current_file_name,
                is_running,
            })
        })
        .collect::<Vec<_>>();
    versions.sort_by(|left, right| right.file_name.cmp(&left.file_name));
    versions.dedup_by(|left, right| left.file_name == right.file_name);
    versions
}

/// 生成确认后执行的清理脚本。
///
/// `rm` 的目录和前缀固定，调用方只能补充再次通过白名单校验的有限文件名；脚本还会在
/// 每个 `rm` 之前重新保护当前版本和正在运行的版本。
pub(crate) fn cleanup_script(file_names: &[String], current_file_name: &str) -> Result<String> {
    if current_file_name != "infinishell" && !is_managed_file_name(current_file_name) {
        return Err(anyhow!("invalid current InfiniShell version file name"));
    }
    for file_name in file_names {
        if !is_managed_file_name(file_name) {
            return Err(anyhow!(
                "invalid InfiniShell version file name: {file_name}"
            ));
        }
    }

    let file_names = file_names
        .iter()
        .map(|file_name| format!("'{file_name}'"))
        .collect::<Vec<_>>()
        .join(" ");
    Ok(format!(
        r#"install_dir="$HOME/.infinishell/remote-server"
current_file_name='{current_file_name}'
is_infinishell_running() {{
  target_path=$1
  daemon_prefix="$target_path remote-server-daemon"
  proxy_prefix="$target_path remote-server-proxy"
  process_list=$(ps -axo command= 2>/dev/null) || return 2
  printf '%s\n' "$process_list" | (
    while IFS= read -r command_line; do
      case "$command_line" in
        "$daemon_prefix"|"$daemon_prefix "*|"$proxy_prefix"|"$proxy_prefix "*) exit 0 ;;
      esac
    done
    exit 1
  )
}}
for file_name in {file_names}; do
  path="$install_dir/$file_name"
  if [ "$file_name" = "$current_file_name" ]; then
    printf 'C\t%s\n' "$file_name"
  elif [ ! -f "$path" ]; then
    printf 'M\t%s\n' "$file_name"
  else
    is_infinishell_running "$path"
    running_status=$?
    if [ "$running_status" -eq 0 ]; then
      printf 'U\t%s\n' "$file_name"
    elif [ "$running_status" -ne 1 ]; then
      printf 'F\t%s\n' "$file_name"
    elif rm -f -- "$path"; then
      printf 'R\t%s\n' "$file_name"
    else
      printf 'F\t%s\n' "$file_name"
    fi
  fi
done"#
    ))
}

pub(crate) fn parse_cleanup_output(stdout: &str) -> CleanupResult {
    let mut result = CleanupResult::default();
    for line in stdout.lines() {
        let Some((status, file_name)) = line.split_once('\t') else {
            continue;
        };
        if !is_managed_file_name(file_name) {
            continue;
        }
        match status {
            "R" => result.removed.push(file_name.to_string()),
            "C" => result.skipped_current.push(file_name.to_string()),
            "U" => result.skipped_running.push(file_name.to_string()),
            "M" => result.missing.push(file_name.to_string()),
            "F" => result.failed.push(file_name.to_string()),
            _ => {}
        }
    }
    result
}

fn is_managed_file_name(file_name: &str) -> bool {
    let Some(version) = file_name.strip_prefix(MANAGED_FILE_PREFIX) else {
        return false;
    };
    !version.is_empty()
        && version.as_bytes()[0].is_ascii_digit()
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(test)]
#[path = "remote_extensions_tests.rs"]
mod tests;
