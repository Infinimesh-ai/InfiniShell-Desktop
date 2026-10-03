//! 独立部署的低依赖 Windows 窗口站引导入口；不链接应用或 UI。

fn main() -> std::io::Result<()> {
    #[cfg(windows)]
    return command::windows::run_station_bootstrap();
    #[cfg(not(windows))]
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "窗口站引导只支持 Windows",
    ))
}
