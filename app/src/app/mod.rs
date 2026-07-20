mod persist;
mod ui;

pub use ui::WINDOW_SIZE;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use squeezeit::{
    BackupVault, GpuContext, ProcessingBackend, SqueezeReport, SqueezeSettings, SqueezerRegistry,
    batch, rpf::GtaKeys,
};

use crate::logging::LOG_CAPACITY;
use crossbeam_queue::ArrayQueue;
use std::collections::VecDeque;
use tracing::Level;

#[derive(Clone)]
pub(crate) enum Workspace {
    Folder(PathBuf),
    Files(Vec<PathBuf>),
}

impl Workspace {
    pub(crate) fn root(&self) -> PathBuf {
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
    advanced_open: bool,
    selected: usize,
    hits: Vec<(ratatui::layout::Rect, ui::Hit)>,
    quit_requested: bool,

    gta_keys: Option<Arc<GtaKeys>>,

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
    pub fn set_gta_install_path(&mut self, path: Option<PathBuf>) {
        self.settings.gta_install_path = path;
        self.gta_keys = None;
        if let Some(ref exe_path) = self.settings.gta_install_path {
            if exe_path.exists() {
                match squeezeit::gta_keys::ExtractedKeys::extract_from_exe(exe_path) {
                    Ok(keys) => {
                        self.gta_keys = Some(Arc::new(keys.to_rpf_archive_keys()));
                        tracing::info!("GTA V keys extracted from {}", exe_path.display());
                    }
                    Err(e) => {
                        tracing::warn!("Failed to extract GTA V keys: {e}");
                    }
                }
            }
        }
    }

    pub fn load(log_queue: Arc<ArrayQueue<(Level, String)>>, gpu: Option<Arc<GpuContext>>) -> Self {
        let mut app = Self {
            workspace: None,
            settings: SqueezeSettings::default(),
            advanced_open: false,
            selected: 0,
            hits: Vec::new(),
            quit_requested: false,
            gta_keys: None,
            report: Arc::new(SqueezeReport::new()),
            running: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            log_queue,
            log_messages: VecDeque::with_capacity(LOG_CAPACITY),
            gpu,
            done_rx: None,
            last_saved: String::new(),
        };
        if let Some(encoded) =
            persist::settings_path().and_then(|p| std::fs::read_to_string(p).ok())
        {
            app.decode_settings(encoded.trim());
            app.last_saved = app.encode_settings();
        }
        if let Some(ref exe_path) = app.settings.gta_install_path {
            if exe_path.exists() {
                match squeezeit::gta_keys::ExtractedKeys::extract_from_exe(exe_path) {
                    Ok(keys) => {
                        app.gta_keys = Some(Arc::new(keys.to_rpf_archive_keys()));
                    }
                    Err(_) => {}
                }
            }
        }
        if app.settings.backend == ProcessingBackend::GPU
            && let Some(gpu) = &app.gpu
        {
            gpu.warm_up();
        }
        app
    }

    fn start(&mut self) {
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        let settings = self.settings.clone();
        let report = Arc::clone(&self.report);
        let running = Arc::clone(&self.running);
        let cancel = Arc::clone(&self.cancel);

        let gpu = (settings.backend == ProcessingBackend::GPU)
            .then(|| self.gpu.clone())
            .flatten();

        let gta_keys = if self.gta_keys.is_some() {
            self.gta_keys.clone()
        } else if let Some(ref exe_path) = settings.gta_install_path {
            if exe_path.exists() {
                match squeezeit::gta_keys::ExtractedKeys::extract_from_exe(exe_path) {
                    Ok(keys) => Some(Arc::new(keys.to_rpf_archive_keys())),
                    Err(e) => {
                        tracing::warn!("Failed to extract GTA V keys: {e}");
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        running.store(true, Ordering::SeqCst);
        cancel.store(false, Ordering::SeqCst);

        while self.log_queue.pop().is_some() {}
        self.log_messages.clear();

        let (done_tx, done_rx) = mpsc::channel();
        self.done_rx = Some(done_rx);

        report.reset_timer();

        std::thread::spawn(move || {
            let registry = match gta_keys {
                Some(keys) => SqueezerRegistry::with_gta_keys_gpu(keys, gpu),
                None => SqueezerRegistry::with_defaults_gpu(gpu),
            };
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
            match BackupVault::new(&folder, None).and_then(|v| v.restore_all()) {
                Ok(count) => tracing::info!("restored {count} file(s) from the backup vault"),
                Err(e) => tracing::error!("restore failed: {e}"),
            }
            running.store(false, Ordering::SeqCst);
        });
    }
}
