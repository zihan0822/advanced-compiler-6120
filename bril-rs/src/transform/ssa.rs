use crate::analyzer::dom::{DomNodeRef, DomTree};
use crate::bril::LabelOrInst;
use crate::cfg::prelude::*;
use crate::optim::dce::global::LivenessAnalysis;
use crate::optim::dflow::WorkListAlgo;
use std::collections::{HashMap, HashSet};
use std::default::Default;
use std::sync::Arc;

pub fn cfg_into_ssa(cfg: Cfg) -> Cfg {
    let mut ssa_ctx = SSATransContext::new(&cfg);
    ssa_ctx.walk_cfg();
    cfg
}

struct SSATransContext<'a> {
    cfg: &'a Cfg,
    dom_tree: DomTree,
    cfg_ptr2dom_node: HashMap<NodePtr, DomNodeRef>,
    cfg_ptr2cfg_node: HashMap<NodePtr, NodeRef>,
    blk_cache: HashMap<NodePtr, PerBlockCache>,
}

#[derive(Default)]
struct PerBlockCache {
    live_in: HashSet<String>,
    renamed_live_out: HashMap<String, String>,
    liveness_vars: HashSet<String>,
    used_by_frontier: HashSet<String>,
    live_in_may_shadow: HashSet<String>,
}

impl<'a> SSATransContext<'a> {
    fn new(cfg: &'a Cfg) -> Self {
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
            dom_tree,
            cfg_ptr2dom_node,
            cfg_ptr2cfg_node,
            blk_cache: Default::default(),
        }
    }

    fn walk_cfg(&mut self) {
        self.update_liveness_cache();
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
        for node in &self.cfg.nodes {
            let cfg_ptr = Arc::as_ptr(node);
            let mut cfg_lock = node.lock().unwrap();
            let mut local_rename_ctx = BlockSSATransContext::from_label_and_blk(
                cfg_lock.label.clone().unwrap_or("entry".to_string()),
                &mut cfg_lock.blk,
            );
            let blk_cache = self.blk_cache.entry(cfg_ptr).or_default();
            local_rename_ctx.rename_local_vars();
            blk_cache.renamed_live_out = local_rename_ctx.live_out_renaming();
            blk_cache.live_in = cfg_lock.blk.used_but_not_defed();
        }
    }

    fn inspect_dom_frontier(&mut self) {
        for node in &self.cfg.nodes {
            let cfg_ptr = Arc::as_ptr(node);
            let dom_node = self.cfg_ptr2dom_node.get(&cfg_ptr).unwrap();
            let frontiers = DomTree::domination_frontier(Arc::clone(&dom_node));

            let filtered_live_out: HashSet<_> = self
                .blk_cache
                .get_mut(&cfg_ptr)
                .unwrap()
                .liveness_vars
                .clone();

            let mut used_by_frontier = HashSet::new();

            for frontier_ptr in frontiers {
                let cache = self.blk_cache.get_mut(&frontier_ptr).unwrap();
                for may_shadow in filtered_live_out.intersection(&cache.live_in) {
                    cache.live_in_may_shadow.insert(may_shadow.clone());
                    used_by_frontier.insert(may_shadow.clone());
                }
            }

            self.blk_cache
                .get_mut(&cfg_ptr)
                .map(|cache| cache.used_by_frontier = used_by_frontier);
        }
    }

    fn insert_set_and_get(&mut self) {
        fn recurse_on_dom_node(
            node: &DomNodeRef,
            mut non_ambiguious_var_renaming: HashMap<String, String>,
            per_blk_cache: &HashMap<NodePtr, PerBlockCache>,
        ) {
            let dom_node_lock = node.lock().unwrap();
            let cfg_node = &dom_node_lock.cfg_node;
            let cfg_ptr = Arc::as_ptr(cfg_node);
            let cache = per_blk_cache.get(&cfg_ptr).unwrap();
            let to_rename: HashSet<_> = cache
                .live_in
                .iter()
                .filter(|var| !cache.live_in_may_shadow.contains(var.as_str()))
                .cloned()
                .collect();
            let mut cfg_node_lock = cfg_node.lock().unwrap();
            for inst in cfg_node_lock.blk.instrs.iter_mut() {
                if let LabelOrInst::Inst {
                    args: Some(ref mut args),
                    ..
                } = inst
                {
                    args.iter_mut().for_each(|arg| {
                        if to_rename.contains(arg) {
                            let new_name = non_ambiguious_var_renaming.get(arg).unwrap().clone();
                            *arg = new_name;
                        }
                    })
                }
            }
            // insert set expr at the end of block before the last jmp/br inst if there is one
            let last_jmp_or_br = cfg_node_lock.blk.instrs.iter().position(
                |inst| matches!(inst, LabelOrInst::Inst {op, ..} if (op == "br") || (op == "jmp")),
            );
            let mut set_instrs = vec![];
            for to_set in &cache.used_by_frontier {
                let to_set_local_name = cache.renamed_live_out.get(to_set).unwrap();
                set_instrs.push(
                    serde_json::from_str(&format!(
                        r#"{{ 
                        "args" : ["{to_set}", "{to_set_local_name}"], 
                        "op": "set" 
                    }}"#
                    ))
                    .unwrap(),
                )
            }
            if let Some(last_jmp_or_br) = last_jmp_or_br {
                cfg_node_lock.blk.instrs.splice(last_jmp_or_br..last_jmp_or_br, set_instrs);
            } else {
                cfg_node_lock.blk.instrs.extend(set_instrs);
            }

            // insert get expr at the start of block
            // let mut get_exprs = vec![];
            // let cfg_label = cfg_node_lock.label.as_ref().unwrap_or("entry");
            // for to_get in &cache.live_in_may_shadow {
            //     todo!()
            // }

            non_ambiguious_var_renaming.extend(cache.renamed_live_out.clone().into_iter());
            for child_dom_node in &dom_node_lock.successors {
                recurse_on_dom_node(
                    &child_dom_node.upgrade().unwrap(),
                    non_ambiguious_var_renaming.clone(),
                    per_blk_cache,
                );
            }
        }
        recurse_on_dom_node(
            &self.dom_tree.root.upgrade().unwrap(),
            HashMap::new(),
            &self.blk_cache,
        );
    }
}

struct BlockSSATransContext<'a> {
    blk_label: String,
    blk: &'a mut BasicBlock,
    renaming_table: HashMap<String, usize>,
}

impl<'a> BlockSSATransContext<'a> {
    fn from_label_and_blk(blk_label: String, blk: &'a mut BasicBlock) -> Self {
        Self {
            blk_label,
            blk,
            renaming_table: HashMap::new(),
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
                    }
                });
            }
            if let LabelOrInst::Inst {
                dest: Some(ref mut dest),
                ..
            } = inst
            {
                let next_to_assign = self.renaming_table.entry(dest.clone()).or_insert(0);
                *dest = Self::mangle_scheme(&self.blk_label, &dest, *next_to_assign);
                *next_to_assign += 1;
            }
        }
    }

    /// returns the renaming of all live out variables defined within this block
    fn live_out_renaming(self) -> HashMap<String, String> {
        self.renaming_table
            .into_iter()
            .map(|(name, next_to_assign)| {
                let renamed = Self::mangle_scheme(&self.blk_label, &name, next_to_assign - 1);
                (name, renamed)
            })
            .collect()
    }

    #[inline]
    fn mangle_scheme(blk_label: &str, original_name: &str, idx: usize) -> String {
        format!("{blk_label}.{original_name}.{idx}")
    }
}
