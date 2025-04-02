//! generic worklist algorithm, operates on cfg
//!
//! The Algo
//! while worklist is not empty:
//!     b = pick any item from worklist
//!     in[b] = merge(out[p] for predecessor p of b)
//!     out[b] = transfer(b, in[b])
//!     if out[b] is updated:
//!         worklist += successors of b
use crate::cfg::{Cfg, NodePtr, NodeRef};
use std::cmp::{Eq, PartialEq};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

pub trait WorkListAlgo {
    const FORWARD_PASS: bool;
    type InFlowType;
    type OutFlowType;

    fn init_in_flow_state(&self, node: &NodeRef) -> Self::InFlowType;
    fn transfer(node: &NodeRef, in_flow: Self::InFlowType) -> Self::OutFlowType;
    fn merge(out_flow: Vec<Self::OutFlowType>) -> Self::InFlowType;

    fn successors(node: &NodeRef) -> Vec<NodeRef> {
        let node = &node.lock().unwrap();
        if Self::FORWARD_PASS {
            node.successors
                .iter()
                .map(|succ| succ.upgrade().unwrap())
                .collect()
        } else {
            node.predecessors
                .iter()
                .map(|pred| pred.upgrade().unwrap())
                .collect()
        }
    }
    fn predecessors(node: &NodeRef) -> Vec<NodeRef> {
        let node = &node.lock().unwrap();
        if Self::FORWARD_PASS {
            node.predecessors
                .iter()
                .map(|pred| pred.upgrade().unwrap())
                .collect()
        } else {
            node.successors
                .iter()
                .map(|succ| succ.upgrade().unwrap())
                .collect()
        }
    }

    fn montone_improve(_cur: &Self::OutFlowType, _next: &Self::OutFlowType) -> bool {
        true
    }

    fn execute(&self, cfg: &Cfg) -> HashMap<NodePtr, Self::OutFlowType>
    where
        Self::InFlowType: Clone,
        Self::OutFlowType: Clone + Eq,
    {
        let mut worklist: VecDeque<_> = cfg.nodes.iter().cloned().collect();
        let mut out_states: HashMap<NodePtr, Self::OutFlowType> = HashMap::new();

        while let Some(ref next_to_do) = worklist.pop_front() {
            let next_ptr = Arc::as_ptr(next_to_do);
            let pred_out_flow: Vec<_> = Self::predecessors(next_to_do)
                .iter()
                .filter_map(|item| out_states.get(&Arc::as_ptr(item)).cloned())
                .collect();

            let in_flow = if !pred_out_flow.is_empty() {
                Self::merge(pred_out_flow)
            } else {
                self.init_in_flow_state(next_to_do)
            };

            let out_flow = Self::transfer(next_to_do, in_flow);
            let updated = out_states
                .get(&next_ptr)
                .is_none_or(|prev_state| !out_flow.eq(prev_state));
            if updated {
                let successor = Self::successors(next_to_do);
                let _ = out_states.insert(next_ptr, out_flow);
                worklist.extend(successor);
            }
        }
        out_states
    }
}
