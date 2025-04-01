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
use std::cmp::Eq;
use std::collections::{HashMap, VecDeque};
use std::sync::{atomic, Arc, Mutex};

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

    fn para_execute(
        &self,
        cfg: &Cfg,
        num_worker: usize,
    ) -> Arc<Mutex<HashMap<usize, Self::OutFlowType>>>
    where
        Self: Sync,
        Self::InFlowType: Clone + Sync + Send,
        Self::OutFlowType: Clone + Eq + Sync + Send,
    {
        let worklist: Mutex<VecDeque<_>> = Mutex::new(cfg.nodes.iter().cloned().collect());
        // cast node_ptr to usize, which impls Send + Sync
        let out_states: Arc<Mutex<HashMap<usize, Self::OutFlowType>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let should_exit: atomic::AtomicBool = atomic::AtomicBool::new(false);
        enum WorkerState {
            Sleep,
            Working,
            Submitted,
            Idle,
        }
        let worker_states = Mutex::new(
            (0..num_worker)
                .map(|_| WorkerState::Sleep)
                .collect::<Vec<_>>(),
        );

        std::thread::scope(|s| {
            // master rountine
            s.spawn(|| loop {
                let mut slave_all_spin = true;
                for slave_state in worker_states.lock().unwrap().iter() {
                    if !matches!(slave_state, WorkerState::Idle) {
                        slave_all_spin = false;
                        break;
                    }
                }
                if slave_all_spin {
                    assert!(worklist.lock().unwrap().is_empty());
                    should_exit.store(true, atomic::Ordering::Relaxed);
                    break;
                }
            });

            for slave_id in 0..num_worker {
                let should_exit = &should_exit;
                let worklist = &worklist;
                let worker_states = &worker_states;
                let out_states = &out_states;

                s.spawn(move || loop {
                    let next_to_do = {
                        let mut worklist_lock = worklist.lock().unwrap();
                        if let Some(next_to_do) = worklist_lock.pop_front() {
                            worker_states.lock().unwrap()[slave_id] = WorkerState::Working;
                            next_to_do
                        } else if should_exit.load(atomic::Ordering::Relaxed) {
                            break;
                        } else {
                            worker_states.lock().unwrap()[slave_id] = WorkerState::Idle;
                            continue;
                        }
                    };

                    let next_ptr = Arc::as_ptr(&next_to_do) as usize;
                    let pred_out_flow: Vec<_> = {
                        Self::predecessors(&next_to_do)
                            .iter()
                            .filter_map(|item| {
                                out_states
                                    .lock()
                                    .unwrap()
                                    .get(&(Arc::as_ptr(item) as usize))
                                    .cloned()
                            })
                            .collect()
                    };

                    let in_flow = if !pred_out_flow.is_empty() {
                        Self::merge(pred_out_flow)
                    } else {
                        self.init_in_flow_state(&next_to_do)
                    };
                    let out_flow = Self::transfer(&next_to_do, in_flow);
                    let mut out_states_lock = out_states.lock().unwrap();
                    let updated = out_states_lock
                        .get(&next_ptr)
                        .is_none_or(|prev_state| !out_flow.eq(&prev_state));
                    if updated {
                        let successor = Self::successors(&next_to_do);
                        worklist.lock().unwrap().extend(successor);
                        worker_states.lock().unwrap()[slave_id] = WorkerState::Submitted;
                        out_states_lock
                            .entry(next_ptr)
                            .and_modify(|cur| {
                                if Self::montone_improve(cur, &out_flow) {
                                    *cur = out_flow.clone()
                                }
                            })
                            .or_insert(out_flow);
                    }
                });
            }
        });
        out_states
    }
}
