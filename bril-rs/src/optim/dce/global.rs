use crate::bril::LabelOrInst;
use crate::cfg::{BasicBlock, Cfg, NodePtr, NodeRef};
use crate::optim::dflow::WorkListAlgo;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};

pub fn find_unused_variables_per_node(cfg: &Cfg) -> HashMap<NodePtr, HashSet<String>> {
    let ret = LivenessAnalysis.execute(cfg);
    let mut unused_var_per_node = HashMap::new();
    for node in &cfg.nodes {
        let in_flows: HashSet<String> = LivenessAnalysis::predecessors(node)
            .iter()
            .flat_map(|pred| ret.get(&Arc::as_ptr(pred)).unwrap().clone())
            .collect();
        let unused = {
            let mut defed = defs(&node.lock().unwrap().blk);
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

    fn merge(out_flows: Vec<Self::OutFlowType>) -> Self::InFlowType {
        out_flows.into_iter().flatten().collect()
    }

    fn transfer(&mut self, node: &NodeRef, in_flow: Option<Self::InFlowType>) -> Self::OutFlowType {
        let node_lock = node.lock().unwrap();
        let blk = &node_lock.blk;

        let (able_to_kill, used_but_not_defed) = (defs(blk), used_but_not_defed(blk));
        if let Some(mut in_flow) = in_flow {
            in_flow.retain(|var| !able_to_kill.contains(var));
            in_flow.extend(used_but_not_defed);
            in_flow
        } else {
            used_but_not_defed
        }
    }
}

pub struct ReachingDefAnalysis<'a>(pub &'a Cfg);

impl<'a> WorkListAlgo for ReachingDefAnalysis<'a> {
    const FORWARD_PASS: bool = true;
    type InFlowType = HashSet<String>;
    type OutFlowType = HashSet<String>;

    fn merge(out_flows: Vec<Self::OutFlowType>) -> Self::InFlowType {
        out_flows.into_iter().flatten().collect()
    }
    fn transfer(&mut self, node: &NodeRef, in_flow: Option<Self::InFlowType>) -> Self::OutFlowType {
        let mut out_flow = in_flow.unwrap_or_else(|| {
            if Arc::as_ptr(node) == Weak::as_ptr(&self.0.root) {
                Self::InFlowType::from_iter(
                    self.0.func_ctx.args_name().unwrap_or_default().into_iter(),
                )
            } else {
                HashSet::new()
            }
        });
        out_flow.extend(node.lock().unwrap().blk.defs());
        out_flow
    }
}

pub(crate) fn used_but_not_defed(blk: &BasicBlock) -> HashSet<String> {
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
pub(crate) fn defs(blk: &BasicBlock) -> HashSet<String> {
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
