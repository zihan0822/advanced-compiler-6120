use super::dflow::WorkListAlgo;
use crate::cfg::{Cfg, NodeRef};
use dashmap::DashMap;
use std::cmp::{self, Eq, Ord, PartialEq, PartialOrd};
use std::collections::VecDeque;
use std::sync::{atomic, Arc, Mutex};

pub trait ParaWorkListExt: WorkListAlgo {
    fn para_execute(&self, cfg: &Cfg) -> DashMap<usize, Self::OutFlowType>
    where
        Self: Sync,
        Self::InFlowType: Clone + Sync + Send,
        Self::OutFlowType: Clone + Eq + Sync + Send,
    {
        let worklist: Mutex<VecDeque<_>> = Mutex::new(cfg.nodes.iter().cloned().collect());
        // cast node_ptr to usize, which impls Send + Sync
        let out_states: DashMap<usize, Self::OutFlowType> = DashMap::new();
        let should_exit: atomic::AtomicBool = atomic::AtomicBool::new(false);
        #[derive(Debug)]
        enum WorkerState {
            Sleep,
            Working,
            Submitted,
            Idle,
        }
        let num_slave = crate::get_num_worklist_worker();
        let worker_states = Mutex::new(
            (0..num_slave)
                .map(|_| WorkerState::Sleep)
                .collect::<Vec<_>>(),
        );

        crate::get_thread_pool().scope(|s| {
            // master rountine
            s.spawn(|_| loop {
                let mut slave_all_spin = true;
                for slave_state in worker_states.lock().unwrap().iter() {
                    if !matches!(slave_state, WorkerState::Idle) {
                        slave_all_spin = false;
                        break;
                    }
                }
                if slave_all_spin {
                    let worklist_lock = worklist.lock().unwrap();
                    assert!(worklist_lock.is_empty());
                    should_exit.store(true, atomic::Ordering::Relaxed);
                    break;
                }
            });

            for slave_id in 0..num_slave {
                let should_exit = &should_exit;
                let worklist = &worklist;
                let worker_states = &worker_states;
                let out_states = &out_states;

                s.spawn(move |_| loop {
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
                                    .get(&(Arc::as_ptr(item) as usize))
                                    .map(|pred| pred.clone())
                            })
                            .collect()
                    };

                    let in_flow = if !pred_out_flow.is_empty() {
                        Self::merge(pred_out_flow)
                    } else {
                        self.init_in_flow_state(&next_to_do)
                    };
                    let out_flow = Self::transfer(&next_to_do, in_flow);
                    out_states
                        .entry(next_ptr)
                        .and_modify(|cur| {
                            if !out_flow.eq(cur) && Self::montone_improve(cur, &out_flow) {
                                *cur = out_flow.clone();
                                worklist
                                    .lock()
                                    .unwrap()
                                    .extend(Self::successors(&next_to_do));
                                worker_states.lock().unwrap()[slave_id] = WorkerState::Submitted;
                            }
                        })
                        .or_insert_with(|| {
                            worklist
                                .lock()
                                .unwrap()
                                .extend(Self::successors(&next_to_do));
                            worker_states.lock().unwrap()[slave_id] = WorkerState::Submitted;
                            out_flow
                        });
                });
            }
        });
        out_states
    }
}

impl<T: WorkListAlgo> ParaWorkListExt for T {}

#[allow(dead_code)]
struct WorkListItem(NodeRef);

impl PartialEq for WorkListItem {
    fn eq(&self, other: &Self) -> bool {
        Arc::as_ptr(&self.0) == Arc::as_ptr(&other.0)
    }
}
impl Eq for WorkListItem {}

impl PartialOrd for WorkListItem {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for WorkListItem {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        let num_suc_self = self.0.lock().unwrap().successors.len();
        let num_suc_other = other.0.lock().unwrap().successors.len();
        num_suc_self.cmp(&num_suc_other)
    }
}
