#![allow(unused_imports)]
use bril_rs::analyzer::dom::*;
use bril_rs::bril::*;
use bril_rs::optim::dce::global;
use bril_rs::{
    bril,
    cfg::{self, BasicBlock, Cfg, NodePtr, NodeRef},
    optim,
};
use clap::Parser;
use std::collections::{HashMap, HashSet};
use std::io::{BufReader, Read};
use std::sync::{Arc, Weak};

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
    for (_, cfg) in &prog_cfgs.0 {
        check_dom_tree_impl(cfg);
    }
    let dot_string = draw_prog_with_dom_as_dot_string(&prog_cfgs);
    println!("{}", dot_string);
    Ok(())
}

fn check_dom_tree_impl(cfg: &Cfg) {
    let dom_tree = DomTree::from_cfg(cfg);
    let dom_set_per_cfg = collect_dom_set_per_cfg(&dom_tree);
    struct Visitor {
        vis: HashSet<NodePtr>,
        dom_set_per_cfg: HashMap<NodePtr, HashSet<NodePtr>>,
        exact_dom_per_cfg: HashMap<NodePtr, HashSet<NodePtr>>,
    }
    impl Visitor {
        fn check_dom_set(&mut self, cfg: &Cfg) {
            let root = Weak::upgrade(&cfg.root).unwrap();
            self.dfs(&root, &mut HashSet::new());
        }
        fn dfs(&mut self, cur: &NodeRef, path: &mut HashSet<NodePtr>) {
            let cur_ptr = Arc::as_ptr(cur);
            if self.vis.contains(&cur_ptr) {
                return;
            }
            self.vis.insert(cur_ptr);
            path.insert(cur_ptr);
            let dom_set_for_cur = self.dom_set_per_cfg.get(&cur_ptr).unwrap();
            if let std::collections::hash_map::Entry::Vacant(e) =
                self.exact_dom_per_cfg.entry(cur_ptr)
            {
                e.insert(path.clone());
            } else {
                let cur_est = self.exact_dom_per_cfg.get_mut(&cur_ptr).unwrap();
                *cur_est = cur_est.intersection(path).cloned().collect();
            }

            assert!(dom_set_for_cur.is_subset(path));
            for child in &cur.lock().unwrap().successors {
                self.dfs(&Weak::upgrade(child).unwrap(), path);
            }
            self.vis.remove(&cur_ptr);
            path.remove(&cur_ptr);
        }
    }
    let mut visitor = Visitor {
        vis: HashSet::new(),
        dom_set_per_cfg,
        exact_dom_per_cfg: HashMap::new(),
    };
    visitor.check_dom_set(cfg);
    assert_eq!(visitor.dom_set_per_cfg, visitor.exact_dom_per_cfg);
}

fn collect_dom_set_per_cfg(dom_tree: &DomTree) -> HashMap<NodePtr, HashSet<NodePtr>> {
    fn recurse_on_dom_node(
        node: &DomNodeRef,
        path: &mut HashSet<NodePtr>,
        collection: &mut HashMap<NodePtr, HashSet<NodePtr>>,
    ) {
        let dom_node_lock = node.lock().unwrap();
        let cfg_ptr = Arc::as_ptr(&dom_node_lock.cfg_node);
        path.insert(cfg_ptr);
        // including current node itself
        collection.insert(cfg_ptr, path.clone());
        for child in &dom_node_lock.successors {
            recurse_on_dom_node(&Weak::upgrade(child).unwrap(), path, collection);
        }
        path.remove(&cfg_ptr);
    }
    let mut dom_set_per_cfg = HashMap::new();
    recurse_on_dom_node(
        &Weak::upgrade(&dom_tree.root).unwrap(),
        &mut HashSet::new(),
        &mut dom_set_per_cfg,
    );
    dom_set_per_cfg
}
