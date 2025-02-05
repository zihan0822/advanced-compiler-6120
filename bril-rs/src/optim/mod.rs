mod dce;
use dce::{dce, value_numbering};

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
    dce: bool,
    value_numbering: bool,
    const_folding: bool,
}

impl Default for LocalOptimizerBuilder {
    fn default() -> Self {
        Self {
            dce: true,
            value_numbering: true,
            const_folding: false,
        }
    }
}

impl LocalOptimizerBuilder {
    pub fn new() -> Self {
        Self {
            dce: false,
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

    pub fn dce(mut self) -> Self {
        self.dce = true;
        self
    }

    pub fn finish(self) -> LocalOptimizer {
        let mut pipeline: Vec<OptimScheme> = vec![];
        if self.value_numbering {
            pipeline.push(Box::new(move |mut blk| {
                dce::conservative_var_renaming(&mut blk);
                value_numbering(blk, self.const_folding)
            }));
        }
        if self.dce {
            pipeline.push(Box::new(dce));
        }
        LocalOptimizer(pipeline)
    }
}
