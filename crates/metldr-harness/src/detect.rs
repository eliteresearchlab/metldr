//! Finds installed agent CLIs and desktop apps without running or changing them.

use crate::{HARNESSES, HarnessId};
use std::path::{Path, PathBuf};

/// A desktop app that can host the MetLDR MCP server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum App {
    ClaudeDesktop,
    CodexApp,
    OpenCode,
    Cursor,
    ChatGpt,
}

pub struct AppInfo {
    pub app: App,
    pub key: &'static str,
    pub label: &'static str,
    /// The app's own MCP configuration file when it exists.
    pub config: Option<PathBuf>,
    /// Whether MetLDR can add itself to a local configuration file for this app.
    pub local_config: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// The first executable for `harness` on `path` or in common user install folders.
pub fn executable(harness: HarnessId, path: Option<&std::ffi::OsStr>, home: &Path) -> Option<PathBuf> {
    let extra = [
        home.join(".local/bin"),
        home.join(".local/share/mise/shims"),
        home.join(".npm-global/bin"),
        home.join(".bun/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
    ];
    let dirs = path.into_iter().flat_map(std::env::split_paths).chain(extra);
    let names = harness.harness().executables;
    for dir in dirs {
        for name in names {
            for candidate in [dir.join(name), dir.join(format!("{name}.exe"))] {
                if is_executable(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Every registered harness with its executable when found.
pub fn executables(path: Option<&std::ffi::OsStr>, home: &Path) -> Vec<(HarnessId, Option<PathBuf>)> {
    HARNESSES.iter().map(|h| (h.id, executable(h.id, path, home))).collect()
}

/// Desktop apps and where each keeps its MCP configuration on `os`.
pub fn desktop_apps(home: &Path, os: Os, app_data: Option<&Path>) -> Vec<AppInfo> {
    let claude = match os {
        Os::MacOs => Some(home.join("Library/Application Support/Claude/claude_desktop_config.json")),
        Os::Windows => app_data.map(|dir| dir.join("Claude/claude_desktop_config.json")),
        // Claude Desktop has no official Linux build.
        Os::Linux => None,
    };
    let entries = [
        (App::ClaudeDesktop, "claude_desktop", "Claude Desktop", claude),
        (App::CodexApp, "codex_app", "Codex app", Some(home.join(".codex/config.toml"))),
        (App::OpenCode, "opencode_desktop", "OpenCode", Some(home.join(".config/opencode/opencode.json"))),
        (App::Cursor, "cursor", "Cursor", Some(home.join(".cursor/mcp.json"))),
        // ChatGPT adds MCP servers as remote connectors in its own settings, not a local file.
        (App::ChatGpt, "chatgpt", "ChatGPT", None),
    ];
    entries
        .into_iter()
        .map(|(app, key, label, path)| AppInfo {
            app,
            key,
            label,
            local_config: path.is_some(),
            config: path.filter(|path| path.is_file()),
        })
        .collect()
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_executables_and_app_configs_in_a_temporary_home() {
        let home = std::env::temp_dir().join(format!("metldr-detect-{}", std::process::id()));
        let bin = home.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(home.join(".cursor")).unwrap();
        std::fs::write(home.join(".cursor/mcp.json"), "{}").unwrap();
        let claude = bin.join("claude");
        std::fs::write(&claude, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::write(bin.join("codex"), "not executable").unwrap();
        }
        let path = std::ffi::OsString::from(bin.as_os_str());
        let found = executables(Some(&path), &home);
        assert_eq!(found.iter().find(|(id, _)| *id == HarnessId::ClaudeCode).unwrap().1.as_deref(), Some(claude.as_path()));
        #[cfg(unix)]
        assert!(found.iter().find(|(id, _)| *id == HarnessId::Codex).unwrap().1.is_none(), "a file without execute permission is not an agent");
        let apps = desktop_apps(&home, Os::Linux, None);
        let cursor = apps.iter().find(|a| a.app == App::Cursor).unwrap();
        assert_eq!(cursor.config.as_deref(), Some(home.join(".cursor/mcp.json").as_path()));
        let claude_desktop = apps.iter().find(|a| a.app == App::ClaudeDesktop).unwrap();
        assert!(!claude_desktop.local_config && claude_desktop.config.is_none());
        let _ = std::fs::remove_dir_all(&home);
    }
}
