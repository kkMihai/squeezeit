mod persist;
mod ui;

pub use ui::WINDOW_SIZE;

use crate::logging::LOG_CAPACITY;
use crossbeam_queue::ArrayQueue;
use squeezeit::{
    Backend, BackupVault, GpuContext, SqueezeReport, SqueezeSettings, SqueezerRegistry, batch,
};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use tracing::Level;

#[derive(Clone)]
pub(crate) enum Workspace {
    Folder(PathBuf),
    Files(Vec<PathBuf>),
}

impl Workspace {
    fn root(&self) -> PathBuf {
        match self {
            Self::Folder(folder) => folder.clone(),
            Self::Files(files) => files
                .first()
                .and_then(|f| f.parent())
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from(".")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Setup,
    Running,
    Done,
}

pub struct SqueezeItApp {
    workspace: Option<Workspace>,
    settings: SqueezeSettings,
    quiet: bool,
    settings_open: bool,
    selected: usize,
    hits: Vec<(ratatui::layout::Rect, ui::Hit)>,
    quit_requested: bool,
    report: Arc<SqueezeReport>,
    running: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    log_queue: Arc<ArrayQueue<(Level, String)>>,
    log_messages: VecDeque<(Level, String)>,
    gpu: Option<Arc<GpuContext>>,
    done_rx: Option<mpsc::Receiver<()>>,
    last_saved: String,
}

impl SqueezeItApp {
    pub fn load(log_queue: Arc<ArrayQueue<(Level, String)>>, gpu: Option<Arc<GpuContext>>) -> Self {
        let mut app = Self {
            workspace: None,
            settings: SqueezeSettings::default(),
            quiet: false,
            settings_open: false,
            selected: 0,
            hits: Vec::new(),
            quit_requested: false,
            report: Arc::new(SqueezeReport::new()),
            running: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            log_queue,
            log_messages: VecDeque::with_capacity(LOG_CAPACITY),
            gpu,
            done_rx: None,
            last_saved: String::new(),
        };

        if let Some(saved) = persist::settings_path().and_then(|p| std::fs::read_to_string(p).ok())
        {
            app.decode_settings(saved.trim());
            app.last_saved = app.encode_settings();
        }
        if app.settings.backend == Backend::Gpu
            && let Some(gpu) = &app.gpu
        {
            gpu.begin_warm_up();
        }
        app
    }

    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self {
            workspace: None,
            settings: SqueezeSettings::default(),
            quiet: false,
            settings_open: true,
            selected: 0,
            hits: Vec::new(),
            quit_requested: false,
            report: Arc::new(SqueezeReport::new()),
            running: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            log_queue: Arc::new(ArrayQueue::new(LOG_CAPACITY)),
            log_messages: VecDeque::new(),
            gpu: None,
            done_rx: None,
            last_saved: String::new(),
        }
    }

    pub(crate) fn phase(&self) -> Phase {
        if self.running.load(Ordering::SeqCst) {
            return Phase::Running;
        }
        if self.report.snapshot().total_files > 0 {
            Phase::Done
        } else {
            Phase::Setup
        }
    }

    fn load_exclusions(&mut self, path: &Path) {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                self.settings.exclusions.names = text
                    .lines()
                    .map(|line| line.split('#').next().unwrap_or_default().trim())
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned)
                    .collect();
                tracing::info!(
                    "skipping {} texture name(s) from {}",
                    self.settings.exclusions.names.len(),
                    path.display()
                );
            }
            Err(e) => tracing::warn!("could not read {}: {e}", path.display()),
        }
    }

    fn has_backups(&self) -> bool {
        self.workspace
            .as_ref()
            .is_some_and(|w| BackupVault::has_backups(&w.root(), None))
    }

    fn start(&mut self) {
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        let settings = self.settings.clone();
        let report = Arc::clone(&self.report);
        let running = Arc::clone(&self.running);
        let cancel = Arc::clone(&self.cancel);
        let gpu = (settings.backend == Backend::Gpu)
            .then(|| self.gpu.clone())
            .flatten();

        running.store(true, Ordering::SeqCst);
        cancel.store(false, Ordering::SeqCst);

        while self.log_queue.pop().is_some() {}
        self.log_messages.clear();

        let (done_tx, done_rx) = mpsc::channel();
        self.done_rx = Some(done_rx);
        report.reset_timer();

        std::thread::spawn(move || {
            let gpu = gpu.filter(|ctx| {
                let ready = ctx.wait_until_ready();
                if !ready {
                    tracing::warn!("the GPU compressor never came up, running on the CPU");
                }
                ready
            });
            let registry = SqueezerRegistry::new(gpu);
            let root = workspace.root();
            let targets = match &workspace {
                Workspace::Folder(folder) => batch::collect_targets(folder, &registry),
                Workspace::Files(files) => files
                    .iter()
                    .filter(|f| registry.claims(f))
                    .cloned()
                    .collect(),
            };
            let result = batch::run_targets(
                &targets, &root, &registry, &settings, None, &report, &cancel,
            );
            match result {
                Ok(()) if cancel.load(Ordering::SeqCst) => tracing::warn!("batch cancelled"),
                Ok(()) => tracing::info!("batch finished"),
                Err(e) => tracing::error!("batch aborted: {e}"),
            }
            report.mark_finished();
            running.store(false, Ordering::SeqCst);
            let _ = done_tx.send(());
        });
    }

    fn restore(&self) {
        let Some(folder) = self.workspace.as_ref().map(Workspace::root) else {
            return;
        };
        let running = Arc::clone(&self.running);
        running.store(true, Ordering::SeqCst);

        std::thread::spawn(move || {
            match BackupVault::new(&folder, None).and_then(BackupVault::restore_all) {
                Ok(count) => tracing::info!("restored {count} file(s) from the backup vault"),
                Err(e) => tracing::error!("restore failed: {e}"),
            }
            running.store(false, Ordering::SeqCst);
        });
    }
}
