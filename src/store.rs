//! All persistence lives here. It is JSON today; the API is deliberately narrow so a
//! different backing store (sqlite, if per-app history or frequency ranking ever lands)
//! is a change to this file alone.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Default cap on remembered emoji; 4 rows of the picker's 12 columns.
pub const DEFAULT_RECENT_LIMIT: usize = 48;
/// Bounds offered in the settings window.
pub const RECENT_LIMIT_RANGE: (usize, usize) = (12, 240);

/// User-facing preferences, edited in the settings window and saved alongside the
/// recents. Command-line flags override these for a single run.
#[derive(Serialize, Deserialize)]
pub struct Settings {
    /// Insert into the focused field. Off means copy to the clipboard and stop.
    #[serde(default = "yes")]
    pub insert: bool,
    /// Also put the emoji on the clipboard when it was inserted directly.
    #[serde(default)]
    pub always_copy: bool,
    /// How many recently used emoji to keep.
    #[serde(default = "default_recent_limit")]
    pub recent_limit: usize,
    /// Fitzpatrick skin tone applied to emoji that take one: 0 is the default yellow,
    /// 1-5 are light through dark.
    #[serde(default)]
    pub skin_tone: u8,
}

fn default_recent_limit() -> usize {
    DEFAULT_RECENT_LIMIT
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            insert: true,
            always_copy: false,
            recent_limit: DEFAULT_RECENT_LIMIT,
            skin_tone: 0,
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    recents: Vec<String>,
    #[serde(default)]
    restore_token: Option<String>,
    /// Set once the portal has been refused, so we nag about Ctrl+V only the first time.
    #[serde(default)]
    paste_denied: bool,
    #[serde(default)]
    settings: Settings,
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
        let limit = self.settings.recent_limit.clamp(RECENT_LIMIT_RANGE.0, RECENT_LIMIT_RANGE.1);
        self.recents.truncate(limit);
    }

    /// Apply a newly chosen limit right away, rather than waiting for the list to grow
    /// back into it.
    pub fn set_recent_limit(&mut self, limit: usize) {
        let limit = limit.clamp(RECENT_LIMIT_RANGE.0, RECENT_LIMIT_RANGE.1);
        self.settings.recent_limit = limit;
        self.recents.truncate(limit);
    }

    pub fn restore_token(&self) -> Option<&str> {
        self.restore_token.as_deref()
    }

    pub fn set_restore_token(&mut self, token: Option<String>) {
        if token.is_some() {
            self.restore_token = token;
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn settings_mut(&mut self) -> &mut Settings {
        &mut self.settings
    }

    pub fn clear_recents(&mut self) {
        self.recents.clear();
    }

    /// Forget the portal permission, so KDE asks again on the next fallback paste.
    pub fn clear_restore_token(&mut self) {
        self.restore_token = None;
        self.paste_denied = false;
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
        for i in 0..500 {
            s.record_use(&i.to_string());
        }
        assert_eq!(s.recents().len(), DEFAULT_RECENT_LIMIT);
        assert_eq!(s.recents()[0], "499");
    }

    #[test]
    fn lowering_the_limit_trims_immediately() {
        let mut s = Store::default();
        for i in 0..60 {
            s.record_use(&i.to_string());
        }
        s.set_recent_limit(12);
        assert_eq!(s.recents().len(), 12);
        assert_eq!(s.recents()[0], "59");
    }

    #[test]
    fn the_limit_is_clamped_to_the_offered_range() {
        let mut s = Store::default();
        s.set_recent_limit(9999);
        assert_eq!(s.settings().recent_limit, RECENT_LIMIT_RANGE.1);
        s.set_recent_limit(0);
        assert_eq!(s.settings().recent_limit, RECENT_LIMIT_RANGE.0);
    }
}
