use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MischiefLevel {
    Calm,
    #[default]
    Playful,
    Devil,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub name: String,
    pub level: MischiefLevel,
    pub allow_cursor_chase: bool,
    pub allow_hide_icons: bool,
    pub hide_icons_consent_version: u8,
    pub quiet: bool,
    pub roaming: bool,
    pub low_power: bool,
    pub scale: f32,
    pub neglect_seconds: u64,
    pub icon_hide_seconds: u64,
    pub affinity: u32,
    pub feeds: u32,
    pub interactions: u32,
    pub skin: Option<PathBuf>,
    pub clipboard_enabled: bool,
    pub clipboard_days: u32,
    pub clipboard_limit: u32,
    pub content_quota_gib: u32,
    pub download_dropped_resources: bool,
    pub explorer_enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            name: "麻薯".into(),
            level: MischiefLevel::Playful,
            allow_cursor_chase: false,
            allow_hide_icons: false,
            hide_icons_consent_version: 0,
            quiet: false,
            roaming: true,
            low_power: false,
            scale: 1.0,
            neglect_seconds: 15 * 60,
            icon_hide_seconds: 15,
            affinity: 10,
            feeds: 0,
            interactions: 0,
            skin: None,
            clipboard_enabled: true,
            clipboard_days: 30,
            clipboard_limit: 500,
            content_quota_gib: 5,
            download_dropped_resources: true,
            explorer_enabled: false,
        }
    }
}

impl Config {
    pub fn sanitize(&mut self) {
        self.name = self.name.chars().take(16).collect();
        if self.name.trim().is_empty() {
            self.name = "麻薯".into();
        }
        self.scale = if self.scale.is_finite() {
            self.scale.clamp(0.65, 1.5)
        } else {
            1.0
        };
        self.neglect_seconds = self.neglect_seconds.clamp(30, 7200);
        self.icon_hide_seconds = self.icon_hide_seconds.clamp(5, 30);
        self.affinity = self.affinity.min(100);
        self.clipboard_days = self.clipboard_days.clamp(1, 3650);
        self.clipboard_limit = self.clipboard_limit.clamp(10, 100000);
        self.content_quota_gib = self.content_quota_gib.clamp(1, 1024);
        if self.hide_icons_consent_version != 1 {
            self.allow_hide_icons = false;
        }
    }
}

pub fn data_dir() -> io::Result<PathBuf> {
    ProjectDirs::from("dev", "JarhsonNing", "Mochi")
        .map(|dirs| dirs.data_local_dir().to_path_buf())
        .ok_or_else(|| io::Error::other("Cannot locate application data directory"))
}

pub fn load(path: &Path) -> io::Result<Config> {
    match fs::read(path) {
        Ok(bytes) => {
            let mut config: Config = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            config.sanitize();
            Ok(config)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Config::default()),
        Err(error) => Err(error),
    }
}

pub fn atomic_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    {
        use std::io::Write;
        let mut file = fs::File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let source: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dangerous_features_require_versioned_consent() {
        let mut config = Config {
            allow_hide_icons: true,
            scale: f32::NAN,
            ..Config::default()
        };
        config.sanitize();
        assert!(!config.allow_hide_icons);
        assert_eq!(config.scale, 1.0);
        config.hide_icons_consent_version = 1;
        config.allow_hide_icons = true;
        config.icon_hide_seconds = 10000;
        config.sanitize();
        assert!(config.allow_hide_icons);
        assert_eq!(config.icon_hide_seconds, 30);
    }
    #[test]
    fn old_settings_get_safe_defaults() {
        let config: Config = serde_json::from_str(r#"{"name":"团子"}"#).unwrap();
        assert_eq!(config.name, "团子");
        assert!(!config.allow_hide_icons);
        assert!(!config.allow_cursor_chase);
        assert!(config.skin.is_none());
    }
}
