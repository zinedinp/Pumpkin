/// A local endpoint name, unique to the calling process.
#[must_use]
pub fn unique_endpoint() -> String {
    #[cfg(unix)]
    {
        std::env::temp_dir()
            .join(format!("pumpkin-gui-{}.sock", std::process::id()))
            .to_string_lossy()
            .into_owned()
    }
    #[cfg(windows)]
    {
        format!(r"\\.\pipe\pumpkin-gui-{}", std::process::id())
    }
}
