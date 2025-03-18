use crate::cfg::prelude::*;
use std::cmp::min;
use std::collections::{HashMap, HashSet};
use std::default::Default;
use std::sync::{Arc, Mutex, Weak};

pub type CompRef = Arc<Mutex<Component>>;
pub type WeakCompRef = Weak<Mutex<Component>>;

#[derive(Default)]
pub struct Component {
    pub predecessors: Vec<WeakCompRef>,
    pub successors: Vec<CompRef>,
    pub cfg_nodes: Vec<NodeRef>,
}

impl Component {
    pub fn size(&self) -> usize {
        self.cfg_nodes.len()
    }
    pub fn contains(&self, query: &NodePtr) -> bool {
        self.cfg_nodes
            .iter()
            .any(|node| Arc::as_ptr(node) == *query)
    }

    pub fn entries(&self) -> Vec<NodeRef> {
        let cfg_ptr_set = self.cfg_ptr_set();

        let mut entries = vec![];
        for node in &self.cfg_nodes {
            let node_lock = node.lock().unwrap();
            let outside_connects: Vec<_> = node_lock
                .predecessors
                .iter()
                .filter(|pred| !cfg_ptr_set.contains(&Weak::as_ptr(pred)))
                .collect();
            if !outside_connects.is_empty() {
                entries.push(Arc::clone(node));
            }
        }
        entries
    }

    pub fn exits(&self) -> Vec<NodeRef> {
        let cfg_ptr_set: HashSet<_> = self.cfg_ptr_set();

        let mut exits = vec![];
        for node in &self.cfg_nodes {
            let node_lock = node.lock().unwrap();
            let outside_connects: Vec<_> = node_lock
                .successors
                .iter()
                .filter(|succ| !cfg_ptr_set.contains(&Weak::as_ptr(succ)))
                .collect();
            if !outside_connects.is_empty() {
                exits.push(Arc::clone(node));
            }
        }
        exits
    }

    fn cfg_ptr_set(&self) -> HashSet<NodePtr> {
        self.cfg_nodes.iter().map(Arc::as_ptr).collect()
    }
}

pub fn find_sccs(cfg: &Cfg) -> Vec<CompRef> {
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
                if self.stack.contains(&child_ptr) {
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

    let comps: Vec<_> = visitor
        .comps
        .into_iter()
        .map(|cfg_ptrs| {
            Arc::new(Mutex::new(Component {
                cfg_nodes: cfg_ptrs
                    .iter()
                    .map(|ptr| cfg_ptr2node.get(ptr).cloned().unwrap())
                    .collect(),
                ..Default::default()
            }))
        })
        .collect();

    let comp_ptr2comp: HashMap<_, _> = comps
        .iter()
        .map(|comp| (Arc::as_ptr(comp), Arc::clone(comp)))
        .collect();

    let cfg_ptr2comp_ptr: HashMap<_, _> = comps
        .iter()
        .flat_map(|comp| {
            let cfg_ptr_set: Vec<_> = comp
                .lock()
                .unwrap()
                .cfg_nodes
                .iter()
                .map(|node| (Arc::as_ptr(node), Arc::as_ptr(comp)))
                .collect();
            cfg_ptr_set
        })
        .collect();

    for comp in &comps {
        let weak_comp = Arc::downgrade(comp);
        let mut comp_lock = comp.lock().unwrap();
        let comp_child_ptr: HashSet<_> = comp_lock
            .cfg_nodes
            .iter()
            .flat_map(|node| {
                node.lock()
                    .unwrap()
                    .successors
                    .iter()
                    .map(|succ| {
                        let succ_ptr = Weak::as_ptr(succ);
                        cfg_ptr2comp_ptr.get(&succ_ptr).cloned().unwrap()
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        let comp_child: Vec<_> = comp_child_ptr
            .iter()
            .filter_map(|ptr| {
                let child_comp = comp_ptr2comp.get(ptr).cloned().unwrap();
                // exclude self-reference
                if Arc::as_ptr(&child_comp) != Arc::as_ptr(comp) {
                    Some(child_comp)
                } else {
                    None
                }
            })
            .collect();
        for child in &comp_child {
            // exclude self-reference
            child.lock().unwrap().predecessors.push(weak_comp.clone());
        }
        comp_lock.successors = comp_child;
    }
    comps
}
