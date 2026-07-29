mod persist;
mod ui;

pub use ui::WINDOW_SIZE;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use crossbeam_queue::ArrayQueue;
use squeezeit::{
    Backend, BackupVault, GpuContext, GtaKeys, SqueezeReport, SqueezeSettings, SqueezerRegistry,
    batch,
};
use tracing::Level;

use crate::logging::LOG_CAPACITY;

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

pub struct SqueezeItApp {
    workspace: Option<Workspace>,
    settings: SqueezeSettings,

    headless: bool,
    gta_exe: Option<PathBuf>,
    gta_keys: Option<Arc<GtaKeys>>,

    advanced_open: bool,
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
            headless: false,
            gta_exe: None,
            gta_keys: None,
            advanced_open: false,
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
        app.load_gta_keys();
        if app.settings.backend == Backend::Gpu
            && let Some(gpu) = &app.gpu
        {
            gpu.warm_up();
        }
        app
    }

    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self {
            workspace: None,
            settings: SqueezeSettings::default(),
            headless: false,
            gta_exe: None,
            gta_keys: None,
            advanced_open: true,
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

    fn set_gta_exe(&mut self, path: Option<PathBuf>) {
        self.gta_exe = path;
        self.load_gta_keys();
    }

    fn load_gta_keys(&mut self) {
        self.gta_keys = None;
        let Some(exe) = self.gta_exe.as_deref().filter(|p| p.exists()) else {
            return;
        };
        match squeezeit::gta_keys::from_exe(exe) {
            Ok(keys) => {
                tracing::info!("GTA V keys extracted from {}", exe.display());
                self.gta_keys = Some(Arc::new(keys));
            }
            Err(e) => tracing::warn!("failed to extract GTA V keys: {e}"),
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
        let keys = self.gta_keys.clone();
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
            let registry = SqueezerRegistry::new(gpu, keys);
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
