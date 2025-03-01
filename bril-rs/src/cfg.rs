use crate::bril::{Arg, Function, LabelOrInst, Prog};
use crate::graphviz_prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, Weak};

pub type NodeRef = Arc<Mutex<CfgNode>>;
pub type WeakNodeRef = Weak<Mutex<CfgNode>>;
pub type NodePtr = *const Mutex<CfgNode>;

pub(crate) mod prelude {
    pub(crate) use super::{BasicBlock, Cfg, CfgNode, FuncCtx};
    pub(crate) use super::{NodePtr, NodeRef, WeakNodeRef};
}

/// maintains cfg for each function in input bril prog
pub struct ProgCfgs(pub Vec<(FuncCtx, Cfg)>);

#[derive(Clone)]
pub struct FuncCtx {
    pub name: String,
    pub args: Option<Vec<Arg>>,
    pub ty: Option<String>,
}

impl ProgCfgs {
    pub fn from_bril_prog(prog: &Prog) -> Self {
        let cfgs = prog
            .functions
            .iter()
            .map(|f| {
                let func_ctx = FuncCtx {
                    name: f.name.clone(),
                    args: f.args.clone(),
                    ty: f.ty.clone(),
                };
                (func_ctx, Cfg::from_bril_func(f))
            })
            .collect();
        Self(cfgs)
    }

    pub fn port_as_dot_string(&self) -> String {
        let g = self.port_as_dot();
        g.print(&mut PrinterContext::default())
    }

    pub(crate) fn port_as_dot(&self) -> Graph {
        let subgraphs: Vec<Subgraph> = self
            .0
            .iter()
            .enumerate()
            .map(|(i, (func_ctx, cfg))| {
                let scope = format!(r#""@{}""#, &func_ctx.name);
                let mut stmts = vec![
                    stmt!(attr!("label", scope)),
                    stmt!(attr!("labelloc", "t")),
                    stmt!(attr!("labeljust", "l")),
                    stmt!(attr!("style", "solid")),
                    stmt!(attr!("fontcolor", "brown")),
                ];
                if let Graph::DiGraph {
                    stmts: subgraph_stmts,
                    ..
                } = cfg.port_as_dot_with_scope(|i| format!("{}_{i}", &func_ctx.name))
                {
                    stmts.extend(subgraph_stmts)
                } else {
                    unreachable!()
                }
                Subgraph {
                    id: id!(format!("cluster_{}", i)),
                    stmts,
                }
            })
            .collect::<Vec<_>>();

        let mut g = graph!(di id!("G"));
        for subgraph in subgraphs {
            g.add_stmt(stmt!(subgraph));
        }
        g
    }
}

#[derive(Debug, Clone)]
pub struct Cfg {
    pub nodes: Vec<NodeRef>,
    pub root: WeakNodeRef,
}

#[derive(Debug)]
pub struct CfgNode {
    pub label: Option<String>,
    pub blk: BasicBlock,
    pub successors: Vec<WeakNodeRef>,
    pub predecessors: Vec<WeakNodeRef>,
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
                    predecessors: vec![],
                }))
            })
            .collect();

        let mut node_by_label = HashMap::<String, WeakNodeRef>::new();
        for node in &nodes {
            if let Some(label) = &node.lock().unwrap().label {
                node_by_label.insert(String::from(label), Arc::downgrade(node));
            }
        }

        for (i, node) in nodes.iter().enumerate() {
            let mut node_lock = node.lock().unwrap();

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
                        Some(vec![Arc::downgrade(&nodes[i + 1])])
                    } else {
                        None
                    }
                }
                // handle the case where a basic label may only contain one label no instr
                LabelOrInst::Label { .. } => {
                    debug_assert!(node_lock.label.is_some());
                    if i < nodes.len() - 1 {
                        Some(vec![Arc::downgrade(&nodes[i + 1])])
                    } else {
                        None
                    }
                }
            };

            // do this in two pass to accommondate borrow checker
            if let Some(successors) = successors {
                // update predecessor info
                for successor in &successors {
                    let successor = successor.upgrade().unwrap();
                    successor
                        .lock()
                        .unwrap()
                        .predecessors
                        .push(Arc::downgrade(node));
                }

                node_lock.successors.extend(successors);
            }
        }
        let root = Arc::downgrade(&nodes[0]);
        Self { nodes, root }
    }

    pub fn port_as_dot_string(&self) -> String {
        let g = self.port_as_dot();
        g.print(&mut PrinterContext::default())
    }

    pub(crate) fn port_as_dot(&self) -> Graph {
        let mut g = graph!(di id!("subrountine"));
        let (nodes, edges) = self.nodes_and_edges_in_dot(|i| i.to_string());
        for node in nodes.into_values() {
            g.add_stmt(stmt!(node));
        }
        for edge in edges {
            g.add_stmt(stmt!(edge))
        }
        g
    }

    pub(crate) fn port_as_dot_with_scope<F: Fn(usize) -> String>(&self, scoper: F) -> Graph {
        let mut g = graph!(di id!("subrountine"));
        let (nodes, edges) = self.nodes_and_edges_in_dot(scoper);
        for node in nodes.into_values() {
            g.add_stmt(stmt!(node));
        }
        for edge in edges {
            g.add_stmt(stmt!(edge))
        }
        g
    }

    /// output cfg in dot format
    pub(crate) fn nodes_and_edges_in_dot<F: Fn(usize) -> String>(
        &self,
        scoper: F,
    ) -> (HashMap<NodePtr, DotNode>, Vec<DotEdge>) {
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
                        self.dfs(Some(cur_idx), Weak::upgrade(child).unwrap());
                    }
                }
            }
        }

        // relabel nodes indexed from 0
        let relabeled_nodes: HashMap<NodePtr, usize> = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, node)| (Arc::as_ptr(node), i))
            .collect();

        let visitor = Visitor {
            relabeled_nodes,
            vis: vec![false; self.nodes.len()],
            first: self.root.upgrade().unwrap(),
            edges: vec![],
        };

        let edges = visitor.find_edges();
        let (mut node_stmts_map, mut edge_stmts) = (HashMap::new(), vec![]);

        for (i, node) in self.nodes.iter().enumerate() {
            let node_lock = node.lock().unwrap();
            let node_id = scoper(i);
            node_stmts_map.insert(
                Arc::as_ptr(node),
                node!(
                    node_id;
                    attr!("label", &node_lock.caption()),
                    attr!("shape", "box")
                ),
            );
        }
        for (u, v) in &edges {
            let (u_id, v_id) = (scoper(*u), scoper(*v));
            edge_stmts.push(edge!(node_id!(u_id) => node_id!(v_id)));
        }
        (node_stmts_map, edge_stmts)
    }
}

impl CfgNode {
    /// little html codes for node content display
    pub fn caption(&self) -> String {
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
            .filter_map(|inst| {
                if let LabelOrInst::Inst { op, .. } = inst {
                    Some(op.clone())
                } else {
                    None
                }
            })
            .take(2)
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
                        cur_blk.instrs.push(instr.clone());
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

    pub fn used_but_not_defed(&self) -> HashSet<String> {
        let mut used = HashSet::new();
        for inst in self.instrs.iter().rev() {
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
    pub fn defs(&self) -> HashSet<String> {
        let mut def = HashSet::new();
        for inst in self.instrs.iter().rev() {
            if let LabelOrInst::Inst {
                dest: Some(dest), ..
            } = inst
            {
                def.insert(dest.clone());
            }
        }
        def
    }

    fn new_with_label(label: String) -> Self {
        Self {
            label: Some(label.clone()),
            instrs: vec![LabelOrInst::Label { label }],
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
