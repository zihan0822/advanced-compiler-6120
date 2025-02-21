pub mod analyzer;
pub mod bril;
pub mod cfg;
pub mod optim;

mod graphviz_prelude {
    pub use dot_generator::*;
    pub use dot_structures::*;
    pub use dot_structures::{Edge as DotEdge, Node as DotNode};
    pub use graphviz_rust::printer::{DotPrinter, PrinterContext};
}
