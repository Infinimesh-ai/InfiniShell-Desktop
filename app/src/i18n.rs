//! Fluent-based localization layer for InfiniShell Desktop.
//!
//! 加载链:
//!   1. `init()` 在启动时调用一次(idempotent),用 `RustEmbed` 加载 `app/i18n/{locale}/*.ftl`
//!   2. `LANGUAGE_LOADER` 是全局 `OnceLock<FluentLanguageLoader>`,在 fallback chain
//!      上 select 当前 locale (默认按系统 locale,可被 settings 覆盖)
//!   3. 业务侧调 `t!("key")` / `t!("key", name = ..)` 取字符串,key 缺失自动回退英文
//!
//! 缺失 key 时:
//!   - 当前 locale 没有 → fluent 内部 fallback 到 fallback_language (en)
//!   - 连英文都没有 → 返回 key 本身字符串(并 log::warn,便于 CI 抓未翻译条目)

use std::borrow::Cow;
use std::sync::OnceLock;

#[cfg(not(target_os = "macos"))]
use i18n_embed::DesktopLanguageRequester;
use i18n_embed::fluent::{FluentLanguageLoader, fluent_language_loader};
use i18n_embed::{I18nAssets, LanguageLoader};
use rust_embed::RustEmbed;
use unic_langid::LanguageIdentifier;

/// 把 `app/i18n` 目录嵌进二进制。每次构建会重新嵌入(debug-embed feature 已在 workspace 开)。
#[derive(RustEmbed)]
#[folder = "i18n/"]
struct Localizations;

/// 将桌面端与 TUI 的 Fluent domain 合并到共享的 `warp` 运行时加载器中。
///
/// 两个 crate 仍各自用 `warp.ftl` / `warp_tui.ftl` 做编译期消息键校验；运行时查询统一
/// 经过本模块，因此在加载 `warp.ftl` 时同时返回同语言的 `warp_tui.ftl`。Fluent loader
/// 原生支持同一路径对应多个资源 bundle，无需复制消息或维护第二套全局 locale 状态。
struct MergedLocalizations;

impl I18nAssets for MergedLocalizations {
    fn get_files(&self, file_path: &str) -> Vec<Cow<'_, [u8]>> {
        let mut files = Localizations::get(file_path)
            .map(|file| file.data)
            .into_iter()
            .collect::<Vec<_>>();

        if let Some((locale, domain)) = file_path.rsplit_once('/')
            && domain == "warp.ftl"
        {
            let tui_path = format!("{locale}/warp_tui.ftl");
            if let Some(file) = Localizations::get(&tui_path) {
                files.push(file.data);
            }
        }

        files
    }

    fn filenames_iter(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(Localizations::iter().map(|path| path.to_string()))
    }
}

static LANGUAGE_LOADER: OnceLock<FluentLanguageLoader> = OnceLock::new();

/// 在 app 启动早期调用一次。
///
/// `override_locale`:用户在 Settings 显式选择的语言(如 "zh-CN"),为 `None` 时按系统 locale。
/// 永远不会 panic — 加载失败会 fallback 到内置英文 bundle。
pub fn init(override_locale: Option<&str>) {
    if LANGUAGE_LOADER.get().is_some() {
        return;
    }

    let loader = fluent_language_loader!();

    // 总是先加载 fallback (en) bundle —— 任何 locale 缺 key 都会落到它。
    if let Err(e) = loader.load_fallback_language(&MergedLocalizations) {
        log::error!("[i18n] failed to load fallback (en) bundle: {e}");
    }

    // 决定运行时 locale 列表(按优先级)。
    let requested: Vec<LanguageIdentifier> = match override_locale {
        Some(s) => match s.parse::<LanguageIdentifier>() {
            Ok(li) => vec![li],
            Err(e) => {
                log::warn!("[i18n] invalid override_locale {s:?}: {e} — falling back to system");
                system_requested_languages()
            }
        },
        None => system_requested_languages(),
    };

    if let Err(e) = i18n_embed::select(&loader, &MergedLocalizations, &requested) {
        log::warn!("[i18n] select() failed: {e} — running with fallback only");
    }
    // Fluent 默认在每个插值变量两侧加入 FSI/PDI 双向文本隔离符。桌面/TUI 渲染器都按
    // 可见单元格测量字符串，这些隐藏字符会破坏截断、快照和复制文本；本产品当前只提供
    // LTR 界面语言，因此统一关闭隔离符。
    loader.set_use_isolating(false);

    log::info!(
        "[i18n] initialized; current_languages={:?}, fallback={}",
        loader.current_languages(),
        loader.fallback_language()
    );

    propagate_ui_locale(&loader);

    let _ = LANGUAGE_LOADER.set(loader);
}

/// Forward the resolved UI locale to `warpui::set_ui_locale` so DirectWrite / CoreText
/// glyph fallback biases CJK Han characters toward the user's UI language.
fn propagate_ui_locale(loader: &FluentLanguageLoader) {
    let langs = loader.current_languages();
    if let Some(li) = langs.first() {
        warpui::set_ui_locale(li.to_string());
    }
}

fn system_requested_languages() -> Vec<LanguageIdentifier> {
    #[cfg(target_os = "macos")]
    {
        macos_requested_languages()
    }

    #[cfg(not(target_os = "macos"))]
    {
        DesktopLanguageRequester::requested_languages()
    }
}

#[cfg(target_os = "macos")]
fn macos_requested_languages() -> Vec<LanguageIdentifier> {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    use warpui::platform::mac::utils::nsstring_as_str;

    unsafe {
        let locale_class = class!(NSLocale);
        let preferred_languages: *const Object = msg_send![locale_class, preferredLanguages];
        let count: usize = msg_send![preferred_languages, count];

        let mut requested = Vec::with_capacity(count);
        for index in 0..count {
            let language: *const Object = msg_send![preferred_languages, objectAtIndex: index];
            match nsstring_as_str(language) {
                Ok(language) => {
                    if let Some(language) = parse_language_identifier(language) {
                        requested.push(language);
                    }
                }
                Err(err) => {
                    log::warn!(
                        "[i18n] failed to read macOS preferred language at index {index}: {err}"
                    );
                }
            }
        }

        languages_or_fallback(requested)
    }
}

fn parse_language_identifier(language: &str) -> Option<LanguageIdentifier> {
    match language.parse::<LanguageIdentifier>() {
        Ok(language) => Some(language),
        Err(err) => {
            log::warn!("[i18n] invalid language identifier {language:?}: {err}");
            None
        }
    }
}

fn languages_or_fallback(languages: Vec<LanguageIdentifier>) -> Vec<LanguageIdentifier> {
    if languages.is_empty() {
        vec![fallback_language()]
    } else {
        languages
    }
}

fn fallback_language() -> LanguageIdentifier {
    "en".parse().expect("en is a valid language identifier")
}

/// 取全局 loader。`init()` 没调过时返回 `None`(早期/测试代码可用 [`t_or`] 兜底)。
pub fn loader() -> Option<&'static FluentLanguageLoader> {
    LANGUAGE_LOADER.get()
}

/// 切换运行时 locale(在 `init()` 之后任意时刻可调)。
///
/// 实现细节:`FluentLanguageLoader::load_languages` 内部用 RwLock 保护语言数据,
/// 故 `&loader` 即可热替换,无需重建。但 **已渲染的 UI 文本不会自动刷新** ——
/// `t!()` 返回的是当时拷贝的 `String`,要看到新语言需要 view 重建/重绘。
/// 调用方可决定是否触发全局重绘,或提示用户重启。
///
/// `locale` 传 BCP-47(如 `"en"`、`"zh-CN"`)。失败时保留原 locale,记录 warn,不 panic。
pub fn set_locale(locale: &str) {
    let Some(loader) = LANGUAGE_LOADER.get() else {
        log::warn!("[i18n] set_locale({locale:?}) called before init() — ignoring");
        return;
    };
    let lang_id: LanguageIdentifier = match locale.parse() {
        Ok(li) => li,
        Err(e) => {
            log::warn!("[i18n] set_locale({locale:?}): invalid BCP-47: {e}");
            return;
        }
    };
    if let Err(e) = loader.load_languages(&MergedLocalizations, &[lang_id]) {
        log::warn!("[i18n] set_locale({locale:?}) failed: {e}");
        return;
    }
    loader.set_use_isolating(false);
    log::info!(
        "[i18n] locale switched to {locale:?}; current_languages={:?}",
        loader.current_languages()
    );
    propagate_ui_locale(loader);
}

/// 重置回系统语言(撤销显式 override)。
pub fn reset_to_system_locale() {
    let Some(loader) = LANGUAGE_LOADER.get() else {
        return;
    };
    let requested = system_requested_languages();
    if let Err(e) = i18n_embed::select(loader, &MergedLocalizations, &requested) {
        log::warn!("[i18n] reset_to_system_locale failed: {e}");
    }
    loader.set_use_isolating(false);
    propagate_ui_locale(loader);
}

/// 获取已激活的语言列表(主选 + fallback)。仅供调试 / settings UI 显示用。
pub fn current_languages() -> Vec<LanguageIdentifier> {
    LANGUAGE_LOADER
        .get()
        .map(|l| l.current_languages())
        .unwrap_or_default()
}

/// 业务层主入口:`t!("key")` 或 `t!("key", name = value, count = 3)`。
///
/// - 包了 `i18n_embed_fl::fl!`,但额外做了"loader 未初始化"的兜底:
///   返回 key 本身,避免 panic
/// - 返回 `String`(直接喂给 GPUI Text/label_text,无需额外转换)
#[macro_export]
macro_rules! t {
    ($message_id:literal $(,)?) => {{
        match $crate::i18n::loader() {
            Some(loader) => ::i18n_embed_fl::fl!(loader, $message_id),
            None => {
                ::log::warn!(
                    "[i18n] t!({:?}) called before init(); returning key as-is",
                    $message_id
                );
                String::from($message_id)
            }
        }
    }};
    ($message_id:literal, $($args:tt)*) => {{
        match $crate::i18n::loader() {
            Some(loader) => ::i18n_embed_fl::fl!(loader, $message_id, $($args)*),
            None => {
                ::log::warn!(
                    "[i18n] t!({:?}, ...) called before init(); returning key as-is",
                    $message_id
                );
                String::from($message_id)
            }
        }
    }};
}

/// 与 `t!` 等价,但返回 `&'static str`(每次调用都会通过 `Box::leak` 永久占用一段堆)。
///
/// 使用约束:**仅在 `LazyLock`/一次性初始化里调用**(如 `StaticCommand` 这种 struct
/// 字段是 `&'static str`、又必须从 fluent 取文本的场景)。**禁止在热路径或循环里使用**,
/// 否则会持续泄漏内存。编译期仍享受 `fl!()` 的 key 校验。
#[macro_export]
macro_rules! t_static {
    ($message_id:literal $(,)?) => {{
        let s: String = $crate::t!($message_id);
        &*::std::boxed::Box::leak(s.into_boxed_str())
    }};
}

/// 同 `t!` 但带显式默认值,适合极早期/loader 未就绪场景。
pub fn t_or(message_id: &str, fallback: &str) -> String {
    match LANGUAGE_LOADER.get() {
        Some(loader) if loader.has(message_id) => loader.get(message_id),
        _ => fallback.to_string(),
    }
}

#[cfg(test)]
#[path = "i18n_tests.rs"]
mod tests;
