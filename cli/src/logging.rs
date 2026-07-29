use indicatif::ProgressBar;
use squeezeit::log::Fields;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

pub struct BarLayer {
    pub bar: ProgressBar,
    pub verbose: bool,
}

impl<S: Subscriber> Layer<S> for BarLayer {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let fields = Fields::of(event);
        if fields.is_file_event() {
            self.bar.inc(1);
        }
        let failed = *event.metadata().level() == Level::ERROR;
        if failed || (self.verbose && fields.is_file_event()) {
            self.bar.println(format!("  {}", fields.render()));
        }
    }
}
