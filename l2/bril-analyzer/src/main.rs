use bril_rs::{bril, cfg};
use clap::Parser;
use std::collections::HashMap;
use std::io::{BufReader, Read};

#[derive(Parser)]
struct Args {
    #[arg(short)]
    f: Option<String>,
    #[arg(long, action)]
    cfg: bool,
    #[arg(long, action)]
    op: bool,
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
    if args.op {
        let mut op_stats = count_ops(&bril_prog).into_iter().collect::<Vec<_>>();
        op_stats.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
        for (op, count) in op_stats {
            println!("{}: \t{}", op, count);
        }
    }
    if args.cfg {
        let dot = cfg::ProgCfgs::from_bril_prog(&bril_prog).port_graph_as_dot();
        println!("{}", dot);
    }
    Ok(())
}

/// counts the number of each op type
fn count_ops(prog: &bril::Prog) -> HashMap<String, usize> {
    let mut stats = HashMap::<String, usize>::new();
    for function in &prog.functions {
        for instr in &function.instrs {
            if let bril::LabelOrInst::Inst { ref op, .. } = instr {
                stats
                    .entry(op.clone())
                    .and_modify(|count| *count += 1)
                    .or_insert(1);
            }
        }
    }
    stats
}
