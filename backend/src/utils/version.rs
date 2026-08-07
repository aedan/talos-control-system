use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct VersionInfo {
    pub version: String,
    pub commit: String,
    pub build_time: String,
}

impl Default for VersionInfo {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            commit: option_env!("GIT_HASH").unwrap_or("unknown").to_string(),
            build_time: option_env!("BUILD_TIME").unwrap_or("unknown").to_string(),
        }
    }
}

lazy_static::lazy_static! {
    pub static ref VERSION_INFO: VersionInfo = VersionInfo::default();
}

pub fn get_version() -> &'static str {
    &VERSION_INFO.version
}

pub fn get_commit() -> &'static str {
    &VERSION_INFO.commit
}

pub fn get_build_time() -> &'static str {
    &VERSION_INFO.build_time
}

pub fn format_version() -> String {
    format!(
        "{} (commit: {}, built: {})",
        VERSION_INFO.version, VERSION_INFO.commit, VERSION_INFO.build_time
    )
}
