use crate::bril::LabelOrInst;
use crate::cfg::{BasicBlock, Cfg, NodePtr, WeakNodeRef};
use crate::optim::dflow::{worklist_algo, WorkListItem};
use std::cmp::{Eq, PartialEq};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Weak};

pub fn find_unused_variables_per_node(cfg: &Cfg) -> HashMap<NodePtr, HashSet<String>> {
    let liveness_nodes = cfg
        .nodes
        .iter()
        .map(|node| LivenessNode::from_weak_node_ref(&Arc::downgrade(node)));
    let ret = worklist_algo(liveness_nodes);
    let mut unused_var_per_node = HashMap::new();
    for liveness_node in ret.keys() {
        let in_flows: HashSet<String> = liveness_node
            .predecessors()
            .iter()
            .flat_map(|pred| ret.get(pred).unwrap().clone())
            .collect();
        let cfg_node = &liveness_node.node;
        let unused = {
            let mut live_on_exit = liveness_node.able_to_kill.clone();
            live_on_exit.retain(|var| !in_flows.contains(var));
            live_on_exit
        };
        unused_var_per_node.insert(Weak::as_ptr(cfg_node), unused);
    }
    unused_var_per_node
}

#[derive(Clone)]
struct LivenessNode {
    node: WeakNodeRef,
    live_on_entry: HashSet<String>,
    able_to_kill: HashSet<String>,
}

impl Hash for LivenessNode {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Weak::as_ptr(&self.node).hash(state);
    }
}

impl PartialEq for LivenessNode {
    fn eq(&self, other: &Self) -> bool {
        let self_ptr = Weak::as_ptr(&self.node);
        let other_ptr = Weak::as_ptr(&other.node);
        self_ptr.eq(&other_ptr)
    }
}
impl Eq for LivenessNode {}

impl WorkListItem for LivenessNode {
    type InFlowType = HashSet<String>;
    type OutFlowType = HashSet<String>;
    fn merge(&self, out_flows: Vec<Self::OutFlowType>) -> Self::InFlowType {
        out_flows.into_iter().flatten().collect()
    }

    fn transfer(&self, in_flow: Option<Self::InFlowType>) -> Self::OutFlowType {
        if let Some(mut in_flow) = in_flow {
            in_flow.retain(|var| !self.able_to_kill.contains(var));
            in_flow.extend(self.live_on_entry.clone());
            in_flow
        } else {
            self.live_on_entry.clone()
        }
    }

    fn successors(&self) -> Vec<Self> {
        let node = self.node.upgrade().unwrap();
        let node_lock = node.lock().unwrap();
        node_lock
            .predecessors
            .iter()
            .map(Self::from_weak_node_ref)
            .collect()
    }

    fn predecessors(&self) -> Vec<Self> {
        let node = self.node.upgrade().unwrap();
        let node_lock = node.lock().unwrap();
        node_lock
            .successors
            .iter()
            .map(Self::from_weak_node_ref)
            .collect()
    }
}

impl LivenessNode {
    fn from_weak_node_ref(from: &WeakNodeRef) -> Self {
        let node = from.upgrade().unwrap();
        let node_lock = node.lock().unwrap();
        let blk = &node_lock.blk;
        let (live_on_entry, able_to_kill) = (live_on_entry(blk), defs(blk));
        Self {
            node: Weak::clone(from),
            live_on_entry,
            able_to_kill,
        }
    }
}

fn live_on_entry(blk: &BasicBlock) -> HashSet<String> {
    let mut used = HashSet::new();
    for inst in blk.instrs.iter().rev() {
        if let LabelOrInst::Inst {
            dest: Some(dest), ..
        } = inst
        {
            let _ = used.remove(dest);
        }
        if let LabelOrInst::Inst {
            args: Some(args), ..
        } = inst
        {
            used.extend(args.clone());
        }
    }
    used
}

/// returns a set of all variables defined within the block but not used
/// this does not include variables that defined in uppersteam block
fn defs(blk: &BasicBlock) -> HashSet<String> {
    let mut def = HashSet::new();
    for inst in blk.instrs.iter().rev() {
        if let LabelOrInst::Inst {
            dest: Some(dest), ..
        } = inst
        {
            def.insert(dest.clone());
        }
    }
    def
}
