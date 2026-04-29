use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use zellij_tile::prelude::*;

const DEFAULT_POLL_INTERVAL: f64 = 2.0;

struct State {
    patterns: Vec<String>,
    known_files: HashSet<String>,
    poll_interval: f64,
    active: bool,
    ignore_patterns: Vec<String>,
    /// Track whether we're doing the initial snapshot (don't open panes)
    snapshotting: bool,
    /// Track how many snapshot responses are still pending
    pending_snapshot_count: usize,
}

impl Default for State {
    fn default() -> Self {
        Self {
            patterns: Vec::new(),
            known_files: HashSet::new(),
            poll_interval: DEFAULT_POLL_INTERVAL,
            active: false,
            ignore_patterns: Vec::new(),
            snapshotting: false,
            pending_snapshot_count: 0,
        }
    }
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        let mut i = 0;
        loop {
            let key = format!("pattern_{}", i);
            match configuration.get(&key) {
                Some(pattern) => {
                    self.patterns.push(pattern.clone());
                    eprintln!("[ztail] Loaded pattern_{}: {}", i, pattern);
                    i += 1;
                }
                None => break,
            }
        }

        i = 0;
        loop {
            let key = format!("ignore_{}", i);
            match configuration.get(&key) {
                Some(pattern) => {
                    self.ignore_patterns.push(pattern.clone());
                    eprintln!("[ztail] Loaded ignore_{}: {}", i, pattern);
                    i += 1;
                }
                None => break,
            }
        }

        if let Some(interval_str) = configuration.get("poll_interval") {
            if let Ok(interval) = interval_str.parse::<f64>() {
                self.poll_interval = interval;
            }
        }

        // Restore known_files persisted from a previous session
        if let Ok(saved) = std::fs::read_to_string(Self::state_file_path()) {
            self.known_files = saved
                .lines()
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect();
            eprintln!(
                "[ztail] Restored {} known files from state",
                self.known_files.len()
            );
        }

        request_permission(&[PermissionType::RunCommands]);

        subscribe(&[
            EventType::Timer,
            EventType::PermissionRequestResult,
            EventType::RunCommandResult,
        ]);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PermissionRequestResult(status) => {
                if status == PermissionStatus::Granted {
                    self.active = true;
                    // Do initial snapshot: expand all patterns to record existing files.
                    // Only snapshot if we have no restored state; if state was restored we
                    // already know the existing files and can start watching immediately.
                    if self.known_files.is_empty() {
                        self.snapshotting = true;
                        self.pending_snapshot_count = self.patterns.len();
                    }
                    self.run_glob_commands();
                    set_timeout(self.poll_interval);
                    hide_self();
                }
            }
            Event::Timer(_elapsed) => {
                if self.active {
                    self.run_glob_commands();
                    set_timeout(self.poll_interval);
                }
            }
            Event::RunCommandResult(exit_code, stdout, stderr, context) => {
                self.handle_glob_result(exit_code, stdout, stderr, context);
            }
            _ => {}
        }
        false
    }

    fn render(&mut self, _rows: usize, _cols: usize) {}
}

/// Match a string against a glob pattern supporting `*` (any chars) and `?` (single char).
fn glob_match(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    let (plen, tlen) = (pat.len(), txt.len());

    let mut dp = vec![vec![false; tlen + 1]; plen + 1];
    dp[0][0] = true;

    for i in 1..=plen {
        if pat[i - 1] == '*' {
            dp[i][0] = dp[i - 1][0];
        } else {
            break;
        }
    }

    for i in 1..=plen {
        for j in 1..=tlen {
            if pat[i - 1] == '*' {
                dp[i][j] = dp[i - 1][j] || dp[i][j - 1];
            } else if pat[i - 1] == '?' || pat[i - 1] == txt[j - 1] {
                dp[i][j] = dp[i - 1][j - 1];
            }
        }
    }

    dp[plen][tlen]
}

impl State {
    /// Run `ls <pattern>` via bash for each configured pattern.
    /// Results come back asynchronously via RunCommandResult events.
    fn run_glob_commands(&self) {
        for (i, pattern) in self.patterns.iter().enumerate() {
            let mut context = BTreeMap::new();
            context.insert("pattern_index".to_string(), i.to_string());
            context.insert("pattern".to_string(), pattern.clone());

            // Use bash -c with ls to expand the glob on the host.
            // ls -1d lists one file per line. 2>/dev/null suppresses "no match" errors.
            // We use -d to avoid listing directory contents if a glob matches a dir.
            let shell_cmd = format!("ls -1d {} 2>/dev/null || true", pattern);

            run_command(&["bash", "-c", &shell_cmd], context);
        }
    }

    /// Handle the result of a glob expansion command.
    fn handle_glob_result(
        &mut self,
        _exit_code: Option<i32>,
        stdout: Vec<u8>,
        _stderr: Vec<u8>,
        context: BTreeMap<String, String>,
    ) {
        let pattern = match context.get("pattern") {
            Some(p) => p.clone(),
            None => return,
        };

        let output = String::from_utf8_lossy(&stdout);
        let files: Vec<&str> = output.lines().filter(|l| !l.is_empty()).collect();

        if self.snapshotting {
            eprintln!(
                "[ztail] Snapshot pattern '{}' -> {} existing files",
                pattern,
                files.len()
            );
            for file in &files {
                if !self.is_ignored(file) {
                    self.known_files.insert(file.to_string());
                }
            }

            // Only exit snapshot mode once ALL pattern results have returned
            self.pending_snapshot_count = self.pending_snapshot_count.saturating_sub(1);
            if self.pending_snapshot_count == 0 {
                self.snapshotting = false;
                eprintln!("[ztail] Snapshot complete, watching for new files.");
                self.persist_known_files();
            }
        } else {
            for file in &files {
                let file_str = file.to_string();
                if !self.is_ignored(&file_str) && !self.known_files.contains(&file_str) {
                    eprintln!("[ztail] New file detected: {}", file_str);
                    self.known_files.insert(file_str.clone());
                    self.persist_known_files();
                    self.open_tail_pane(&file_str);
                }
            }
        }
    }

    /// Persist the current known_files set so it survives session detach/reattach.
    fn persist_known_files(&self) {
        let serialized = self
            .known_files
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let path = Self::state_file_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(path, serialized) {
            eprintln!("[ztail] Failed to persist known files: {}", e);
        }
    }

    fn state_file_path() -> std::path::PathBuf {
        // Place the state file next to other zellij data, falling back to /tmp.
        let base = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"));
        base.join(".cache")
            .join("zellij")
            .join("ztail_known_files.txt")
    }

    /// Check if a file path matches any of the ignore patterns.
    fn is_ignored(&self, path: &str) -> bool {
        let file_name = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        for pattern in &self.ignore_patterns {
            if glob_match(pattern, path) || glob_match(pattern, &file_name) {
                return true;
            }
        }
        false
    }

    fn open_tail_pane(&self, file_path: &str) {
        let command = CommandToRun {
            path: PathBuf::from("tail"),
            args: vec!["-f".to_string(), file_path.to_string()],
            cwd: None,
        };
        open_command_pane_floating(command, None, BTreeMap::new());
    }
}
