use crate::cfg::{Cfg, NodePtr, NodeRef, ProgCfgs};
use crate::graphviz_prelude::*;
use crate::optim::dflow::WorkListAlgo;
use crate::optim::para_dflow::ParaWorkListExt;
use rand::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, Weak};

pub type DomNodeRef = Arc<Mutex<DomNode>>;
pub type WeakDomNodeRef = Weak<Mutex<DomNode>>;

pub fn draw_prog_with_dom_as_dot_string(prog: &ProgCfgs) -> String {
    let mut g = graph!(di id!("Prog"));
    for (i, cfg) in prog.0.iter().enumerate() {
        let mut rng = rand::rng();
        // randomly choose one node to compute its frontier
        let frontier_target = cfg.nodes[1..].choose(&mut rng);
        let func_graph = draw_cfg_with_dom(cfg, frontier_target);
        if let Graph::DiGraph { stmts, .. } = func_graph {
            g.add_stmt(stmt!(Subgraph {
                id: id!(format!("cluster_{i}")),
                stmts
            }));
        }
    }
    g.print(&mut PrinterContext::default())
}

pub fn draw_cfg_with_dom_as_dot_string(cfg: &Cfg) -> String {
    let mut rng = rand::rng();
    // randomly choose one node to compute its frontier
    let frontier_target = cfg.nodes[1..].choose(&mut rng);
    let g = draw_cfg_with_dom(cfg, frontier_target);
    g.print(&mut PrinterContext::default())
}

fn draw_cfg_with_dom(cfg: &Cfg, highlight_frontier_for: Option<&NodeRef>) -> Graph {
    let dom_tree = DomTree::from_cfg(cfg);
    let func_name = &cfg.func_ctx.name;
    let (mut dot_nodes_map, dot_edges) =
        cfg.nodes_and_edges_in_dot(|i| format!("{func_name}_cfg_{i}"));
    let cfg_ptr2dom_node: HashMap<_, _> = dom_tree
        .nodes
        .iter()
        .map(|dom_node| {
            let dom_node_lock = dom_node.lock().unwrap();
            let cfg_ptr = Arc::as_ptr(&dom_node_lock.cfg_node);
            (cfg_ptr, Arc::clone(dom_node))
        })
        .collect();
    if let Some(frontier_target) = highlight_frontier_for {
        let frontiers = DomTree::domination_frontier(
            cfg_ptr2dom_node
                .get(&Arc::as_ptr(frontier_target))
                .unwrap()
                .clone(),
        );
        // coloring the node for which we are looking for its dom frontier
        if !frontiers.is_empty() {
            let dot_node = dot_nodes_map
                .get_mut(&Arc::as_ptr(frontier_target))
                .unwrap();
            dot_node
                .attributes
                .extend(vec![attr!("fillcolor", "coral1"), attr!("style", "filled")]);
        }
        for cfg_ptr in &frontiers {
            let dot_node = dot_nodes_map.get_mut(cfg_ptr).unwrap();
            dot_node.attributes.extend(vec![
                attr!("fillcolor", "darkolivegreen2"),
                attr!("style", "filled"),
            ]);
        }
    }
    let mut g = graph!(di id!(func_name));
    let scope = format!(r#""@{}""#, &func_name);
    let mut g_stmts = vec![
        stmt!(attr!("label", scope)),
        stmt!(attr!("labelloc", "t")),
        stmt!(attr!("labeljust", "l")),
        stmt!(attr!("style", "solid")),
        stmt!(attr!("fontcolor", "brown")),
    ];
    if let Graph::DiGraph { mut stmts, .. } =
        dom_tree.port_as_dot_with_scope(|i| format!("{}_dom_{i}", &func_name))
    {
        stmts.extend(vec![
            stmt!(attr!("style", "solid")),
            stmt!(attr!("color", "blue")),
            stmt!(attr!("label", r#""DOM""#)),
        ]);
        g_stmts.push(stmt!(Subgraph {
            id: id!(format!("cluster_dom_{func_name}")),
            stmts
        }));
    } else {
        unreachable!()
    }
    let mut cfg_stmts = vec![];
    for dot_node in dot_nodes_map.into_values() {
        cfg_stmts.push(stmt!(dot_node));
    }
    for dot_edge in dot_edges {
        cfg_stmts.push(stmt!(dot_edge));
    }
    cfg_stmts.push(stmt!(attr!("label", "CFG")));
    g_stmts.push(stmt!(Subgraph {
        id: id!(format!("cluster_cfg_{func_name}")),
        stmts: cfg_stmts
    }));

    for stmt in g_stmts {
        g.add_stmt(stmt);
    }
    g
}

pub struct DomTree {
    pub root: WeakDomNodeRef,
    pub nodes: Vec<DomNodeRef>,
}

impl DomTree {
    pub fn from_cfg(cfg: &Cfg) -> Self {
        let build_ctx = DomTreeConstCtx::new(cfg);
        let ret = build_ctx.para_execute(cfg);
        let ptr2node: HashMap<_, _> = cfg
            .nodes
            .iter()
            .map(|node| {
                (
                    Arc::as_ptr(node),
                    Arc::new(Mutex::new(DomNode::from_cfg_node(node))),
                )
            })
            .collect();
        let dom_tree = Self {
            root: Arc::downgrade(ptr2node.get(&Weak::as_ptr(&cfg.root)).unwrap()),
            nodes: ptr2node.values().cloned().collect(),
        };
        let doms_per_node: Vec<_> = ret.iter().map(|kv| kv.value().clone()).collect();
        for doms in doms_per_node {
            let mut doms: Vec<_> = Vec::from_iter(doms);
            doms.sort_by_key(|ptr| ret.get(ptr).unwrap().len());
            let doms = doms
                .iter()
                .map(|ptr| Arc::downgrade(ptr2node.get(&(*ptr as *const _)).unwrap()))
                .collect();
            dom_tree.construct_path(doms);
        }
        dom_tree
    }

    pub fn port_as_dot_with_scope<F: Fn(usize) -> String>(&self, scoper: F) -> Graph {
        // no back edge
        struct Visitor {
            relabeled_nodes: HashMap<*const Mutex<DomNode>, usize>,
            edges: Vec<(usize, usize)>,
        }
        impl Visitor {
            fn dfs(&mut self, from: Option<usize>, cur: DomNodeRef) {
                let node_ptr = Arc::as_ptr(&cur);
                let cur_idx = *self.relabeled_nodes.get(&node_ptr).unwrap();
                if let Some(from) = from {
                    self.edges.push((from, cur_idx));
                }
                for child in &cur.lock().unwrap().successors {
                    self.dfs(Some(cur_idx), Weak::upgrade(child).unwrap());
                }
            }
        }

        let relabeled_nodes: HashMap<_, _> = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, node)| (Arc::as_ptr(node), i))
            .collect();

        let mut visitor = Visitor {
            relabeled_nodes,
            edges: vec![],
        };
        visitor.dfs(None, Weak::upgrade(&self.root).unwrap());

        let mut g = graph!(di id!("DOM"));
        for (i, dom_node) in self.nodes.iter().enumerate() {
            let dom_node_lock = dom_node.lock().unwrap();
            let caption = &dom_node_lock.cfg_node.lock().unwrap().caption();
            let node_id = scoper(i);
            g.add_stmt(stmt!(
                node!(node_id; attr!("label", caption), attr!("shape", "box"))
            ));
        }
        for (u, v) in visitor.edges {
            let (u_id, v_id) = (scoper(u), scoper(v));
            g.add_stmt(stmt!(edge!(node_id!(u_id) => node_id!(v_id))))
        }
        g
    }

    fn construct_path(&self, path: Vec<WeakDomNodeRef>) {
        let mut cur_node = Weak::upgrade(&self.root).unwrap();
        debug_assert_eq!(Arc::as_ptr(&cur_node), Weak::as_ptr(&self.root));
        for node in path.iter().skip(1) {
            {
                let mut cur_lock = cur_node.lock().unwrap();
                if !cur_lock
                    .successors
                    .iter()
                    .any(|suc| Weak::as_ptr(suc) == Weak::as_ptr(node))
                {
                    cur_lock.successors.push(node.clone());
                }
            }
            cur_node = Weak::upgrade(node).unwrap();
        }
    }

    #[inline]
    pub fn entry_frontier(&self) -> HashSet<NodePtr> {
        Self::domination_frontier(Weak::upgrade(&self.root).unwrap())
    }

    /// A's domination frontier contains B if A does not dominate B, but A dominates
    /// a predecessor of B (it's the finge in the CFG right after A's domination stops)
    pub fn domination_frontier(node: DomNodeRef) -> HashSet<NodePtr> {
        fn collect_subtree_nodes(cur: &DomNodeRef, nodes: &mut Vec<DomNodeRef>) {
            nodes.push(cur.clone());
            for child in &cur.lock().unwrap().successors {
                collect_subtree_nodes(&Weak::upgrade(child).unwrap(), nodes)
            }
        }

        let mut subtree_nodes = vec![];
        let mut frontier = HashSet::new();
        // including input node itself, every node is considered to be self-dominated
        collect_subtree_nodes(&node, &mut subtree_nodes);
        let target_cfg_ptr = {
            let dom_node_lock = node.lock().unwrap();
            Arc::as_ptr(&dom_node_lock.cfg_node)
        };
        let ptr2nodes: HashMap<_, _> = subtree_nodes
            .iter()
            .map(|dom_node| {
                let dom_node_lock = dom_node.lock().unwrap();
                let cfg_node_ptr = Arc::as_ptr(&dom_node_lock.cfg_node);
                (cfg_node_ptr, dom_node.clone())
            })
            .collect();

        for dominated in &subtree_nodes {
            let dom_node_lock = dominated.lock().unwrap();
            let cfg_node = &dom_node_lock.cfg_node;
            for frontier_candidate in &cfg_node.lock().unwrap().successors {
                let cfg_ptr = Weak::as_ptr(frontier_candidate);
                // a node's dom frontier can be itself
                if cfg_ptr == target_cfg_ptr || !ptr2nodes.contains_key(&cfg_ptr) {
                    frontier.insert(cfg_ptr);
                }
            }
        }

        frontier
    }

    pub fn is_dominator_of(&self, a: NodePtr, b: NodePtr) -> bool {
        fn in_substree(dom_node: &DomNodeRef, query: NodePtr) -> bool {
            let dom_node_lock = dom_node.lock().unwrap();
            let cfg_ptr = Arc::as_ptr(&dom_node_lock.cfg_node);
            if cfg_ptr == query {
                return true;
            }
            for child in &dom_node_lock.successors {
                if in_substree(&child.upgrade().unwrap(), query) {
                    return true;
                }
            }
            false
        }
        let starting_node = self
            .nodes
            .iter()
            .find(|dom_node| {
                let cfg_node = &dom_node.lock().unwrap().cfg_node;
                Arc::as_ptr(cfg_node) == a
            })
            .cloned()
            .unwrap();
        in_substree(&starting_node, b)
    }
}

pub struct DomNode {
    pub cfg_node: NodeRef,
    pub successors: Vec<WeakDomNodeRef>,
}

impl DomNode {
    fn from_cfg_node(cfg_node: &NodeRef) -> Self {
        Self {
            cfg_node: cfg_node.clone(),
            successors: vec![],
        }
    }
}

struct DomTreeConstCtx {
    root_ptr: usize,
    all_nodes: Vec<usize>,
}

impl WorkListAlgo for DomTreeConstCtx {
    const FORWARD_PASS: bool = true;
    type InFlowType = HashSet<usize>;
    type OutFlowType = HashSet<usize>;

    fn init_in_flow_state(&self, node: &NodeRef) -> Self::InFlowType {
        let node_ptr = Arc::as_ptr(node) as usize;
        if node_ptr == self.root_ptr {
            HashSet::new()
        } else {
            self.all_nodes.iter().cloned().collect()
        }
    }

    fn montone_improve(cur: &Self::OutFlowType, next: &Self::OutFlowType) -> bool {
        next.is_subset(cur)
    }

    fn transfer(node: &NodeRef, mut in_flow: Self::InFlowType) -> Self::OutFlowType {
        in_flow.insert(Arc::as_ptr(node) as usize);
        in_flow
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
        let root_ptr = Weak::as_ptr(&cfg.root) as usize;
        let all_nodes = cfg
            .nodes
            .iter()
            .map(|node| Arc::as_ptr(node) as usize)
            .collect();
        Self {
            root_ptr,
            all_nodes,
        }
    }
}
