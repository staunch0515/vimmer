//! The Anthropic API key for the Claude helper, shared by "generate from text" and the editing
//! assistant.
//!
//! When `ANTHROPIC_API_KEY` is in this process's environment the helper inherits it, as in the
//! CLI. Otherwise (the app was started from Finder) it is read from the user's login shell once,
//! kept in memory only, and handed to the helper process alone (`LlmHelper.env`). It is never
//! written to an event, an error, a log, a file or the settings; as a last line of defence every
//! message is passed through [`scrub`] before it leaves the calling module.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use vimmer_storyboard::generate::{LlmHelper, ENV_API_KEY};

pub const NO_KEY_MESSAGE: &str =
    "ANTHROPIC_API_KEY was not found. Set ANTHROPIC_API_KEY in ~/.zshrc (export ANTHROPIC_API_KEY=...), then try again.";

/// The key as read from the user's shell. Memory only; only a found key is remembered, so
/// adding it to ~/.zshrc works without restarting the app.
static SHELL_KEY: Mutex<Option<String>> = Mutex::new(None);

/// Where the helper's key comes from.
#[derive(PartialEq)]
pub enum HelperKey {
    /// This process has `ANTHROPIC_API_KEY`; the helper inherits it.
    Inherited,
    /// Read from the login shell; give it to the helper with [`HelperKey::give_to`].
    FromShell(String),
}

impl HelperKey {
    /// The key the caller must scrub from its messages (`None` when inherited: this module
    /// never holds that value).
    pub fn secret(&self) -> Option<&str> {
        match self {
            HelperKey::Inherited => None,
            HelperKey::FromShell(k) => Some(k),
        }
    }

    /// Hands the key to the helper process only.
    pub fn give_to(&self, helper: &mut LlmHelper) {
        if let HelperKey::FromShell(key) = self {
            helper.env.set(ENV_API_KEY, key.clone());
        }
    }
}

// No `Debug` that could print the value.
impl std::fmt::Debug for HelperKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            HelperKey::Inherited => "Inherited",
            HelperKey::FromShell(_) => "FromShell(..)",
        })
    }
}

/// The key for the helper, or `None` when neither this process nor the login shell has it.
pub fn helper_key() -> Option<HelperKey> {
    if key_in_env() {
        return Some(HelperKey::Inherited);
    }
    let mut cached = SHELL_KEY.lock().unwrap_or_else(|e| e.into_inner());
    if cached.is_none() {
        *cached = read_key_from_shell();
    }
    cached.clone().map(HelperKey::FromShell)
}

fn key_in_env() -> bool {
    std::env::var_os(ENV_API_KEY).is_some_and(|v| !v.to_string_lossy().trim().is_empty())
}

/// `$ANTHROPIC_API_KEY` as a login shell that has read `~/.zshrc` sees it. Only stdout is read;
/// nothing is logged.
fn read_key_from_shell() -> Option<String> {
    if !Path::new("/bin/zsh").is_file() {
        return None;
    }
    let script = format!(r#"source ~/.zshrc >/dev/null 2>&1; printf %s "${ENV_API_KEY}""#);
    let out = Command::new("/bin/zsh")
        .args(["-lc", &script])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|k| !k.is_empty())
}

/// `text` without `secret` in it.
pub fn scrub(text: &str, secret: Option<&str>) -> String {
    match secret.filter(|s| s.len() >= 8) {
        Some(s) => text.replace(s, "[api key removed]"),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_scrubbed_and_never_debug_printed() {
        assert_eq!(scrub("a sk-fake-test-value-000 b", Some("sk-fake-test-value-000")), "a [api key removed] b");
        assert_eq!(scrub("short secrets are left", Some("ab")), "short secrets are left");
        let key = HelperKey::FromShell("sk-fake-test-value-000".into());
        assert!(!format!("{key:?}").contains("sk-fake"));
        let mut helper = LlmHelper::locate(Some(Path::new("/bin/sh"))).unwrap();
        key.give_to(&mut helper);
        assert_eq!(helper.env.names(), [ENV_API_KEY]);
        assert!(!format!("{helper:?}").contains("sk-fake"));
        let mut inherited = LlmHelper::locate(Some(Path::new("/bin/sh"))).unwrap();
        HelperKey::Inherited.give_to(&mut inherited);
        assert!(inherited.env.is_empty() && HelperKey::Inherited.secret().is_none());
    }
}
