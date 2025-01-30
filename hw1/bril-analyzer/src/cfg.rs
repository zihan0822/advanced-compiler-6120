use crate::bril::{LabelOrInst, Prog};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type NodeRef = Arc<Mutex<CfgNode>>;

#[derive(Debug)]
pub struct Cfg(Vec<NodeRef>);

#[derive(Debug)]
pub struct CfgNode {
    label: Option<String>,
    blk: BasicBlock,
    successors: Vec<NodeRef>,
}

impl Cfg {
    #[inline]
    pub fn from_bril_prog(prog: &Prog) -> Self {
        Self::from_basic_blks(&BasicBlock::walk_prog(prog))
    }

    pub fn from_basic_blks(blks: &[BasicBlock]) -> Self {
        let nodes: Vec<_> = blks
            .iter()
            .map(|blk| {
                Arc::new(Mutex::new(CfgNode {
                    label: blk.label.clone(),
                    blk: blk.clone(),
                    successors: vec![],
                }))
            })
            .collect();

        let mut node_by_label = HashMap::<String, NodeRef>::new();
        for node in &nodes {
            if let Some(label) = &node.lock().unwrap().label {
                node_by_label.insert(String::from(label), Arc::clone(node));
            }
        }

        for (i, node) in nodes.iter().enumerate() {
            let mut node_lock = node.lock().unwrap();

            dbg!(node_lock.blk.instrs.len());
            debug_assert!(!node_lock.blk.instrs.is_empty());
            let successors = match node_lock.blk.instrs.last().unwrap() {
                LabelOrInst::Inst { op, labels, .. } => {
                    if is_terminator_op(op) {
                        Some(
                            labels
                                .as_ref()
                                .unwrap()
                                .iter()
                                .map(|label| node_by_label.get(label).cloned().unwrap())
                                .collect::<Vec<_>>(),
                        )
                    } else if i < nodes.len() - 1 {
                        // if non-terminator, we try to execute the following block
                        Some(vec![nodes[i + 1].clone()])
                    } else {
                        None
                    }
                }
                _ => None,
            };

            // do this in two pass to accommondate borrow checker
            if let Some(successors) = successors {
                node_lock.successors.extend(successors);
            }
        }

        Self(nodes)
    }

    /// output cfg in dot format
    pub fn port_graph_as_dot(&self) -> String {
        type NodePtr = *const Mutex<CfgNode>;

        struct Visitor {
            first: NodeRef,
            vis: Vec<bool>,
            relabeled_nodes: HashMap<NodePtr, usize>,
            edges: Vec<(usize, usize)>,
        }
        impl Visitor {
            fn find_edges(mut self) -> Vec<(usize, usize)> {
                // the first node, a.k.a the first basic block is indexed as 0
                self.dfs(None, self.first.clone());
                self.edges
            }

            fn dfs(&mut self, from: Option<usize>, cur: NodeRef) {
                let cur_idx = *self.relabeled_nodes.get(&Arc::as_ptr(&cur)).unwrap();
                if let Some(from) = from {
                    self.edges.push((from, cur_idx));
                }
                if !self.vis[cur_idx] {
                    self.vis[cur_idx] = true;
                    for child in &cur.lock().unwrap().successors {
                        self.dfs(Some(cur_idx), Arc::clone(child));
                    }
                }
            }
        }

        // relabel nodes indexed from 0
        let relabeled_nodes: HashMap<NodePtr, usize> = self
            .0
            .iter()
            .enumerate()
            .map(|(i, node)| (Arc::as_ptr(node), i))
            .collect();

        let visitor = Visitor {
            relabeled_nodes,
            vis: vec![false; self.0.len()],
            first: self.0[0].clone(),
            edges: vec![],
        };

        let edges = visitor.find_edges();
        let nodes_desc: Vec<String> = self
            .0
            .iter()
            .enumerate()
            .map(|(i, node)| {
                let node_lock = &node.lock().unwrap();
                // use first two inst/label as caption of a given node
                let mut captions = vec![];
                let mut num_instr = 2;
                if let Some(ref label) = node_lock.label {
                    captions.push(format!(".{}", label));
                    num_instr = 1;
                }
                let instrs = node_lock
                    .blk
                    .instrs
                    .iter()
                    .take(num_instr)
                    .filter_map(|inst| {
                        if let LabelOrInst::Inst { op, .. } = inst {
                            Some(op.to_string())
                        } else {
                            None
                        }
                    });
                captions.extend(instrs);
                format! {r#"{} [label = "{}"]"#, i, captions.join("; ")}
            })
            .collect();

        let edge_desc: Vec<String> = edges
            .iter()
            .map(|(u, v)| format!("{} -> {}", u, v))
            .collect();

        format!(
            "digraph CFG {{ \n\
                node [shape = box] \n\
                {} \n\
                {} \n\
            }}",
            nodes_desc.join("\n"),
            edge_desc.join("\n")
        )
    }
}

#[derive(Debug, Clone)]
pub struct BasicBlock {
    label: Option<String>,
    instrs: Vec<LabelOrInst>,
}

impl BasicBlock {
    pub fn walk_prog(prog: &Prog) -> Vec<BasicBlock> {
        let mut blks = vec![];
        let mut cur_blk = Self::new();
        for instr in prog.functions.iter().flat_map(|f| f.instrs.iter()) {
            match instr {
                LabelOrInst::Label { label } => {
                    if cur_blk.label.is_none() && cur_blk.instrs.is_empty() {
                        cur_blk.label = Some(label.clone());
                    } else {
                        blks.push(std::mem::replace(
                            &mut cur_blk,
                            Self::new_with_label(label.clone()),
                        ));
                    }
                }
                LabelOrInst::Inst { op, .. } if is_terminator_op(op) => {
                    cur_blk.instrs.push(instr.clone());
                    blks.push(std::mem::replace(&mut cur_blk, Self::new()));
                }
                _ => cur_blk.instrs.push(instr.clone()),
            }
        }
        if !cur_blk.instrs.is_empty() {
            blks.push(cur_blk);
        }
        blks
    }

    fn new_with_label(label: String) -> Self {
        Self {
            label: Some(label),
            instrs: vec![],
        }
    }

    fn new() -> Self {
        Self {
            label: None,
            instrs: vec![],
        }
    }
}

fn is_terminator_op(op: &str) -> bool {
    matches!(op, "br" | "jmp")
}
