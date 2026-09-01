use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::Config;

const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(250);

enum WatchMessage {
    Changed,
    Error(notify::Error),
}

/// Watches every `xenomorph.toml` file contributing to the effective config.
///
/// Parent directories are watched so changes are still detected when an editor
/// saves by replacing or renaming a config file. Duplicate filesystem events
/// from one save are debounced into a single callback.
pub struct WorkspaceConfigWatcher {
    config_paths: Vec<PathBuf>,
    watcher: Option<RecommendedWatcher>,
    worker: Option<JoinHandle<()>>,
}

impl WorkspaceConfigWatcher {
    /// Starts watching the config file used by [`Config::get`].
    pub fn watch(
        on_event: impl FnMut(notify::Result<()>) + Send + 'static,
    ) -> notify::Result<Self> {
        Self::watch_paths(Config::get().workspace_config_paths(), on_event)
    }

    /// Starts watching a specific config path.
    pub fn watch_path(
        config_path: impl Into<PathBuf>,
        on_event: impl FnMut(notify::Result<()>) + Send + 'static,
    ) -> notify::Result<Self> {
        Self::watch_paths_with_debounce(vec![config_path.into()], DEFAULT_DEBOUNCE, on_event)
    }

    /// Starts watching multiple config paths as one debounced config source.
    pub fn watch_paths(
        config_paths: Vec<PathBuf>,
        on_event: impl FnMut(notify::Result<()>) + Send + 'static,
    ) -> notify::Result<Self> {
        Self::watch_paths_with_debounce(config_paths, DEFAULT_DEBOUNCE, on_event)
    }

    /// Returns the authoritative config file monitored by this watcher.
    pub fn config_path(&self) -> &Path {
        &self.config_paths[0]
    }

    /// Returns all config files monitored by this watcher.
    pub fn config_paths(&self) -> &[PathBuf] {
        &self.config_paths
    }

    fn watch_paths_with_debounce(
        config_paths: Vec<PathBuf>,
        debounce: Duration,
        mut on_event: impl FnMut(notify::Result<()>) + Send + 'static,
    ) -> notify::Result<Self> {
        if config_paths.is_empty() {
            return Err(notify::Error::generic("no config paths were provided"));
        }
        for config_path in &config_paths {
            if config_path.parent().is_none() {
                return Err(notify::Error::generic(&format!(
                    "config path '{}' has no parent directory",
                    config_path.display()
                )));
            }
        }

        let watched_paths = config_paths.clone();
        let (sender, receiver) = mpsc::channel();

        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<Event>| match result {
                Ok(event)
                    if watched_paths
                        .iter()
                        .any(|config_path| event_targets_config(&event, config_path)) =>
                {
                    let _ = sender.send(WatchMessage::Changed);
                }
                Ok(_) => {}
                Err(error) => {
                    let _ = sender.send(WatchMessage::Error(error));
                }
            })?;
        let mut watched_parents = HashSet::new();
        for config_path in &config_paths {
            let parent = config_path.parent().ok_or_else(|| {
                notify::Error::generic(&format!(
                    "config path '{}' has no parent directory",
                    config_path.display()
                ))
            })?;
            if watched_parents.insert(parent.to_path_buf()) {
                watcher.watch(parent, RecursiveMode::NonRecursive)?;
            }
        }

        let worker = thread::spawn(move || loop {
            match receiver.recv() {
                Ok(WatchMessage::Changed) => loop {
                    match receiver.recv_timeout(debounce) {
                        Ok(WatchMessage::Changed) => {}
                        Ok(WatchMessage::Error(error)) => on_event(Err(error)),
                        Err(RecvTimeoutError::Timeout) => {
                            on_event(Ok(()));
                            break;
                        }
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                },
                Ok(WatchMessage::Error(error)) => on_event(Err(error)),
                Err(_) => return,
            }
        });

        Ok(Self {
            config_paths,
            watcher: Some(watcher),
            worker: Some(worker),
        })
    }
}

impl Drop for WorkspaceConfigWatcher {
    fn drop(&mut self) {
        // Dropping the native watcher also drops its callback and closes the
        // worker channel, allowing the debounce worker to terminate cleanly.
        self.watcher.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn event_targets_config(event: &Event, config_path: &Path) -> bool {
    let is_change = matches!(
        event.kind,
        EventKind::Any | EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    );
    is_change
        && event
            .paths
            .iter()
            .any(|event_path| paths_match(event_path, config_path))
}

fn paths_match(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{event_targets_config, WorkspaceConfigWatcher};
    use notify::{
        event::{AccessKind, ModifyKind},
        Event, EventKind,
    };
    use std::fs;
    use std::path::Path;
    use std::sync::mpsc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn targets_only_changes_to_the_selected_config() {
        let config = Path::new("/workspace/xenomorph.toml");
        let config_event =
            Event::new(EventKind::Modify(ModifyKind::Any)).add_path(config.to_path_buf());
        let other_event = Event::new(EventKind::Modify(ModifyKind::Any))
            .add_path(Path::new("/workspace/nested/xenomorph.toml").to_path_buf());

        assert!(event_targets_config(&config_event, config));
        assert!(!event_targets_config(&other_event, config));
    }

    #[test]
    fn ignores_non_mutating_access_events() {
        let config = Path::new("/workspace/xenomorph.toml");
        let event = Event::new(EventKind::Access(AccessKind::Read)).add_path(config.to_path_buf());

        assert!(!event_targets_config(&event, config));
    }

    #[test]
    fn watches_real_config_file_changes() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("xenomorph-watcher-{unique}"));
        let config_path = root.join("xenomorph.toml");
        fs::create_dir_all(&root).expect("test directory should be created");
        fs::write(&config_path, "[debug]\nloglevel = \"info\"\n")
            .expect("initial config should be written");

        let (sender, receiver) = mpsc::channel();
        let watcher = WorkspaceConfigWatcher::watch_path(&config_path, move |event| {
            let _ = sender.send(event.map_err(|error| error.to_string()));
        })
        .expect("config watcher should start");
        fs::write(&config_path, "[debug]\nloglevel = \"warning\"\n")
            .expect("config change should be written");

        assert_eq!(
            receiver
                .recv_timeout(Duration::from_secs(5))
                .expect("config change should be observed"),
            Ok(())
        );

        drop(watcher);
        fs::remove_dir_all(root).expect("test directory should be removed");
    }

    #[test]
    fn watches_changes_to_extended_configs() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("xenomorph-chain-watcher-{unique}"));
        let inner_dir = root.join("shared");
        let outer_config = root.join("xenomorph.toml");
        let inner_config = inner_dir.join("xenomorph.toml");
        fs::create_dir_all(&inner_dir).expect("test directories should be created");
        fs::write(&outer_config, "extends = \"./shared/xenomorph.toml\"\n")
            .expect("outer config should be written");
        fs::write(&inner_config, "abstract = true\n").expect("inner config should be written");

        let (sender, receiver) = mpsc::channel();
        let watcher = WorkspaceConfigWatcher::watch_paths(
            vec![outer_config, inner_config.clone()],
            move |event| {
                let _ = sender.send(event.map_err(|error| error.to_string()));
            },
        )
        .expect("config chain watcher should start");
        fs::write(&inner_config, "abstract = true\n[parser]\nentry = \"ui\"\n")
            .expect("inner config change should be written");

        assert_eq!(
            receiver
                .recv_timeout(Duration::from_secs(5))
                .expect("extended config change should be observed"),
            Ok(())
        );

        drop(watcher);
        fs::remove_dir_all(root).expect("test directory should be removed");
    }
}
