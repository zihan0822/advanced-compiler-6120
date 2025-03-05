#![allow(unused_imports)]
use bril_rs::analyzer;
use bril_rs::bril::*;
use bril_rs::optim::dce::global;
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
    let prog_cfgs = cfg::ProgCfgs::from_bril_prog(&bril_prog);
    for cfg in &prog_cfgs.0 {
        if let Err(msg) = analyzer::uninitialized_var_detection(cfg) {
            println!("@{}", cfg.func_ctx.name);
            println!("{}", msg);
        }
    }
    Ok(())
}
