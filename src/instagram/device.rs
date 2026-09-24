use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const API_DOMAIN: &str = "i.instagram.com";

pub const API_BASE: &str = "https://i.instagram.com/api/v1";

pub const APP_ID: &str = "567067343352427";

const APP_VERSION: &str = "428.0.0.47.67";
const VERSION_CODE: &str = "961145276";
const BLOKS_VERSIONING_ID: &str =
    "7189b949425f9bf80ea8bd880cf5a3080b292d9b1c4b38a18d112f7c4b71e7a8";

pub const DEFAULT_SESSION_PATH: &str = "session.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub uuid: String,
    pub phone_id: String,
    pub client_session_id: String,
    pub advertising_id: String,
    pub android_device_id: String,
    pub request_id: String,
    pub tray_session_id: String,
    pub user_agent: String,
    pub bloks_versioning_id: String,
    pub app_id: String,
    pub locale: String,
    pub country: String,
    pub timezone_offset: i32,
}

impl DeviceIdentity {
    pub fn fresh() -> Self {
        let uuid = Uuid::new_v4().to_string();
        let phone_id = Uuid::new_v4().to_string();
        let client_session_id = Uuid::new_v4().to_string();
        let advertising_id = Uuid::new_v4().to_string();
        let request_id = Uuid::new_v4().to_string();
        let tray_session_id = Uuid::new_v4().to_string();
        let android_device_id = format!("android-{}", &Uuid::new_v4().simple().to_string()[..16]);

        let user_agent = format!(
            "Instagram {APP_VERSION} Android (34/14; 480dpi; 1344x2992; \
             Google/google; Pixel 8 Pro; husky; husky; en_US; {VERSION_CODE})"
        );

        Self {
            uuid,
            phone_id,
            client_session_id,
            advertising_id,
            android_device_id,
            request_id,
            tray_session_id,
            user_agent,
            bloks_versioning_id: BLOKS_VERSIONING_ID.to_string(),
            app_id: APP_ID.to_string(),
            locale: "en_US".to_string(),
            country: "US".to_string(),
            timezone_offset: timezone_offset_secs(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionStore {
    #[serde(default)]
    pub sessionid: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub mid: String,
    #[serde(default)]
    pub ig_u_rur: String,
    #[serde(default)]
    pub ig_www_claim: String,
    #[serde(default)]
    pub authorization: String,
    #[serde(default)]
    pub device: Option<DeviceIdentity>,
    #[serde(default)]
    pub last_login_unix: Option<i64>,

    #[serde(default = "session_version")]
    pub version: u32,
}

fn session_version() -> u32 {
    1
}

impl SessionStore {
    pub fn load(path: &Path) -> Option<Self> {
        let bytes = fs::read(path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let tmp = path.with_extension(format!(
            "json.tmp.{}.{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        let data = serde_json::to_vec_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let write_result = (|| {
            let mut options = OpenOptions::new();
            options.create(true).truncate(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp)?;
            file.write_all(&data)?;
            file.sync_all()?;
            Ok::<_, std::io::Error>(())
        })();
        if let Err(error) = write_result {
            let _ = fs::remove_file(&tmp);
            return Err(error);
        }
        fs::rename(&tmp, path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub fn with_sessionid(mut self, sessionid: &str) -> Self {
        if !sessionid.is_empty() {
            self.sessionid = sessionid.to_string();
        }
        if self.device.is_none() {
            self.device = Some(DeviceIdentity::fresh());
        }
        self
    }

    pub fn ensure_device(&mut self) -> &DeviceIdentity {
        if self.device.is_none() {
            self.device = Some(DeviceIdentity::fresh());
        }
        self.device.as_ref().expect("device just set")
    }
}

pub fn resolve_session_path(raw: Option<&str>) -> PathBuf {
    let s = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_SESSION_PATH);
    PathBuf::from(s)
}

fn timezone_offset_secs() -> i32 {
    use chrono::Local;
    Local::now().offset().local_minus_utc()
}

pub fn jitter_u64(min: u64, max: u64) -> u64 {
    if max <= min {
        return min;
    }
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);

    let mix = n ^ (n >> 17) ^ (std::ptr::addr_of!(n) as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    min + (mix % (max - min + 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn fresh_has_android_prefix() {
        let d = DeviceIdentity::fresh();
        assert!(d.android_device_id.starts_with("android-"));
        assert!(d.user_agent.contains("Instagram"));
        assert!(d.user_agent.contains(APP_VERSION));
    }

    #[test]
    fn session_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "ig-pruner-sess-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");
        let mut store = SessionStore::default().with_sessionid("12345:abc:def");
        store.user_id = "12345".into();
        store.username = "tester".into();
        store.mid = "Ymid".into();
        store.save(&path).unwrap();
        let loaded = SessionStore::load(&path).unwrap();
        assert_eq!(loaded.sessionid, "12345:abc:def");
        assert_eq!(loaded.user_id, "12345");
        assert_eq!(loaded.mid, "Ymid");
        assert!(loaded.device.is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn session_file_is_created_private() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("ig-pruner-mode-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.json");
        SessionStore::default().save(&path).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn jitter_in_range() {
        for _ in 0..20 {
            let v = jitter_u64(10, 20);
            assert!((10..=20).contains(&v));
        }
    }
}
