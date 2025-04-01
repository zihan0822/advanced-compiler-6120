use crate::cfg::{Cfg, NodePtr, NodeRef};
use crate::optim::dflow::WorkListAlgo;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};

pub fn find_unused_variables_per_node(cfg: &Cfg) -> HashMap<NodePtr, HashSet<String>> {
    let ret = LivenessAnalysis.para_execute(cfg, crate::NUM_WORKLIST_WORKER);
    let mut unused_var_per_node = HashMap::new();
    let ret_lock = ret.lock().unwrap();
    for node in &cfg.nodes {
        let in_flows: HashSet<String> = LivenessAnalysis::predecessors(node)
            .iter()
            .flat_map(|pred| ret_lock.get(&(Arc::as_ptr(pred) as usize)).unwrap().clone())
            .collect();
        let unused = {
            let mut defed = node.lock().unwrap().blk.defs();
            defed.retain(|var| !in_flows.contains(var));
            defed
        };
        unused_var_per_node.insert(Arc::as_ptr(node), unused);
    }
    unused_var_per_node
}

pub struct LivenessAnalysis;

impl WorkListAlgo for LivenessAnalysis {
    const FORWARD_PASS: bool = false;
    type InFlowType = HashSet<String>;
    type OutFlowType = HashSet<String>;

    fn init_in_flow_state(&self, _: &NodeRef) -> Self::InFlowType {
        HashSet::new()
    }

    fn merge(out_flows: Vec<Self::OutFlowType>) -> Self::InFlowType {
        out_flows.into_iter().flatten().collect()
    }

    fn transfer(node: &NodeRef, mut in_flow: Self::InFlowType) -> Self::OutFlowType {
        let node_lock = node.lock().unwrap();
        let blk = &node_lock.blk;

        let (able_to_kill, used_but_not_defed) = (blk.defs(), blk.used_but_not_defed());
        in_flow.retain(|var| !able_to_kill.contains(var));
        in_flow.extend(used_but_not_defed);
        in_flow
    }
}

pub struct ReachingDefAnalysis<'a>(pub &'a Cfg);

impl<'a> WorkListAlgo for ReachingDefAnalysis<'a> {
    const FORWARD_PASS: bool = true;
    type InFlowType = HashSet<String>;
    type OutFlowType = HashSet<String>;

    fn init_in_flow_state(&self, node: &NodeRef) -> Self::InFlowType {
        if Arc::as_ptr(node) == Weak::as_ptr(&self.0.root) {
            Self::InFlowType::from_iter(self.0.func_ctx.args_name().unwrap_or_default())
        } else {
            HashSet::new()
        }
    }
    fn merge(out_flows: Vec<Self::OutFlowType>) -> Self::InFlowType {
        out_flows.into_iter().flatten().collect()
    }
    fn transfer(node: &NodeRef, in_flow: Self::InFlowType) -> Self::OutFlowType {
        let mut out_flow = in_flow;
        out_flow.extend(node.lock().unwrap().blk.defs());
        out_flow
    }
}
