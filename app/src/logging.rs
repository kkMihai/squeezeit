use std::sync::Arc;

use crossbeam_queue::ArrayQueue;
use squeezeit::log::Fields;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

pub const LOG_CAPACITY: usize = 500;

pub struct Log {
    pub queue: Arc<ArrayQueue<(Level, String)>>,
}

impl Log {
    pub fn new() -> Self {
        Self {
            queue: Arc::new(ArrayQueue::new(LOG_CAPACITY)),
        }
    }
}

pub struct FeedLayer {
    pub queue: Arc<ArrayQueue<(Level, String)>>,
}

impl<S: Subscriber> Layer<S> for FeedLayer {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let level = *event.metadata().level();
        self.queue.force_push((level, Fields::of(event).render()));
    }
}
