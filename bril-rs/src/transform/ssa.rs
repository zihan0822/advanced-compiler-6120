use crate::analyzer::dom::{DomNodeRef, DomTree};
use crate::bril::LabelOrInst;
use crate::cfg::prelude::*;
use crate::optim::dce::global::LivenessAnalysis;
use crate::optim::dflow::WorkListAlgo;
use std::collections::{HashMap, HashSet};
use std::default::Default;
use std::sync::{Arc, Weak};

pub fn cfg_into_ssa(cfg: Cfg, func_ctx: FuncCtx) -> Cfg {
    let mut ssa_ctx = SSATransContext::new(&cfg, &func_ctx);
    ssa_ctx.walk_cfg();
    cfg
}

struct SSATransContext<'a> {
    cfg: &'a Cfg,
    func_ctx: &'a FuncCtx,
    dom_tree: DomTree,
    cfg_ptr2dom_node: HashMap<NodePtr, DomNodeRef>,
    cfg_ptr2cfg_node: HashMap<NodePtr, NodeRef>,
    blk_cache: HashMap<NodePtr, PerBlockCache>,
}

#[derive(Default)]
struct PerBlockCache {
    // original-new -> new-name
    renamed_live_in: HashMap<String, String>,
    renamed_live_out: HashMap<String, String>,
    liveness_vars: HashSet<String>,
    live_in_ty: HashMap<String, String>,
    live_in_may_shadow: HashSet<String>,
}

impl<'a> SSATransContext<'a> {
    fn new(cfg: &'a Cfg, func_ctx: &'a FuncCtx) -> Self {
        let mut cfg_ptr2dom_node = HashMap::new();
        let mut cfg_ptr2cfg_node = HashMap::new();
        let dom_tree = DomTree::from_cfg(cfg);
        for dom_node in &dom_tree.nodes {
            let dom_node_lock = dom_node.lock().unwrap();
            let cfg_ptr = Arc::as_ptr(&dom_node_lock.cfg_node);
            cfg_ptr2cfg_node.insert(cfg_ptr, Arc::clone(&dom_node_lock.cfg_node));
            cfg_ptr2dom_node.insert(cfg_ptr, Arc::clone(&dom_node));
        }
        Self {
            cfg,
            func_ctx,
            dom_tree,
            cfg_ptr2dom_node,
            cfg_ptr2cfg_node,
            blk_cache: Default::default(),
        }
    }

    fn walk_cfg(&mut self) {
        self.per_blk_var_renaming();
        self.inspect_dom_frontier();
        self.insert_set_and_get();
    }

    fn update_liveness_cache(&mut self) {
        let liveness_ret = LivenessAnalysis.execute(&self.cfg);
        for node in &self.cfg.nodes {
            let in_flows: HashSet<String> = LivenessAnalysis::predecessors(node)
                .iter()
                .flat_map(|pred| liveness_ret.get(&Arc::as_ptr(pred)).unwrap().clone())
                .collect();
            let defs = node.lock().unwrap().blk.defs();
            self.blk_cache
                .entry(Arc::as_ptr(node))
                .or_default()
                .liveness_vars = in_flows.intersection(&defs).cloned().collect();
        }
    }

    fn per_blk_var_renaming(&mut self) {
        let func_args_and_ty = self.func_ctx.args.as_ref().map_or(HashMap::new(), |args| {
            args.iter()
                .map(|arg| (arg.name.clone(), arg.ty.clone()))
                .collect()
        });

        let mut live_in_type_algo = VarTypeAnalysis {
            root_ptr: Weak::as_ptr(&self.cfg.root),
            args_ty: func_args_and_ty,
        };

        let per_blk_live_in_types = live_in_type_algo.execute(&self.cfg);

        for node in &self.cfg.nodes {
            let cfg_ptr = Arc::as_ptr(node);
            let mut cfg_lock = node.lock().unwrap();

            let mut blk_live_in_types = VarTypeAnalysis::merge(
                cfg_lock
                    .predecessors
                    .iter()
                    .map(|pred| {
                        per_blk_live_in_types
                            .get(&Weak::as_ptr(&pred))
                            .unwrap()
                            .clone()
                    })
                    .collect(),
            );
            let mut local_rename_ctx = BlockSSATransContext::from_label_and_blk(
                cfg_lock.label.clone().unwrap_or("entry".to_string()),
                &mut cfg_lock.blk,
            );
            let blk_cache = self.blk_cache.entry(cfg_ptr).or_default();
            local_rename_ctx.rename_local_vars();
            let (renamed_live_in, renamed_live_out) = local_rename_ctx.live_in_and_out_renaming();
            blk_live_in_types.retain(|var, _| renamed_live_in.contains_key(var));
            blk_cache.live_in_ty = blk_live_in_types;
            blk_cache.renamed_live_in = renamed_live_in;
            blk_cache.renamed_live_out = renamed_live_out;
        }
    }

    fn inspect_dom_frontier(&mut self) {
        for node in &self.cfg.nodes {
            let cfg_ptr = Arc::as_ptr(node);
            let dom_node = self.cfg_ptr2dom_node.get(&cfg_ptr).unwrap();
            let frontiers = DomTree::domination_frontier(Arc::clone(&dom_node));

            let node_cache = self.blk_cache.get(&cfg_ptr).unwrap();
            let remote_live_out: HashSet<_> = node_cache.renamed_live_out.keys().cloned().collect();

            for frontier_ptr in frontiers {
                let cache = self.blk_cache.get_mut(&frontier_ptr).unwrap();
                let original_live_in: HashSet<_> = cache.renamed_live_in.keys().cloned().collect();

                for may_shadow in remote_live_out.intersection(&original_live_in) {
                    cache.live_in_may_shadow.insert(may_shadow.clone());
                }
            }
        }
    }

    fn insert_set_and_get(&mut self) {
        fn preorder_recurse_on_dom_node(
            node: &DomNodeRef,
            mut non_ambiguious_var_renaming: HashMap<String, String>,
            per_blk_cache: &HashMap<NodePtr, PerBlockCache>,
        ) {
            let dom_node_lock = node.lock().unwrap();
            let cfg_node = &dom_node_lock.cfg_node;
            let cfg_ptr = Arc::as_ptr(cfg_node);
            let cache = per_blk_cache.get(&cfg_ptr).unwrap();

            let mut to_rename: HashMap<_, _> = cache.renamed_live_in.clone();

            to_rename.iter_mut().for_each(|(var, new_name)| {
                if !cache.live_in_may_shadow.contains(var) {
                    let propgated_name = non_ambiguious_var_renaming.get(var).cloned().unwrap();
                    *new_name = propgated_name;
                }
            });

            let mut cfg_node_lock = cfg_node.lock().unwrap();
            for inst in cfg_node_lock.blk.instrs.iter_mut() {
                if let LabelOrInst::Inst {
                    args: Some(ref mut args),
                    ..
                } = inst
                {
                    args.iter_mut().for_each(|arg| {
                        if let Some(new_name) = to_rename.get(arg) {
                            *arg = new_name.clone();
                        }
                    })
                }
            }

            // insert get expr at the start of block
            let mut get_exprs = vec![];
            for to_get in &cache.live_in_may_shadow {
                let renamed = cache.renamed_live_in.get(to_get).unwrap();
                let ty = cache.live_in_ty.get(to_get).unwrap();
                get_exprs.push(
                    serde_json::from_str(&format!(
                        r#"{{
                        "dest": "{renamed}",
                        "op": "get",
                        "type": "{ty}" 
                    }}"#
                    ))
                    .unwrap(),
                );
            }
            let first_non_label_idx = cfg_node_lock
                .blk
                .instrs
                .iter()
                .position(|inst| !matches!(inst, LabelOrInst::Label { .. }))
                .unwrap_or(0);
            cfg_node_lock
                .blk
                .instrs
                .splice(first_non_label_idx..first_non_label_idx, get_exprs);

            let mut merged_renamed_live_out = cache.renamed_live_out.clone();
            for original_to_get_name in &cache.live_in_may_shadow {
                // if the live in we get is not shadowed
                if !merged_renamed_live_out.contains_key(original_to_get_name) {
                    let renamed_to_get = cache.renamed_live_in.get(original_to_get_name).unwrap();
                    merged_renamed_live_out
                        .insert(original_to_get_name.clone(), renamed_to_get.clone());
                }
            }
            non_ambiguious_var_renaming.extend(merged_renamed_live_out.into_iter());

            let mut set_instrs = vec![];
            for succ in &cfg_node_lock.successors {
                let succ_ptr = Weak::as_ptr(&succ);
                let succ_cache = per_blk_cache.get(&succ_ptr).unwrap();
                for to_set in &succ_cache.live_in_may_shadow {
                    let to_set_local_name = non_ambiguious_var_renaming.get(to_set).unwrap();
                    let remote_name = succ_cache.renamed_live_in.get(to_set).unwrap();
                    set_instrs.push(
                        serde_json::from_str(&format!(
                            r#"{{
                            "args" : ["{remote_name}", "{to_set_local_name}"],
                            "op": "set"
                        }}"#
                        ))
                        .unwrap(),
                    )
                }
            }
            // insert set expr at the end of block before the last jmp/br inst if there is one
            let last_jmp_or_br = cfg_node_lock.blk.instrs.iter().position(
                |inst| matches!(inst, LabelOrInst::Inst {op, ..} if (op == "br") || (op == "jmp")),
            );

            if let Some(last_jmp_or_br) = last_jmp_or_br {
                cfg_node_lock
                    .blk
                    .instrs
                    .splice(last_jmp_or_br..last_jmp_or_br, set_instrs);
            } else {
                cfg_node_lock.blk.instrs.extend(set_instrs);
            }

            for child_dom_node in &dom_node_lock.successors {
                preorder_recurse_on_dom_node(
                    &child_dom_node.upgrade().unwrap(),
                    non_ambiguious_var_renaming.clone(),
                    per_blk_cache,
                );
            }
        }
        let func_args: HashMap<_, _> = self.func_ctx.args.as_ref().map_or(HashMap::new(), |args| {
            args.iter()
                .map(|arg| (arg.name.clone(), arg.name.clone()))
                .collect()
        });
        preorder_recurse_on_dom_node(
            &self.dom_tree.root.upgrade().unwrap(),
            func_args,
            &self.blk_cache,
        );
    }
}

struct BlockSSATransContext<'a> {
    blk_label: String,
    blk: &'a mut BasicBlock,
    renaming_table: HashMap<String, usize>,
    renamed_live_in: HashMap<String, String>,
}

impl<'a> BlockSSATransContext<'a> {
    fn from_label_and_blk(blk_label: String, blk: &'a mut BasicBlock) -> Self {
        Self {
            blk_label,
            blk,
            renaming_table: HashMap::new(),
            renamed_live_in: HashMap::new(),
        }
    }

    fn rename_local_vars(&mut self) {
        for inst in &mut self.blk.instrs {
            if let LabelOrInst::Inst {
                args: Some(ref mut args),
                ..
            } = inst
            {
                args.iter_mut().for_each(|arg| {
                    if let Some(next_to_assign) = self.renaming_table.get(arg) {
                        *arg = Self::mangle_scheme(&self.blk_label, arg, *next_to_assign - 1);
                    } else {
                        // live in vars
                        let renamed = Self::mangle_scheme(&self.blk_label, &arg, 0);
                        self.renamed_live_in.insert(arg.clone(), renamed.clone());
                    }
                });
            }
            if let LabelOrInst::Inst {
                dest: Some(ref mut dest),
                ..
            } = inst
            {
                let next_to_assign = self.renaming_table.entry(dest.clone()).or_insert(
                    if self.renamed_live_in.contains_key(dest) {
                        1
                    } else {
                        0
                    },
                );
                *dest = Self::mangle_scheme(&self.blk_label, &dest, *next_to_assign);
                *next_to_assign += 1;
            }
        }
    }

    /// returns the renaming of all live out variables defined within this block
    fn live_in_and_out_renaming(self) -> (HashMap<String, String>, HashMap<String, String>) {
        let renamed_live_out = self
            .renaming_table
            .into_iter()
            .map(|(name, next_to_assign)| {
                let renamed = Self::mangle_scheme(&self.blk_label, &name, next_to_assign - 1);
                (name, renamed)
            })
            .collect();
        (self.renamed_live_in, renamed_live_out)
    }

    #[inline]
    fn mangle_scheme(blk_label: &str, original_name: &str, idx: usize) -> String {
        format!("{blk_label}.{original_name}.{idx}")
    }
}

struct VarTypeAnalysis {
    root_ptr: NodePtr,
    args_ty: HashMap<String, String>,
}

impl WorkListAlgo for VarTypeAnalysis {
    const FORWARD_PASS: bool = true;
    type InFlowType = HashMap<String, String>;
    type OutFlowType = HashMap<String, String>;

    fn transfer(&mut self, node: &NodeRef, in_flow: Option<Self::InFlowType>) -> Self::OutFlowType {
        let node_ptr = Arc::as_ptr(node);
        let mut in_flow = if node_ptr == self.root_ptr {
            self.args_ty.clone()
        } else {
            in_flow.unwrap_or_default()
        };
        for inst in &node.lock().unwrap().blk.instrs {
            if let LabelOrInst::Inst {
                dest: Some(dest),
                ty: Some(ty),
                ..
            } = inst
            {
                in_flow.insert(dest.clone(), ty.clone());
            }
        }
        in_flow
    }

    fn merge(mut out_flow: Vec<Self::OutFlowType>) -> Self::InFlowType {
        out_flow
            .into_iter()
            .reduce(|mut acc, next| {
                for (var, ty) in next.into_iter() {
                    if let Some(existing_ty) = acc.get(&var) {
                        assert_eq!(existing_ty, &ty);
                    } else {
                        acc.insert(var, ty);
                    }
                }
                acc
            })
            .unwrap_or_default()
    }
}
