//! All persistence lives here. It is JSON today; the API is deliberately narrow so a
//! different backing store (sqlite, if per-app history or frequency ranking ever lands)
//! is a change to this file alone.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const MAX_RECENTS: usize = 48;

#[derive(Default, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    recents: Vec<String>,
    #[serde(default)]
    restore_token: Option<String>,
    /// Set once the portal has been refused, so we nag about Ctrl+V only the first time.
    #[serde(default)]
    paste_denied: bool,
}

fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("emoji-picker/state.json"))
}

impl Store {
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            // Write-then-rename so a crash mid-write cannot leave a truncated file.
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
    }

    pub fn recents(&self) -> &[String] {
        &self.recents
    }

    pub fn record_use(&mut self, ch: &str) {
        self.recents.retain(|r| r != ch);
        self.recents.insert(0, ch.to_owned());
        self.recents.truncate(MAX_RECENTS);
    }

    pub fn restore_token(&self) -> Option<&str> {
        self.restore_token.as_deref()
    }

    pub fn set_restore_token(&mut self, token: Option<String>) {
        if token.is_some() {
            self.restore_token = token;
        }
    }

    pub fn paste_denied(&self) -> bool {
        self.paste_denied
    }

    pub fn set_paste_denied(&mut self, denied: bool) {
        self.paste_denied = denied;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_use_dedupes_and_moves_to_front() {
        let mut s = Store::default();
        s.record_use("a");
        s.record_use("b");
        s.record_use("a");
        assert_eq!(s.recents(), ["a", "b"]);
    }

    #[test]
    fn recents_are_capped() {
        let mut s = Store::default();
        for i in 0..100 {
            s.record_use(&i.to_string());
        }
        assert_eq!(s.recents().len(), MAX_RECENTS);
        assert_eq!(s.recents()[0], "99");
    }
}
