use crate::bril::LabelOrInst;
use crate::cfg::prelude::*;
use crate::optim::dflow::WorkListAlgo;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::default::Default;
use std::sync::{Arc, Mutex, Weak};

pub fn cfg_into_ssa(mut cfg: Cfg) -> Cfg {
    if let Some(dummy_entry_blk) = require_dummy_entry_blk(&cfg) {
        let old_root_cfg_node = Weak::clone(&cfg.root);
        let new_root_cfg_node = Arc::new(Mutex::new(CfgNode {
            label: None,
            blk: dummy_entry_blk,
            successors: vec![old_root_cfg_node.clone()],
            predecessors: vec![],
        }));
        old_root_cfg_node
            .upgrade()
            .unwrap()
            .lock()
            .unwrap()
            .predecessors
            .push(Arc::downgrade(&new_root_cfg_node));
        cfg.root = Arc::downgrade(&new_root_cfg_node);
        cfg.nodes.insert(0, new_root_cfg_node);
    }

    let mut ssa_ctx = SSATransContext::new(&cfg);
    ssa_ctx.walk_cfg();
    cfg
}

pub fn cfg_from_ssa(cfg: Cfg) -> Cfg {
    let mut to_get_tys = HashMap::new();
    for node in &cfg.nodes {
        for inst in &node.lock().unwrap().blk.instrs {
            if let LabelOrInst::Inst {
                op,
                dest: Some(dest),
                ty: Some(ty),
                ..
            } = inst
            {
                if op == "get" {
                    assert!(to_get_tys.insert(dest.clone(), ty.clone()).is_none());
                }
            }
        }
    }

    for node in &cfg.nodes {
        let mut node_lock = node.lock().unwrap();
        let instrs = &mut node_lock.blk.instrs;
        // delete all get instr
        instrs.retain(|inst| !matches!(inst, LabelOrInst::Inst {op, ..} if op == "get"));
        instrs.iter_mut().for_each(|inst| {
            if let LabelOrInst::Inst {
                op,
                args: Some(args),
                dest,
                ty,
                ..
            } = inst
            {
                if op == "set" {
                    *op = "id".to_string();
                    let (to_set, canonical_repr) = (args[0].clone(), args[1].clone());
                    *ty = Some(to_get_tys.get(&to_set).unwrap().clone());
                    *dest = Some(to_set);
                    *args = vec![canonical_repr];
                }
            }
        })
    }
    cfg
}

fn require_dummy_entry_blk(cfg: &Cfg) -> Option<BasicBlock> {
    let root_node = cfg.root.upgrade().unwrap();
    let root_node_lock = root_node.lock().unwrap();
    if root_node_lock.label.is_some() && cfg.func_ctx.args.is_some() {
        let mut instrs = vec![];
        for arg in cfg.func_ctx.args.as_ref().unwrap() {
            instrs.push(
                serde_json::from_str(&format!(
                    r#"{{
                        "dest": "{0}",
                        "args": ["{0}"],
                        "op": "id",
                        "type": "{1}" 
                    }}"#,
                    &arg.name, &arg.ty
                ))
                .unwrap(),
            );
        }
        Some(BasicBlock {
            label: None,
            instrs,
        })
    } else {
        None
    }
}

struct SSATransContext<'a> {
    cfg: &'a Cfg,
    blk_cache: HashMap<NodePtr, PerBlockCache>,
}

#[derive(Default)]
struct PerBlockCache {
    // original-new -> new-name
    renamed_live_in: HashMap<String, String>,
    renamed_live_out: HashMap<String, String>,
    reach_def: HashMap<String, HashSet<NodePtr>>,
    live_in_ty: HashMap<String, String>,
    live_in_may_shadow: HashSet<String>,
}

impl<'a> SSATransContext<'a> {
    fn new(cfg: &'a Cfg) -> Self {
        Self {
            cfg,
            blk_cache: Default::default(),
        }
    }

    fn walk_cfg(&mut self) {
        // add dummy entry block if the current entry block can be the target of jump
        self.update_reach_def_cache();
        self.per_blk_var_renaming();
        self.insert_set_and_get();
    }

    fn update_reach_def_cache(&mut self) {
        let func_args_and_ty = self
            .cfg
            .func_ctx
            .args
            .as_ref()
            .map_or(HashMap::new(), |args| {
                args.iter()
                    .map(|arg| (arg.name.clone(), arg.ty.clone()))
                    .collect()
            });
        let reach_def_ctx = ReachDefWithLabelProp {
            root_ptr: Weak::as_ptr(&self.cfg.root),
            args_ty: func_args_and_ty,
        };
        let reach_def_ret = reach_def_ctx.execute(self.cfg);

        for node in &self.cfg.nodes {
            let node_ptr = Arc::as_ptr(node);
            let node_lock = node.lock().unwrap();
            let cache = self.blk_cache.entry(node_ptr).or_default();
            let pred_reach_defs = node_lock
                .predecessors
                .iter()
                .map(|pred| reach_def_ret.get(&Weak::as_ptr(pred)).unwrap());

            cache.reach_def = pred_reach_defs.fold(
                HashMap::<String, HashSet<NodePtr>>::new(),
                |mut acc, next| {
                    for (var, from) in next.clone() {
                        acc.entry(var).or_default().extend(from);
                    }
                    acc
                },
            );
            cache.live_in_may_shadow = cache
                .reach_def
                .iter()
                .filter_map(|(var, remote_list)| {
                    if remote_list.len() > 1 {
                        Some(var.clone())
                    } else {
                        None
                    }
                })
                .collect();
        }
    }

    fn per_blk_var_renaming(&mut self) {
        let func_args_and_ty = self
            .cfg
            .func_ctx
            .args
            .as_ref()
            .map_or(HashMap::new(), |args| {
                args.iter()
                    .map(|arg| (arg.name.clone(), arg.ty.clone()))
                    .collect()
            });

        let live_in_type_algo = VarTypeAnalysis {
            root_ptr: Weak::as_ptr(&self.cfg.root),
            args_ty: func_args_and_ty,
        };

        let per_blk_live_in_types = live_in_type_algo.execute(self.cfg);

        for node in &self.cfg.nodes {
            let cfg_ptr = Arc::as_ptr(node);
            let mut cfg_lock = node.lock().unwrap();

            let blk_live_in_types = VarTypeAnalysis::merge(
                cfg_lock
                    .predecessors
                    .iter()
                    .map(|pred| {
                        per_blk_live_in_types
                            .get(&Weak::as_ptr(pred))
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
            blk_cache.live_in_ty = blk_live_in_types;
            blk_cache.renamed_live_in = renamed_live_in;
            blk_cache.renamed_live_out = renamed_live_out;
        }
    }

    fn insert_set_and_get(&mut self) {
        // first pass update renamed-live-out if in-coming var may shadow
        for node in &self.cfg.nodes {
            let node_ptr = Arc::as_ptr(node);
            let cache = self.blk_cache.get_mut(&node_ptr).unwrap();
            for shadowed_name in cache
                .live_in_may_shadow
                .iter()
                .filter(|var| cache.renamed_live_in.contains_key(var.as_str()))
            {
                // if incoming conflicting def is used somewhere is the blk
                // and it is not overwritten by any liveout
                if !cache.renamed_live_out.contains_key(shadowed_name) {
                    cache.renamed_live_out.insert(
                        shadowed_name.clone(),
                        cache.renamed_live_in.get(shadowed_name).unwrap().clone(),
                    );
                }
            }
        }

        let mut registered_set_instrs: HashMap<NodePtr, BTreeSet<LabelOrInst>> = HashMap::new();

        // second pass insert set/get instr
        for node in &self.cfg.nodes {
            let node_ptr = Arc::as_ptr(node);
            let mut node_lock = node.lock().unwrap();
            let cache = self.blk_cache.get(&node_ptr).unwrap();
            let mut to_rename = cache.renamed_live_in.clone();

            for (var, new_name) in to_rename.iter_mut() {
                // non-conflicting def, we fetch remote name
                if !cache.live_in_may_shadow.contains(var) {
                    *new_name = self.fetch_remote_name(node_ptr, var);
                }
            }

            for inst in node_lock.blk.instrs.iter_mut() {
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

            // insert get exprs
            let mut get_instrs = vec![];

            for to_get in &cache.live_in_may_shadow {
                let Some(renamed) = cache.renamed_live_in.get(to_get) else {
                    continue;
                };
                let ty = cache.live_in_ty.get(to_get).unwrap();
                get_instrs.push(
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

            let first_non_label_idx = node_lock
                .blk
                .instrs
                .iter()
                .position(|inst| !matches!(inst, LabelOrInst::Label { .. }))
                .unwrap_or(1); // otherwise, basic block only contains a single label

            node_lock
                .blk
                .instrs
                .splice(first_non_label_idx..first_non_label_idx, get_instrs);

            for succ in &node_lock.successors {
                let succ_ptr = Weak::as_ptr(succ);
                let succ_cache = self.blk_cache.get(&succ_ptr).unwrap();
                for to_set in &succ_cache.live_in_may_shadow {
                    if !succ_cache.renamed_live_in.contains_key(to_set) {
                        continue;
                    }
                    self.register_set_instrs(&mut registered_set_instrs, succ_ptr, to_set);
                }
            }
        }

        for node in &self.cfg.nodes {
            let node_ptr = Arc::as_ptr(node);
            let mut node_lock = node.lock().unwrap();
            // insert set expr at the end of block before the last jmp/br inst if there is one
            let last_jmp_or_br = node_lock.blk.instrs.iter().position(
                |inst| matches!(inst, LabelOrInst::Inst {op, ..} if (op == "br") || (op == "jmp")),
            );
            let Some(set_instrs) = registered_set_instrs.remove(&node_ptr) else {
                continue;
            };
            if set_instrs.is_empty() {
                continue;
            }
            if let Some(last_jmp_or_br) = last_jmp_or_br {
                node_lock
                    .blk
                    .instrs
                    .splice(last_jmp_or_br..last_jmp_or_br, set_instrs);
            } else {
                node_lock.blk.instrs.extend(set_instrs);
            }
        }
    }

    /// register set expr at the remote blk when get expr is inserted at current blk
    fn register_set_instrs(
        &self,
        blk_set_instrs: &mut HashMap<NodePtr, BTreeSet<LabelOrInst>>,
        cur_ptr: NodePtr,
        name: &String,
    ) {
        let cache = self.blk_cache.get(&cur_ptr).unwrap();
        let remote_list = cache.reach_def.get(name.as_str()).unwrap();
        let to_set = cache.renamed_live_in.get(name.as_str()).unwrap();
        assert!(remote_list.len() > 1);
        for remote_ptr in remote_list {
            let canonical_repr = self.canonical_repr_at_blk(*remote_ptr, name);
            let set_instr = serde_json::from_str(&format!(
                r#"{{
                        "args" : ["{to_set}", "{canonical_repr}"],
                        "op": "set"
                    }}"#
            ))
            .unwrap();

            blk_set_instrs
                .entry(*remote_ptr)
                .or_default()
                .insert(set_instr);
        }
    }

    // fetch renaming of remote symbol
    fn fetch_remote_name(&self, cur_ptr: NodePtr, name: &String) -> String {
        let cache = self.blk_cache.get(&cur_ptr).unwrap();
        let root_ptr = Weak::as_ptr(&self.cfg.root);

        let func_args_name: HashSet<_> =
            HashSet::from_iter(self.cfg.func_ctx.args_name().unwrap_or_default());

        if cur_ptr == root_ptr {
            assert!(func_args_name.contains(name.as_str()));
            // directly return arg name
            return name.clone();
        }
        let remote_set = cache.reach_def.get(name).unwrap();
        assert!(remote_set.len() == 1);
        let remote_ptr = remote_set.iter().next().unwrap();
        self.canonical_repr_at_blk(*remote_ptr, name)
    }

    fn canonical_repr_at_blk(&self, ptr: NodePtr, name: &String) -> String {
        let func_args_name: HashSet<_> =
            HashSet::from_iter(self.cfg.func_ctx.args_name().unwrap_or_default());

        let root_ptr = Weak::as_ptr(&self.cfg.root);
        if let Some(canonical_repr) = self.blk_cache.get(&ptr).unwrap().renamed_live_out.get(name) {
            canonical_repr.clone()
        } else {
            assert_eq!(ptr, root_ptr);
            assert!(func_args_name.contains(name.as_str()));
            name.clone()
        }
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
                        let renamed = Self::mangle_scheme(&self.blk_label, arg, 0);
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
                *dest = Self::mangle_scheme(&self.blk_label, dest, *next_to_assign);
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

    fn init_in_flow_state(&self, node: &NodeRef) -> Self::InFlowType {
        if Arc::as_ptr(node) == self.root_ptr {
            self.args_ty.clone()
        } else {
            HashMap::new()
        }
    }

    fn transfer(node: &NodeRef, mut in_flow: Self::InFlowType) -> Self::OutFlowType {
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

    fn merge(out_flow: Vec<Self::OutFlowType>) -> Self::InFlowType {
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

pub struct ReachDefWithLabelProp {
    root_ptr: NodePtr,
    args_ty: HashMap<String, String>,
}

impl WorkListAlgo for ReachDefWithLabelProp {
    const FORWARD_PASS: bool = true;
    type InFlowType = HashMap<String, HashSet<NodePtr>>;
    type OutFlowType = HashMap<String, HashSet<NodePtr>>;
    fn init_in_flow_state(&self, node: &NodeRef) -> Self::InFlowType {
        let node_ptr = Arc::as_ptr(node);
        if node_ptr == self.root_ptr {
            self.args_ty
                .keys()
                .map(|arg| (arg.clone(), HashSet::from([node_ptr])))
                .collect()
        } else {
            HashMap::new()
        }
    }

    fn merge(out_flows: Vec<Self::OutFlowType>) -> Self::InFlowType {
        out_flows
            .into_iter()
            .reduce(|mut acc, next| {
                for (var, from) in next {
                    if let Some(cur_from) = acc.get_mut(&var) {
                        cur_from.extend(from);
                    } else {
                        acc.insert(var.clone(), from);
                    };
                }
                acc
            })
            .unwrap_or_default()
    }

    fn transfer(node: &NodeRef, mut in_flow: Self::InFlowType) -> Self::OutFlowType {
        let node_ptr = Arc::as_ptr(node);
        let node_lock = node.lock().unwrap();
        let blk = &node_lock.blk;

        let used_but_not_defed = blk.used_but_not_defed();

        let able_to_kill: HashMap<_, _> = blk
            .defs()
            .into_iter()
            .map(|var| (var, HashSet::from([node_ptr])))
            .collect();

        in_flow.iter_mut().for_each(|(var, from)| {
            // only reset source if conflicting def is used within the block
            if from.len() > 1 && used_but_not_defed.contains(var) {
                *from = HashSet::from([node_ptr]);
            }
        });
        in_flow.extend(able_to_kill);
        in_flow
    }
}
