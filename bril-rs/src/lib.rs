use std::sync::OnceLock;
pub mod analyzer;
pub mod bril;
pub mod cfg;
pub mod optim;
pub mod transform;

static NUM_WORKLIST_WORKER: OnceLock<usize> = OnceLock::new();
static WORKER_POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();

pub(crate) fn get_num_worklist_worker() -> usize {
    *NUM_WORKLIST_WORKER.get_or_init(|| {
        std::env::var("NUM_WORKLIST_WORKER")
            .map_or(4, |num_worker| str::parse::<usize>(&num_worker).unwrap())
    })
}

pub(crate) fn get_thread_pool() -> &'static rayon::ThreadPool {
    WORKER_POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(get_num_worklist_worker() + 1)
            .build()
            .unwrap()
    })
}

mod graphviz_prelude {
    pub use dot_generator::*;
    pub use dot_structures::*;
    pub use dot_structures::{Edge as DotEdge, Node as DotNode};
    pub use graphviz_rust::printer::{DotPrinter, PrinterContext};
}
