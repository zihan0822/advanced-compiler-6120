use crate::cfg::{Cfg, NodePtr, NodeRef};
use crate::optim::dflow::WorkListAlgo;
use std::collections::HashSet;
use std::sync::{Weak, Arc};

pub struct DomTree {
    pub root: Weak<DomNode>,
    pub nodes: Vec<Arc<DomNode>>
}
pub struct DomNode {
    pub cfg_node: NodeRef,
    pub successors: Vec<Weak<Self>>,
    pub predecessors: Vec<Weak<Self>>
}

impl DomTree {
    pub fn from_cfg(cfg: &Cfg) {
        let mut build_ctx = DomGraphTreeCtx::new(cfg);
        let ret = build_ctx.execute(cfg);
        todo!()
    }
}


struct DomGraphConstCtx {
    root_ptr: NodePtr,
    all_nodes: Vec<NodePtr>,
}

impl WorkListAlgo for DomTreeConstCtx {
    const FORWARD_PASS: bool = true;
    type InFlowType = HashSet<NodePtr>;
    type OutFlowType = HashSet<NodePtr>;

    fn transfer(&mut self, node: &NodeRef, mut in_flow: Option<Self::InFlowType>) -> Self::OutFlowType {
        let node_ptr = Arc::as_ptr(node);
        if node_ptr == self.root_ptr {
            HashSet::from([self.root_ptr])
        } else {
            if let Some(mut in_flow) = in_flow {
                in_flow.insert(node_ptr);
                in_flow
            } else {
                self.all_nodes.iter().cloned().collect()
            }
        }
    }

    fn merge(out_flow: Vec<Self::OutFlowType>) -> Self::InFlowType {
        out_flow
            .into_iter()
            .reduce(|a, b| a.intersection(&b).cloned().collect())
            .unwrap()
    }
}

impl DomTreeConstCtx {
    fn new(cfg: &Cfg) -> Self {
        let root_ptr = Weak::as_ptr(&cfg.root);
        let all_nodes = cfg.nodes.iter().map(Arc::as_ptr).collect();
        Self {
            root_ptr,
            all_nodes
        }
    }
}
