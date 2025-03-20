#![allow(unused_imports)]
use bril_rs::analyzer;
use bril_rs::analyzer::scc;
use bril_rs::bril::*;
use bril_rs::optim::dce::global;
use bril_rs::optim::loops;
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
    let mut prog_cfgs = cfg::ProgCfgs::from_bril_prog(&bril_prog);
    // for cfg in &prog_cfgs.0 {
    //     let sccs = scc::find_sccs(cfg);
    //     eprintln!("@{}", cfg.func_ctx.name);
    //     for comp in &sccs {
    //         let comp_lock = comp.lock().unwrap();
    //         eprintln!("comp size: {}", comp_lock.cfg_nodes.len());
    //     }
    //     let natural_loops = loops::find_natural_loops(cfg, &sccs);
    //     for natural_loop in &natural_loops {
    //         eprintln!(
    //             "natural loop size: {}",
    //             natural_loop.comp.lock().unwrap().cfg_nodes.len()
    //         );
    //     }
    // }
    // println!(
    //     "{:#}",
    //     serde_json::to_string(&prog_cfgs.into_bril_prog()).unwrap()
    // );

    let mut optim_cfgs = vec![];
    for cfg in prog_cfgs.0 {
        optim_cfgs.push(loops::loop_invariant_code_motion(cfg));
    }
    println!(
        "{:#}",
        serde_json::to_string(&bril_rs::cfg::ProgCfgs(optim_cfgs).into_bril_prog()).unwrap()
    );

    Ok(())
}

