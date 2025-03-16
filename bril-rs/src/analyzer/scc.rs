use crate::cfg::prelude::*;
use std::cmp::min;
use std::collections::{HashMap, HashSet};
use std::default::Default;
use std::sync::{Arc, Mutex, Weak};

type CompRef = Arc<Mutex<Component>>;
type WeakCompRef = Arc<Mutex<Component>>;

pub struct ReducedCfg {
    pub root: CompRef,
    pub comps: Vec<CompRef>,
}

pub struct Component {
    pub predecessors: Vec<WeakCompRef>,
    pub successors: Vec<CompRef>,
    pub cfg_nodes: Vec<NodeRef>,
}

pub fn find_sccs(cfg: &Cfg) -> Vec<Component> {
    #[derive(Default)]
    struct Visitor {
        vis: HashSet<NodePtr>,
        preorder: HashMap<NodePtr, usize>,
        lowest: HashMap<NodePtr, usize>,
        comps: Vec<Vec<NodePtr>>,
        stack: Vec<NodePtr>,
    }

    impl Visitor {
        fn preorder_pass(&mut self, cur: NodeRef, idx: &mut usize) {
            let node_ptr = Arc::as_ptr(&cur);
            if self.vis.contains(&node_ptr) {
                return;
            }
            self.preorder.insert(node_ptr, *idx);
            self.lowest.insert(node_ptr, *idx);
            self.vis.insert(node_ptr);
            for child in cur.lock().unwrap().successors.iter() {
                *idx += 1;
                self.preorder_pass(Weak::upgrade(child).unwrap(), idx)
            }
        }

        fn find_lowest_reachable(&mut self, cur: NodeRef) {
            let node_ptr = Arc::as_ptr(&cur);
            let cur_preorder = self.preorder.get(&node_ptr).copied().unwrap();
            let mut lowest = cur_preorder;
            self.vis.insert(node_ptr);
            self.stack.push(node_ptr);

            for child in cur.lock().unwrap().successors.iter() {
                let child_ptr = Weak::as_ptr(child);
                if self.stack.iter().any(|p| *p == child_ptr) {
                    lowest = min(lowest, self.preorder.get(&child_ptr).copied().unwrap());
                }
                if !self.vis.contains(&child_ptr) {
                    self.find_lowest_reachable(child.upgrade().unwrap());
                    lowest = min(lowest, self.lowest.get(&child_ptr).copied().unwrap());
                }
            }
            self.lowest.insert(node_ptr, lowest);
            if cur_preorder == lowest {
                let idx = self.stack.iter().rposition(|ptr| *ptr == node_ptr).unwrap();
                let comp = self.stack.split_off(idx);
                self.comps.push(comp);
            }
        }

        fn reset_vis(&mut self) {
            self.vis = HashSet::new();
        }
    }
    let mut visitor: Visitor = Default::default();
    let root_cfg_node = Weak::upgrade(&cfg.root).unwrap();
    visitor.preorder_pass(root_cfg_node.clone(), &mut 0);
    visitor.reset_vis();
    visitor.find_lowest_reachable(root_cfg_node.clone());
    let cfg_ptr2node: HashMap<_, _> = cfg
        .nodes
        .iter()
        .map(|node| (Arc::as_ptr(node), node.clone()))
        .collect();

    let comps = visitor
        .comps
        .into_iter()
        .map(|cfg_ptrs| Component {
            cfg_nodes: cfg_ptrs
                .iter()
                .map(|ptr| cfg_ptr2node.get(ptr).cloned().unwrap())
                .collect(),
            predecessors: vec![],
            successors: vec![],
        })
        .collect();
    comps
}
