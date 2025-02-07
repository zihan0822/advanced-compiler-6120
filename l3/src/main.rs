use bril_rs::bril::*;
use bril_rs::{
    bril,
    cfg::{self, BasicBlock},
    optim,
};

use clap::Parser;
use std::io::{BufReader, Read};

#[derive(Parser)]
struct Args {
    #[arg(short)]
    f: Option<String>,
}

fn main() -> std::io::Result<()> {
    let args = Args::parse();
    let mut reader: Box<dyn Read> = if let Some(ref f) = args.f {
        Box::new(BufReader::new(std::fs::File::open(f)?))
    } else {
        Box::new(BufReader::new(std::io::stdin()))
    };
    let mut buf = String::new();
    assert!(reader.read_to_string(&mut buf)? > 0);

    let bril_prog = bril::Prog::from_json(&buf).unwrap();
    let local_optimizer = optim::LocalOptimizerBuilder::new()
        .dce()
        .const_folding()
        .value_numbering()
        .finish();
    let prog = apply_blk_optim(bril_prog, |blk| local_optimizer.run_all(blk));
    println!("{:#}", serde_json::to_string(&prog).unwrap());
    Ok(())
}

fn apply_blk_optim<F>(bril_prog: Prog, optimizer: F) -> Prog
where
    F: Fn(BasicBlock) -> BasicBlock,
{
    let cfgs = cfg::ProgCfgs::from_bril_prog(&bril_prog);
    let mut functions = vec![];
    for (func_ctx, cfg) in cfgs.0.into_iter() {
        let optimized_instrs = cfg
            .nodes
            .iter()
            .flat_map(|node| {
                let blk = &node.lock().unwrap().blk;
                let optimized_blk = optimizer(blk.clone());
                optimized_blk.instrs.into_iter()
            })
            .collect();
        let func = Function {
            name: func_ctx.name,
            args: func_ctx.args,
            ty: func_ctx.ty,
            instrs: optimized_instrs,
        };
        functions.push(func);
    }
    Prog { functions }
}
