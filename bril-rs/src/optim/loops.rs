use crate::analyzer::{dom::*, scc::*};
use crate::bril::LabelOrInst;
use crate::cfg::prelude::*;
use crate::cfg::FuncCtx;
use crate::optim::dce::global::ReachingDefAnalysis;
use crate::optim::dflow::WorkListAlgo;
use crate::transform;

use std::collections::{HashMap, HashSet};
use std::default::Default;
use std::sync::{Arc, Mutex, Weak};

pub struct NaturalLoop<'a> {
    pub entry: NodeRef,
    pub comp: &'a CompRef,
    pub exits: Vec<NodeRef>,
}

pub fn loop_invariant_code_motion(cfg: Cfg) -> Cfg {
    let mut cfg = transform::ssa::cfg_into_ssa(cfg);
    let comps = find_sccs(&cfg);
    let natural_loops = find_natural_loops(&cfg, &comps);
    let reaching_def_ret = ReachingDefAnalysis(&cfg).execute(&cfg);

    for mut natural_loop in natural_loops {
        let live_in: HashSet<String> = natural_loop
            .entry_preds_outside_loop()
            .into_iter()
            .flat_map(|pred| reaching_def_ret.get(&Weak::as_ptr(&pred)).cloned().unwrap())
            .collect();
        let invariants = identify_loop_invariants(&natural_loop, &live_in);
        let entry_ptr = Arc::as_ptr(&natural_loop.entry);
        let entry_idx = cfg
            .nodes
            .iter()
            .position(|node| Arc::as_ptr(node) == entry_ptr)
            .unwrap();

        let mut deleted_instrs = vec![];
        // safe to remove all loop variants, this only holds on ssa
        for node in &natural_loop.comp.lock().unwrap().cfg_nodes {
            let mut node_lock = node.lock().unwrap();
            let instrs = node_lock.blk.instrs.clone();
            let (removed, kept): (Vec<_>, Vec<_>) = instrs.into_iter().partition(
                |inst| matches!(inst, LabelOrInst::Inst {dest: Some(dest), ..} if invariants.contains(dest.as_str())) 
            );
            node_lock.blk.instrs = kept;
            deleted_instrs.extend(removed);
        }

        if !deleted_instrs.is_empty() {
            let preheader_node = natural_loop.inject_preheader_node();
            // topo sort removed instrs
            preheader_node
                .lock()
                .unwrap()
                .blk
                .instrs
                .extend(topo_sort_instrs(&deleted_instrs));
            cfg.nodes.insert(entry_idx, preheader_node);
            eprintln!("{} inst moved", deleted_instrs.len());
        } else {
            eprintln!("no liom chance");
        }
    }
    transform::ssa::cfg_from_ssa(cfg)
}

pub fn find_natural_loops<'a>(cfg: &Cfg, comps: &'a Vec<CompRef>) -> Vec<NaturalLoop<'a>> {
    let dom_tree = DomTree::from_cfg(cfg);
    let mut loops = vec![];
    for comp in comps {
        let (mut entries, exits) = {
            let comp_lock = comp.lock().unwrap();
            (comp_lock.entries(), comp_lock.exits())
        };

        // natural loop should have one single entry block
        if entries.len() == 1 {
            let entry = entries.pop().unwrap();
            // check whether the component contains at least one backedge
            // all backedge should also point to dominator
            if validate_backedges(&entry, &comp.lock().unwrap(), &dom_tree) {
                let natural_loop = NaturalLoop { entry, comp, exits };
                loops.push(natural_loop);
            }
        }
    }
    loops
}

fn topo_sort_instrs(instrs: &[LabelOrInst]) -> Vec<LabelOrInst> {
    let dest_to_idx: HashMap<String, usize> = instrs
        .iter()
        .enumerate()
        .map(|(i, inst)| {
            if let LabelOrInst::Inst {
                dest: Some(dest), ..
            } = inst
            {
                (dest.clone(), i)
            } else {
                unreachable!()
            }
        })
        .collect();

    let mut dep_graph = vec![];
    for inst in instrs {
        let deps = if let LabelOrInst::Inst {
            args: Some(args), ..
        } = inst
        {
            // args not found instrs's dests are considered to be pre-defined
            args.iter()
                .filter_map(|arg| dest_to_idx.get(arg).copied())
                .collect()
        } else {
            vec![]
        };
        dep_graph.push(deps);
    }

    struct TopoAlgo<F: Fn(&LabelOrInst) -> Vec<LabelOrInst>> {
        buf: Vec<LabelOrInst>,
        find_dep: F,
        vis: HashSet<LabelOrInst>,
    }

    impl<F: Fn(&LabelOrInst) -> Vec<LabelOrInst>> TopoAlgo<F> {
        fn sort(&mut self, buf: &[LabelOrInst]) {
            for node in buf {
                if !self.vis.contains(node) {
                    self.dfs(node)
                }
            }
        }

        fn dfs(&mut self, cur: &LabelOrInst) {
            if self.vis.contains(cur) {
                return;
            }
            self.vis.insert(cur.clone());
            for dep in (self.find_dep)(cur) {
                self.dfs(&dep);
            }
            self.buf.push(cur.clone())
        }
    }
    let mut topo_algo = TopoAlgo {
        buf: vec![],
        vis: HashSet::new(),
        find_dep: |inst: &LabelOrInst| {
            if let LabelOrInst::Inst {
                dest: Some(dest), ..
            } = inst
            {
                let idx = dest_to_idx.get(dest.as_str()).unwrap();
                dep_graph[*idx]
                    .iter()
                    .map(|&dep_idx| instrs[dep_idx].clone())
                    .collect()
            } else {
                unreachable!()
            }
        },
    };
    topo_algo.sort(instrs);
    topo_algo.buf
}

fn identify_loop_invariants(
    natural_loop: &NaturalLoop<'_>,
    loop_live_in: &HashSet<String>,
) -> HashSet<String> {
    let loop_subcfg = natural_loop.isolate_subcfg();
    let mut loop_invariant_algo_ctx = LoopInvariantAnalysis {
        reaching_def: loop_live_in,
        natural_loop,
    };
    let ret = loop_invariant_algo_ctx.execute(&loop_subcfg.cfg);
    let invariants = LoopInvariantAnalysis::merge(
        natural_loop
            .entry
            .lock()
            .unwrap()
            .predecessors
            .iter()
            .map(|pred| ret.get(&Weak::as_ptr(pred)).unwrap().clone())
            .collect(),
    );
    loop_subcfg.restore_original_cfg();
    invariants
}

// fn meet_motion_condition(natural_loop: &NaturalLoop<'_>) -> bool {
//     todo!()
// }

fn validate_backedges(entry: &NodeRef, comp: &Component, dom_tree: &DomTree) -> bool {
    struct Visitor<'a> {
        comp: &'a Component,
        vis: HashSet<NodePtr>,
        stack: Vec<NodePtr>,
        backedges: Vec<(NodePtr, NodePtr)>,
    }

    impl<'a> Visitor<'a> {
        fn within_comp_dfs(&mut self, cur: &NodeRef) {
            let cur_ptr = Arc::as_ptr(cur);
            if !self.comp.contains(&cur_ptr) || self.vis.contains(&cur_ptr) {
                return;
            }
            self.vis.insert(cur_ptr);
            self.stack.push(cur_ptr);
            for succ in &cur.lock().unwrap().successors {
                let succ_ptr = Weak::as_ptr(succ);
                if self.stack.contains(&succ_ptr) {
                    self.backedges.push((cur_ptr, succ_ptr));
                }
                self.within_comp_dfs(&Weak::upgrade(succ).unwrap());
            }
            self.stack.pop();
        }
    }
    let mut visitor = Visitor {
        comp,
        vis: HashSet::new(),
        stack: vec![],
        backedges: vec![],
    };
    visitor.within_comp_dfs(entry);
    if visitor.backedges.is_empty() {
        false
    } else {
        let valid = visitor
            .backedges
            .into_iter()
            .all(|(src, dst)| dom_tree.is_dominator_of(dst, src));
        assert!(valid);
        valid
    }
}

impl<'a> NaturalLoop<'a> {
    fn entry_preds_outside_loop(&self) -> Vec<WeakNodeRef> {
        let entry_lock = self.entry.lock().unwrap();
        entry_lock
            .predecessors
            .iter()
            .filter(|pred| !self.comp.lock().unwrap().contains(&Weak::as_ptr(pred)))
            .cloned()
            .collect()
    }

    fn inject_preheader_node(&mut self) -> NodeRef {
        let entry_label = self.entry.lock().unwrap().label.clone().unwrap();
        let preheader_label = format!("{}.preheader", entry_label);

        let (header_preds, preheader_node) = {
            // excluding in-component backedge
            let entry_preds = self.entry_preds_outside_loop();

            let mut entry_lock = self.entry.lock().unwrap();

            // guaranteed to have label, cfg entry block needs to be assigned with a dummy label
            let preheader_node = CfgNode {
                label: Some(preheader_label.clone()),
                blk: BasicBlock {
                    label: Some(preheader_label.clone()),
                    instrs: vec![serde_json::from_str(&format!(
                        r#"{{
                        "label": "{preheader_label}"
                    }}"#
                    ))
                    .unwrap()],
                },
                successors: vec![Arc::downgrade(&self.entry)],
                predecessors: entry_preds.clone(),
            };
            let preheader_node = Arc::new(Mutex::new(preheader_node));
            entry_lock.predecessors = vec![Arc::downgrade(&preheader_node)];
            (entry_preds, preheader_node)
        };

        for header_pred in &header_preds {
            let header_pred = Weak::upgrade(header_pred).unwrap();
            let mut pred_lock = header_pred.lock().unwrap();
            if let Some(LabelOrInst::Inst {
                op,
                labels: Some(ref mut labels),
                ..
            }) = pred_lock.blk.instrs.last_mut()
            {
                if matches!(op.as_str(), "br" | "jmp") {
                    labels.iter_mut().for_each(|dest| {
                        if *dest == entry_label {
                            *dest = preheader_label.clone();
                        }
                    })
                }
            }
            pred_lock
                .successors
                .retain(|succ| Weak::as_ptr(succ) != Arc::as_ptr(&self.entry));
            pred_lock.successors.push(Arc::downgrade(&preheader_node));
        }
        preheader_node
    }

    fn isolate_subcfg(&self) -> IsolatedLoopCfg<'_> {
        let detached_entry_pred = {
            let mut entry_lock = self.entry.lock().unwrap();
            let (kept, detached): (Vec<_>, Vec<_>) = entry_lock
                .predecessors
                .clone()
                .into_iter()
                .partition(|pred| self.comp.lock().unwrap().contains(&Weak::as_ptr(pred)));
            entry_lock.predecessors = kept;
            detached
        };

        let mut detached_exits_succ = HashMap::new();
        for exit in &self.exits {
            let mut exit_lock = exit.lock().unwrap();
            let (kept, detached): (Vec<_>, Vec<_>) = exit_lock
                .successors
                .clone()
                .into_iter()
                .partition(|succ| self.comp.lock().unwrap().contains(&Weak::as_ptr(succ)));
            exit_lock.successors = kept;
            detached_exits_succ.insert(Arc::as_ptr(exit), detached);
        }
        let loop_cfg = Cfg {
            root: Arc::downgrade(&self.entry),
            nodes: self.comp.lock().unwrap().cfg_nodes.clone(),
            // dummy func ctx
            func_ctx: FuncCtx {
                name: "".to_string(),
                args: None,
                ty: None,
            },
        };
        IsolatedLoopCfg {
            cfg: loop_cfg,
            detached_entry_pred,
            detached_exits_succ,
            natural_loop: self,
        }
    }
}

struct IsolatedLoopCfg<'a> {
    cfg: Cfg,
    detached_entry_pred: Vec<WeakNodeRef>,
    detached_exits_succ: HashMap<NodePtr, Vec<WeakNodeRef>>,
    natural_loop: &'a NaturalLoop<'a>,
}

impl<'a> IsolatedLoopCfg<'a> {
    fn restore_original_cfg(self) {
        self.natural_loop
            .entry
            .lock()
            .unwrap()
            .predecessors
            .extend(self.detached_entry_pred);
        for exit in &self.natural_loop.exits {
            exit.lock().unwrap().successors.extend(
                self.detached_exits_succ
                    .get(&Arc::as_ptr(exit))
                    .unwrap()
                    .clone(),
            )
        }
    }
}

struct LoopInvariantAnalysis<'a> {
    reaching_def: &'a HashSet<String>,
    natural_loop: &'a NaturalLoop<'a>,
}

impl<'a> WorkListAlgo for LoopInvariantAnalysis<'a> {
    const FORWARD_PASS: bool = true;
    type InFlowType = HashSet<String>;
    type OutFlowType = HashSet<String>;
    fn init_in_flow_state(&self, node: &NodeRef) -> Self::InFlowType {
        if Arc::as_ptr(node) == Arc::as_ptr(&self.natural_loop.entry) {
            self.reaching_def.clone()
        } else {
            Default::default()
        }
    }
    fn merge(out_flow: Vec<Self::OutFlowType>) -> Self::InFlowType {
        out_flow.into_iter().flatten().collect()
    }
    fn transfer(node: &NodeRef, in_flow: Self::InFlowType) -> Self::OutFlowType {
        let mut out_flow = in_flow;
        for inst in &node.lock().unwrap().blk.instrs {
            if let LabelOrInst::Inst {
                op,
                args: Some(args),
                dest: Some(dest),
                ..
            } = inst
            {
                if matches!(op.as_str(), "add" | "sub" | "div" | "mul" | "id")
                    && args.iter().all(|arg| out_flow.contains(arg))
                {
                    out_flow.insert(dest.clone());
                }
            } else if let LabelOrInst::Inst {
                op,
                value: Some(_),
                dest: Some(dest),
                ..
            } = inst
            {
                if matches!(op.as_str(), "const") {
                    out_flow.insert(dest.clone());
                }
            }
        }
        out_flow
    }
}
