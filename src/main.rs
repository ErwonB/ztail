use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use zellij_tile::prelude::*;

const DEFAULT_POLL_INTERVAL: f64 = 2.0;
const HOST_PREFIX: &str = "/host";

struct State {
    patterns: Vec<String>,
    known_files: HashSet<String>,
    poll_interval: f64,
    active: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            patterns: Vec::new(),
            known_files: HashSet::new(),
            poll_interval: DEFAULT_POLL_INTERVAL,
            active: false,
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

        request_permission(&[PermissionType::RunCommands, PermissionType::FullHdAccess]);

        subscribe(&[EventType::Timer, EventType::PermissionRequestResult]);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PermissionRequestResult(status) => {
                if status == PermissionStatus::Granted {
                    self.active = true;
                    // Remap /host to the filesystem root so glob patterns work
                    // with absolute paths like /tmp/*.log -> /host/tmp/*.log
                    change_host_folder(PathBuf::from("/"));
                    self.scan_for_new_files();
                    set_timeout(self.poll_interval);
                    hide_self();
                }
            }
            Event::Timer(_elapsed) => {
                if self.active {
                    self.scan_for_new_files();
                    set_timeout(self.poll_interval);
                }
            }
            _ => {}
        }
        false
    }

    fn render(&mut self, _rows: usize, _cols: usize) {}
}

impl State {
    fn scan_for_new_files(&mut self) {
        for pattern in &self.patterns.clone() {
            let host_pattern = format!("{}{}", HOST_PREFIX, pattern);

            match glob::glob(&host_pattern) {
                Ok(paths) => {
                    for entry in paths {
                        match entry {
                            Ok(path) => {
                                let real_path = path
                                    .to_string_lossy()
                                    .strip_prefix(HOST_PREFIX)
                                    .unwrap_or(&path.to_string_lossy())
                                    .to_string();

                                if !self.known_files.contains(&real_path) {
                                    self.known_files.insert(real_path.clone());
                                    self.open_tail_pane(&real_path);
                                }
                            }
                            Err(e) => {
                                eprintln!("[ztail] Glob entry error: {}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[ztail] Glob error for '{}': {}", host_pattern, e);
                }
            }
        }
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
