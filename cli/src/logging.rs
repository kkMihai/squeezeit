use indicatif::ProgressBar;
use squeezeit::log::{Fields, is_file_result};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

pub struct BarLayer {
    pub bar: ProgressBar,
    pub verbose: bool,
}

impl<S: Subscriber> Layer<S> for BarLayer {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let level = *event.metadata().level();
        let finished = is_file_result(event);
        if finished {
            self.bar.inc(1);
        }

        let advice = level == Level::WARN && !finished;
        if level == Level::ERROR || advice || (self.verbose && finished) {
            self.print(format!("  {}", Fields::of(event).render()));
        }
    }
}

impl BarLayer {
    fn print(&self, line: String) {
        if self.bar.is_hidden() {
            eprintln!("{line}");
        } else {
            self.bar.println(line);
        }
    }
}
