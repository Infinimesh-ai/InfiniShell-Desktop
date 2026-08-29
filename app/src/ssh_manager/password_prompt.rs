use lazy_static::lazy_static;
use regex::bytes::Regex;

const PASSWORD_PROMPT_PATTERN: &str = r"(?im)(password|passphrase)[^\n]*:\s*$";

lazy_static! {
    static ref PASSWORD_PROMPT_REGEX: Regex =
        Regex::new(PASSWORD_PROMPT_PATTERN).expect("password prompt regex must compile");
}

pub fn bytes_look_like_password_prompt(bytes: &[u8]) -> bool {
    PASSWORD_PROMPT_REGEX.is_match(bytes)
}

/// 密码提示运行在终端输入路径中，提交键必须使用 VT 的 Enter 字节 CR。
pub fn append_password_submit_byte(bytes: &mut Vec<u8>) {
    bytes.push(b'\r');
}

#[cfg(test)]
#[path = "password_prompt_tests.rs"]
mod tests;
