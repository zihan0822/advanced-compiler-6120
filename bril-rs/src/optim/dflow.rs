//! generic worklist algorithm, operates on cfg
//!
//! The Algo
//! while worklist is not empty:
//!     b = pick any item from worklist
//!     in[b] = merge(out[p] for predecessor p of b)
//!     out[b] = transfer(b, in[b])
//!     if out[b] is updated:
//!         worklist += successors of b
use std::cmp::Eq;
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

pub trait WorkListItem: Sized {
    type InFlowType;
    type OutFlowType;

    fn transfer(&self, in_flow: Option<Self::InFlowType>) -> Self::OutFlowType;
    fn merge(&self, out_flow: Vec<Self::OutFlowType>) -> Self::InFlowType;
    fn successors(&self) -> Vec<Self>;
    fn predecessors(&self) -> Vec<Self>;
}

pub fn worklist_algo<T>(worklist: impl Iterator<Item = T>) -> HashMap<T, T::OutFlowType>
where
    T: WorkListItem + Hash + Eq + Clone,
    T::OutFlowType: Clone + Eq,
{
    let mut worklist: VecDeque<_> = worklist.collect();
    let mut out_states: HashMap<T, T::OutFlowType> = HashMap::new();
    while let Some(next_to_do) = worklist.pop_front() {
        let in_flow = {
            let pred_out_flow: Vec<_> = next_to_do
                .predecessors()
                .into_iter()
                .filter_map(|item| out_states.get(&item).cloned())
                .collect();
            if !pred_out_flow.is_empty() {
                Some(next_to_do.merge(pred_out_flow))
            } else {
                None
            }
        };

        let out_flow = next_to_do.transfer(in_flow);
        let updated = out_states
            .get(&next_to_do)
            .map_or(true, |prev_state| !out_flow.eq(prev_state));
        if updated {
            let successor = next_to_do.successors();
            let _ = out_states.insert(next_to_do, out_flow);
            worklist.extend(successor);
        }
    }
    out_states
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Hash, Eq, PartialEq, Clone, Debug)]
    struct TestNode(usize);

    impl WorkListItem for TestNode {
        type InFlowType = usize;
        type OutFlowType = usize;

        fn transfer(&self, in_flow: Option<usize>) -> usize {
            match in_flow {
                None => self.0,
                Some(val) => {
                    if val >= 3 {
                        100
                    } else {
                        val + 1
                    }
                }
            }
        }

        fn merge(&self, out_flows: Vec<usize>) -> usize {
            assert!(out_flows.len() == 1);
            out_flows[0]
        }

        fn successor(&self) -> Vec<Self> {
            vec![match self.0 {
                0 => Self(1),
                1 => Self(2),
                2 => Self(0),
                _ => unreachable!(),
            }]
        }

        fn predecessor(&self) -> Vec<Self> {
            vec![match self.0 {
                0 => Self(2),
                1 => Self(0),
                2 => Self(1),
                _ => unreachable!(),
            }]
        }
    }

    #[test]
    fn cycle() {
        let nodes = (0..3).map(|n| TestNode(n));
        let ret = worklist_algo(nodes);
        assert!(ret.into_values().eq(std::iter::repeat(100).take(3)));
    }
}
