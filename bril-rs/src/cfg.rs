use crate::bril::{Function, LabelOrInst, Prog};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type NodeRef = Arc<Mutex<CfgNode>>;

/// maintains cfg for each function in input bril prog
pub struct ProgCfgs(pub Vec<(String, Cfg)>);

impl ProgCfgs {
    pub fn from_bril_prog(prog: &Prog) -> Self {
        let cfgs = prog
            .functions
            .iter()
            .map(|f| (f.name.clone(), Cfg::from_bril_func(f)))
            .collect();
        Self(cfgs)
    }

    pub fn port_graph_as_dot(&self) -> String {
        let subgraphs = self
            .0
            .iter()
            .enumerate()
            .map(|(i, (scope, cfg))| {
                dbg!(scope);
                format!(
                    r#"
            subgraph cluster_{} {{
                label = "@{}";
                labelloc = "t";
                labeljust = "l";
                style = "solid";
                fontcolor = "brown";
                {} 
            }} 
        "#,
                    i,
                    scope,
                    cfg.nodes_and_edges_in_dot(|i| format!("{}_{}", scope, i))
                )
            })
            .collect::<Vec<_>>();
        format!(
            "digraph G {{
            {} 
        }}",
            subgraphs.join("\n")
        )
    }
}

#[derive(Debug)]
pub struct Cfg(Vec<NodeRef>);

#[derive(Debug)]
struct CfgNode {
    label: Option<String>,
    blk: BasicBlock,
    successors: Vec<NodeRef>,
}

impl Cfg {
    #[inline]
    pub fn from_bril_func(func: &Function) -> Self {
        Self::from_basic_blks(&BasicBlock::build_from_func(func))
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

            let successors = match node_lock.blk.instrs.last() {
                Some(LabelOrInst::Inst { op, labels, .. }) => {
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
                // handle the case where a basic label may only contain one label no instr
                None => {
                    debug_assert!(node_lock.label.is_some());
                    if i < nodes.len() - 1 {
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

    pub fn port_graph_as_dot(&self) -> String {
        format!(
            "digraph CFG {{
                {} 
        }}",
            self.nodes_and_edges_in_dot(|i| i.to_string())
        )
    }

    /// output cfg in dot format
    fn nodes_and_edges_in_dot<F: Fn(usize) -> String>(&self, scoper: F) -> String {
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
                format!(
                    r#"{} [label = {}]"#,
                    scoper(i),
                    node.lock().unwrap().caption()
                )
            })
            .collect();

        let edge_desc: Vec<String> = edges
            .iter()
            .map(|(u, v)| format!("{} -> {}", scoper(*u), scoper(*v)))
            .collect();

        format!(
            "
                node [shape = box] \n\
                {} \n\
                {} \n\
            ",
            nodes_desc.join("\n"),
            edge_desc.join("\n")
        )
    }
}

impl CfgNode {
    /// little html codes for node content display
    fn caption(&self) -> String {
        let mut tags = vec![];
        if let Some(ref label) = self.label {
            tags.push(format!(
                r#"
                <tr><td align="left" valign="top"><b>.{}</b></td></tr>
            "#,
                label
            ));
        }
        // display first 2 instrs
        let instrs: Vec<_> = self
            .blk
            .instrs
            .iter()
            .take(2)
            .map(|inst| {
                if let LabelOrInst::Inst { op, .. } = inst {
                    op.clone()
                } else {
                    unreachable!()
                }
            })
            .collect();

        if !instrs.is_empty() {
            tags.push(format!(
                r#"
                    <tr><td align="CENTER" valign="MIDDLE">{}</td></tr>
                "#,
                instrs.join("<br/>")
            ))
        }

        format!(
            r#"<
        <table BORDER="0" CELLBORDER="0" CELLSPACING="0">
            {}
        </table>
    >"#,
            tags.join("")
        )
    }
}

#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub label: Option<String>,
    pub instrs: Vec<LabelOrInst>,
}

impl BasicBlock {
    pub fn build_from_func(func: &Function) -> Vec<BasicBlock> {
        let mut blks = vec![];
        let mut cur_blk = Self::new();
        for instr in &func.instrs {
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

        if !cur_blk.instrs.is_empty() || cur_blk.label.is_some() {
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
