use bril_rs::bril::*;
use bril_rs::{bril, cfg, optim};

use clap::Parser;
use std::io::{BufReader, Read};

#[derive(Parser)]
struct Args {
    #[arg(short)]
    f: Option<String>,
    #[arg(short = 'g', default_value_t = false)]
    with_global_ctx: bool,
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
    let prog = apply_cfg_optim(bril_prog, args.with_global_ctx);
    println!("{:#}", serde_json::to_string(&prog).unwrap());
    Ok(())
}

fn apply_cfg_optim(bril_prog: Prog, with_global_ctx: bool) -> Prog {
    let cfgs = cfg::ProgCfgs::from_bril_prog(&bril_prog);
    let mut functions = vec![];
    for (func_ctx, cfg) in cfgs.0.into_iter() {
        let optimized_instrs = optim::dce(cfg, func_ctx.clone(), with_global_ctx)
            .nodes
            .iter()
            .flat_map(|node| {
                let node_lock = node.lock().unwrap();
                node_lock.blk.instrs.clone().into_iter()
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
