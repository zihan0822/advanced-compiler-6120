use bril_rs::bril::*;
use bril_rs::transform::ssa;
use bril_rs::{bril, cfg};

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
    let prog = apply_cfg_optim(bril_prog);
    
    println!("{:#}", serde_json::to_string(&prog).unwrap());
    Ok(())
}

fn apply_cfg_optim(bril_prog: Prog) -> Prog {
    let cfgs = cfg::ProgCfgs::from_bril_prog(&bril_prog);
    let mut functions = vec![];
    for cfg in cfgs.0.into_iter() {
        let cfg = ssa::cfg_from_ssa(cfg);
        functions.push(cfg.into_bril_func());
    }
    Prog { functions }
}
