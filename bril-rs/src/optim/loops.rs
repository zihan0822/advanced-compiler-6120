use crate::analyzer::{dom::*, scc::*};
use crate::cfg::prelude::*;
use std::collections::HashSet;
use std::sync::{Arc, Weak};

pub struct NaturalLoop<'a> {
    pub entry: NodeRef,
    pub comp: &'a CompRef,
    pub exits: Vec<NodeRef>,
}

pub fn find_natural_loops<'a>(cfg: &Cfg, comps: &'a Vec<CompRef>) -> Vec<NaturalLoop<'a>> {
    let dom_tree = DomTree::from_cfg(cfg);
    let mut loops = vec![];
    for comp in comps {
        let comp_lock = comp.lock().unwrap();

        let mut entries = comp_lock.entries();
        // natural loop should have one single entry block
        if entries.len() == 1 {
            let entry = entries.pop().unwrap();
            assert!(validate_backedges(&entry, &comp_lock, &dom_tree));
            loops.push(NaturalLoop {
                entry,
                comp,
                exits: comp_lock.exits(),
            })
        }
    }
    loops
}

fn validate_backedges(entry: &NodeRef, comp: &Component, dom_tree: &DomTree) -> bool {
    struct Visitor<'a> {
        comp: &'a Component,
        vis: HashSet<NodePtr>,
        stack: Vec<NodePtr>,
        backedges: Vec<(NodePtr, NodePtr)>,
    }

    impl<'a> Visitor<'a> {
        fn within_comp_dfs(&mut self, cur: &NodeRef) {
            let cur_ptr = Arc::as_ptr(cur);
            if !self.comp.contains(&cur_ptr) || self.vis.contains(&cur_ptr) {
                return;
            }
            self.vis.insert(cur_ptr);
            self.stack.push(cur_ptr);
            for succ in &cur.lock().unwrap().successors {
                let succ_ptr = Weak::as_ptr(succ);
                if self.stack.contains(&succ_ptr) {
                    self.backedges.push((cur_ptr, succ_ptr));
                }
                self.within_comp_dfs(&Weak::upgrade(succ).unwrap());
            }
            self.stack.pop();
        }
    }
    let mut visitor = Visitor {
        comp,
        vis: HashSet::new(),
        stack: vec![],
        backedges: vec![],
    };
    visitor.within_comp_dfs(entry);
    visitor
        .backedges
        .into_iter()
        .all(|(src, dst)| is_dominator_of(dom_tree, dst, src))
}

fn is_dominator_of(dom_tree: &DomTree, a: NodePtr, b: NodePtr) -> bool {
    fn in_substree(dom_node: &DomNodeRef, query: NodePtr) -> bool {
        let dom_node_lock = dom_node.lock().unwrap();
        let cfg_ptr = Arc::as_ptr(&dom_node_lock.cfg_node);
        if cfg_ptr == query {
            return true;
        }
        for child in &dom_node_lock.successors {
            if in_substree(&child.upgrade().unwrap(), query) {
                return true;
            }
        }
        false
    }
    let starting_node = dom_tree
        .nodes
        .iter()
        .find(|dom_node| {
            let cfg_node = &dom_node.lock().unwrap().cfg_node;
            Arc::as_ptr(cfg_node) == a
        })
        .cloned()
        .unwrap();
    in_substree(&starting_node, b)
}
