use command::blocking::Command;
use warp_ssh_manager::{SecretKind, SshSecretStoreError};

use super::*;

const LOCALE_ENV: &str = "INFINISHELL_SYNC_TEST_LOCALE";
const TEST_NAME: &str = "settings_view::cloud_sync_page::tests::localized_cloud_sync_errors_and_plan_tooltip_follow_locale";

#[test]
fn localized_cloud_sync_errors_and_plan_tooltip_follow_locale() {
    if let Ok(locale) = std::env::var(LOCALE_ENV) {
        // 子进程独占全局语言，普通 cargo test 并行运行时也不影响其它测试。
        crate::i18n::init(Some(&locale));
        // 测试二进制的 ctor 已初始化英文，需走实际语言切换入口。
        crate::i18n::set_locale(&locale);
        assert_eq!(crate::i18n::current_languages()[0].to_string(), locale);
        check_localized_errors(&locale);
        return;
    }

    for locale in ["en", "zh-CN"] {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(LOCALE_ENV, locale)
            .output()
            .unwrap();
        assert!(
            output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "{locale} 错误文案或布局检查失败：\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn check_localized_errors(locale: &str) {
    // 此测试不启动 App，单独为 reqwest 错误夹具设置 TLS 提供者。
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let cases = [
        (
            SyncEngineError::Crypto(CryptoError::Encrypt("diagnostic-marker".into())),
            "Could not encrypt",
            "无法加密",
        ),
        (
            SyncEngineError::Crypto(CryptoError::Decrypt("diagnostic-marker".into())),
            "Could not decrypt",
            "无法解密",
        ),
        (
            SyncEngineError::Provider(anyhow::anyhow!("diagnostic-marker")),
            "Could not access local",
            "无法访问本地",
        ),
        (
            SyncEngineError::Serialization("diagnostic-marker".into()),
            "sync data format",
            "同步数据格式",
        ),
        (
            SyncEngineError::VersionStore("diagnostic-marker".into()),
            "local sync version",
            "本地同步版本",
        ),
    ];
    let mut texts = Vec::new();
    for (error, english, chinese) in cases {
        let text = localize_sync_error(&error);
        assert!(text.contains(if locale == "en" { english } else { chinese }));
        assert!(text.contains("diagnostic-marker"));
        texts.push(text);
    }

    let request_error = reqwest::Client::new()
        .get("://invalid")
        .build()
        .unwrap_err();
    for (error, english, chinese) in [
        (
            GistClientError::Request(request_error),
            "Check your network",
            "检查网络",
        ),
        (
            GistClientError::NotFound,
            "Upload your configuration",
            "请先上传配置",
        ),
        (GistClientError::NoToken, "Add a token", "添加令牌"),
        (
            GistClientError::Api {
                status: 403,
                body: "diagnostic-marker".into(),
            },
            "HTTP 403",
            "HTTP 403",
        ),
        (
            GistClientError::MissingLogin,
            "token could not be verified",
            "无法验证令牌",
        ),
    ] {
        let expected = if locale == "en" { english } else { chinese };
        // 同一类型错误分别覆盖令牌验证直达路径与同步引擎包装路径。
        let text = localize_gist_error(&error);
        assert!(text.contains(expected), "{locale}：{text}");
        assert_eq!(localize_sync_error(&SyncEngineError::Gist(error)), text);
        texts.push(text);
    }

    let keyring_error = || SshSecretStoreError::Keyring("diagnostic-marker".into());
    for (error, english, chinese, rollback_count) in [
        (
            SshSyncProviderError::ReadSecret {
                node_id: "node-marker".into(),
                kind: SecretKind::Password,
                source: keyring_error(),
            },
            "retry the upload",
            "重试上传",
            None,
        ),
        (
            SshSyncProviderError::ReadPreviousSecret {
                node_id: "node-marker".into(),
                kind: SecretKind::Password,
                rolled_back: 2,
                source: keyring_error(),
            },
            "retry the download",
            "重试下载",
            Some(2),
        ),
        (
            SshSyncProviderError::WriteSecret {
                node_id: "node-marker".into(),
                kind: SecretKind::Password,
                source: keyring_error(),
            },
            "credential store permissions",
            "凭据存储权限",
            None,
        ),
        (
            SshSyncProviderError::WriteDatabase {
                rolled_back: 3,
                source: anyhow::anyhow!("diagnostic-marker"),
            },
            "Credential changes rolled back",
            "凭据变更",
            Some(3),
        ),
    ] {
        let action = CloudSyncPageAction::SyncComplete {
            platform: SyncPlatform::GitHub,
            direction: SyncDirection::Download,
            result: Err(Arc::new(SyncEngineError::Provider(error.into()))),
        };
        let CloudSyncPageAction::SyncComplete {
            result: Err(error), ..
        } = action.clone()
        else {
            panic!("动作克隆必须保留同步错误类型");
        };
        let text = localize_sync_error(&error);
        assert!(
            text.contains(if locale == "en" { english } else { chinese }),
            "{locale}：{text}"
        );
        assert!(text.contains("diagnostic-marker"));
        if let Some(count) = rollback_count {
            assert!(
                text.contains(&count.to_string()),
                "{locale} 未保留回滚数：{text}"
            );
        }
        if text.contains("node-marker") {
            assert!(text.contains("Password"));
        }
        if text.contains("secret-service") {
            assert!(text.contains("Credential Manager"));
            assert!(text.contains(if locale == "en" {
                "clear that field"
            } else {
                "清除相应字段"
            }));
        }
        texts.push(text);
    }

    let tooltip = crate::t!("ai-plan-saved-locally-tooltip");
    assert_eq!(
        tooltip,
        if locale == "en" {
            "The plan was automatically saved locally."
        } else {
            "计划已自动保存到本地。"
        }
    );
    check_text_layout(&texts, &tooltip);
}

#[cfg(target_os = "macos")]
fn check_text_layout(texts: &[String], tooltip: &str) {
    use warpui::AssetProvider as _;
    use warpui::elements::DEFAULT_UI_LINE_HEIGHT_RATIO;
    use warpui::fonts::Properties;
    use warpui::platform::mac::{AutoreleasePoolGuard, FontDB};
    use warpui::platform::{FontDB as _, LineStyle};
    use warpui::text_layout::{DEFAULT_TOP_BOTTOM_RATIO, StyleAndFont, TextStyle};

    let _pool = AutoreleasePoolGuard::new();
    let mut font_db = FontDB::new();
    let font_family = font_db
        .load_from_bytes(
            "Hack",
            vec![
                crate::ASSETS
                    .get("bundled/fonts/hack/Hack-Regular.ttf")
                    .unwrap()
                    .to_vec(),
            ],
        )
        .unwrap();
    let line_style = LineStyle {
        font_size: 14.,
        line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
        baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
        fixed_width_tab_size: None,
    };

    for text in texts {
        // 使用实际状态行外层文案，验证长恢复指导也完整换行。
        let text = crate::t!("settings-cloud-sync-failed", error = text.as_str());
        let char_count = text.chars().count();
        let runs = [(
            0..char_count,
            StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
        )];
        for width in [280., 420.] {
            let frame = font_db.text_layout_system().layout_text(
                &text,
                line_style,
                &runs,
                width,
                f32::MAX,
                Default::default(),
                None,
            );
            assert_eq!(frame.lines().last().unwrap().end_index(), char_count);
            assert!(frame.height().is_finite() && frame.height() < 1000.);
            for line in frame.lines() {
                assert!(line.width - line.trailing_whitespace_width <= width);
                assert!(line.chars_with_missing_glyphs.is_empty());
            }
        }
    }

    // Tooltip 使用不换行布局，确认两种语言都能在常见桌面视口内完整容纳。
    let char_count = tooltip.chars().count();
    let runs = [(
        0..char_count,
        StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
    )];
    let frame = font_db.text_layout_system().layout_text(
        tooltip,
        line_style,
        &runs,
        f32::MAX,
        f32::MAX,
        Default::default(),
        None,
    );
    assert_eq!(frame.lines().len(), 1);
    assert_eq!(frame.lines()[0].end_index(), char_count);
    assert!(frame.lines()[0].width + 14. <= 420.);
    assert!(frame.lines()[0].chars_with_missing_glyphs.is_empty());
}

#[cfg(not(target_os = "macos"))]
fn check_text_layout(texts: &[String], tooltip: &str) {
    // 原生排版检查由 macOS 执行，其它平台仍检查完整消息和变量。
    assert!(texts.iter().all(|text| !text.is_empty()));
    assert!(!tooltip.is_empty());
}
