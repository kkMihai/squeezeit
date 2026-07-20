#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod app;
mod logging;

use std::sync::Arc;

use tracing::Level;
use tracing_subscriber::Layer;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::prelude::*;

use app::SqueezeItApp;
use logging::{FeedLayer, Log};

#[cfg(windows)]
fn ensure_own_window() {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Console::GetConsoleProcessList;

    if std::env::var_os("SQUEEZEIT_OWN_WINDOW").is_some() {
        return;
    }
    let mut pids = [0u32; 2];
    let shared = unsafe { GetConsoleProcessList(pids.as_mut_ptr(), 2) } > 1;
    if !shared {
        return;
    }
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    if std::process::Command::new(exe)
        .env("SQUEEZEIT_OWN_WINDOW", "1")
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .is_ok()
    {
        std::process::exit(0);
    }
}

fn main() -> std::io::Result<()> {
    #[cfg(windows)]
    ensure_own_window();

    squeezeit::platform::sustain_background_performance();

    let log = Log::new();
    tracing_subscriber::registry()
        .with(
            FeedLayer {
                queue: Arc::clone(&log.queue),
            }
            .with_filter(
                Targets::new()
                    .with_target("squeezeit", Level::DEBUG)
                    .with_target("squeezeit_app", Level::DEBUG),
            ),
        )
        .init();

    let gpu = squeezeit::GpuContext::create_standalone()
        .ok()
        .map(Arc::new);

    let mut terminal = ratatui::init();
    let (cols, rows) = app::WINDOW_SIZE;
    let _ = ratatui::crossterm::execute!(
        std::io::stdout(),
        ratatui::crossterm::terminal::SetTitle("SqueezeIt"),
        ratatui::crossterm::terminal::SetSize(cols, rows),
        ratatui::crossterm::event::EnableMouseCapture,
    );
    let result = SqueezeItApp::load(Arc::clone(&log.queue), gpu).run(&mut terminal);
    let _ = ratatui::crossterm::execute!(
        std::io::stdout(),
        ratatui::crossterm::event::DisableMouseCapture,
    );
    ratatui::restore();
    result
}
