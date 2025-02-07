mod dce;
pub use dce::dce;

use crate::cfg::BasicBlock;

type OptimScheme = Box<dyn Fn(BasicBlock) -> BasicBlock>;
pub struct LocalOptimizer(Vec<OptimScheme>);

impl LocalOptimizer {
    pub fn run_all(&self, mut blk: BasicBlock) -> BasicBlock {
        for scheme in self.0.iter() {
            blk = scheme(blk);
        }
        blk
    }
}

pub struct LocalOptimizerBuilder {
    value_numbering: bool,
    const_folding: bool,
}

impl Default for LocalOptimizerBuilder {
    fn default() -> Self {
        Self {
            value_numbering: true,
            const_folding: false,
        }
    }
}

impl LocalOptimizerBuilder {
    pub fn new() -> Self {
        Self {
            value_numbering: false,
            const_folding: false,
        }
    }

    pub fn value_numbering(mut self) -> Self {
        self.value_numbering = true;
        self
    }

    pub fn const_folding(mut self) -> Self {
        self.value_numbering = true;
        self.const_folding = true;
        self
    }

    pub fn finish(self) -> LocalOptimizer {
        let mut pipeline: Vec<OptimScheme> = vec![];
        if self.value_numbering {
            pipeline.push(Box::new(move |mut blk| {
                let dangling_playback = dce::conservative_var_renaming(&mut blk);
                if dangling_playback
                    .into_values()
                    .flat_map(|v| v.into_iter())
                    .collect::<Vec<_>>()
                    .is_empty()
                {
                    // no optimization performed if there is incoming variable carried over from ancestor basic block
                    dce::value_numbering(blk, self.const_folding)
                } else {
                    blk
                }
            }));
        }
        LocalOptimizer(pipeline)
    }
}
