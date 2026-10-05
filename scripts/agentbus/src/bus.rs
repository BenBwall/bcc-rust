#[path = "store.rs"]
mod store;
use std::{
    env,
    fs,
    path::{
        Path,
        PathBuf,
    },
};

use chrono::{
    SecondsFormat,
    Utc,
};
use sha2::{
    Digest,
    Sha256,
};
pub use store::{
    Bus,
    DeliveryFilter,
};

pub type Result<T> = std::result::Result<T, String>;
pub const BROADCAST: &str = "all";
pub const EVERY_AGENT: &str = "*";
pub const HOOK_MESSAGE_LIMIT: usize = 10;
pub const HOOK_BODY_LIMIT: usize = 4000;
pub const MAX_STOP_BLOCKS: i64 = 5;
pub const AFK_CONTINUATIONS_PER_HOUR: usize = 120;

pub fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn validate_agent(name: &str, broadcast: bool) -> Result<String> {
    let name = name.trim().to_ascii_lowercase();
    if broadcast && name == BROADCAST {
        return Ok(name);
    }
    let valid = (1..=32).contains(&name.len())
        && name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
    if !valid || name == BROADCAST {
        return Err(format!("invalid agent name {name:?}"));
    }
    Ok(name)
}

pub fn git_common_dir(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let dot_git = dir.join(".git");
        if dot_git.is_dir() {
            return Some(dot_git);
        }
        if dot_git.is_file() {
            let entry = fs::read_to_string(dot_git).ok()?;
            let relative = entry.trim().strip_prefix("gitdir:")?.trim();
            let git_dir = if Path::new(relative).is_absolute() {
                PathBuf::from(relative)
            } else {
                dir.join(relative)
            };
            let git_dir = git_dir.canonicalize().ok()?;
            let common = git_dir.join("commondir");
            if let Ok(text) = fs::read_to_string(common) {
                let path = PathBuf::from(text.trim());
                return if path.is_absolute() {
                    Some(path)
                } else {
                    git_dir.join(path).canonicalize().ok()
                };
            }
            return Some(git_dir);
        }
    }
    None
}

pub fn default_db_path() -> PathBuf {
    if let Some(path) = env::var_os("AGENTBUS_DB").filter(|v| !v.is_empty()) {
        return PathBuf::from(path);
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    if let Some(common) = git_common_dir(root) {
        return common.join("agentbus/bus.db");
    }
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .unwrap_or_default();
    PathBuf::from(home).join(".agentbus/bus.db")
}

pub struct SessionHooks {
    directory: PathBuf,
}
impl SessionHooks {
    pub fn new(db_path: &Path) -> Self {
        Self {
            directory: db_path.with_file_name(format!(
                "{}.sessions",
                db_path.file_name().unwrap_or_default().to_string_lossy()
            )),
        }
    }

    fn path(&self, session_id: &str) -> Result<PathBuf> {
        let id = session_id.trim();
        if id.is_empty() {
            return Err("session id is empty".into());
        }
        let digest = Sha256::digest(id.as_bytes());
        Ok(self.directory.join(format!("{digest:x}")))
    }

    pub fn enabled(&self, session_id: &str) -> bool {
        self.path(session_id).is_ok_and(|p| p.is_file())
    }

    pub fn set_enabled(&self, session_id: &str, enabled: bool) -> Result<()> {
        let path = self.path(session_id)?;
        if enabled {
            fs::create_dir_all(&self.directory).map_err(|e| e.to_string())?;
            fs::write(path, session_id.trim()).map_err(|e| e.to_string())?;
        } else if path.exists() {
            fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

pub fn current_session_id(explicit: Option<&str>) -> Result<String> {
    if let Some(id) = explicit {
        return if id.trim().is_empty() {
            Err("session id is empty".into())
        } else {
            Ok(id.trim().into())
        };
    }
    let mut ids = Vec::new();
    for name in ["CODEX_THREAD_ID", "CLAUDE_CODE_SESSION_ID"] {
        if let Ok(id) = env::var(name)
            && !id.trim().is_empty()
            && !ids.contains(&id.trim().to_string())
        {
            ids.push(id.trim().to_string());
        }
    }
    if ids.len() == 1 {
        Ok(ids.remove(0))
    } else {
        Err("cannot identify this session; pass --session SESSION_ID explicitly".into())
    }
}
