pub mod dom;
use crate::bril::{LabelOrInst, ValueLit};
use crate::cfg::{Cfg, NodePtr, NodeRef};
use crate::optim::{self, dflow::WorkListAlgo};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use std::sync::{Arc, Weak};

/// const propagation
/// merge:
///     U {out[p]} but exclude those conflict variables
/// transfer:
///       in[n] add const var, exclude non-const var
struct GlobalConstPropAlgo {
    root_ptr: NodePtr,
    func_args: Option<Vec<String>>,
}

impl GlobalConstPropAlgo {
    fn new(cfg: &Cfg) -> Self {
        Self {
            root_ptr: Weak::as_ptr(&cfg.root),
            func_args: cfg.func_ctx.args_name(),
        }
    }
}

pub fn find_global_const_folding_ctx(cfg: &Cfg) -> HashMap<NodePtr, HashMap<String, ValueLit>> {
    let ret = GlobalConstPropAlgo::new(cfg).execute(cfg);
    let mut global_ctx = HashMap::new();

    for node in &cfg.nodes {
        let node_ptr = Arc::as_ptr(node);
        let reached_consts: HashMap<_, _> = ret
            .get(&node_ptr)
            .unwrap()
            .clone()
            .into_iter()
            .filter_map(|(var, ty)| {
                if let VarType::Const(const_lit) = ty {
                    Some((var, const_lit))
                } else {
                    None
                }
            })
            .collect();
        global_ctx.insert(node_ptr, reached_consts);
    }
    global_ctx
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VarType {
    Unknown,
    NonConst,
    Const(ValueLit),
}

impl WorkListAlgo for GlobalConstPropAlgo {
    const FORWARD_PASS: bool = true;
    type InFlowType = HashMap<String, VarType>;
    type OutFlowType = HashMap<String, VarType>;

    fn transfer(
        &mut self,
        node: &NodeRef,
        in_flows: Option<Self::InFlowType>,
    ) -> Self::OutFlowType {
        let node_lock = node.lock().unwrap();
        let blk = &node_lock.blk;
        let mut var_tys = if Arc::as_ptr(node) == self.root_ptr {
            if let Some(ref args) = self.func_args {
                args.iter()
                    .map(|arg| (String::clone(arg), VarType::NonConst))
                    .collect()
            } else {
                HashMap::new()
            }
        } else {
            in_flows.unwrap_or_default()
        };
        for inst in &blk.instrs {
            if let LabelOrInst::Inst {
                dest: Some(dest),
                op,
                args,
                value,
                ..
            } = inst
            {
                if op == "const" {
                    let _ = var_tys.insert(dest.clone(), VarType::Const((*value).unwrap()));
                }
                if matches!(op.as_str(), "add" | "sub" | "mul" | "div" | "id") {
                    let args = args.as_ref().unwrap();
                    let can_be_folded = args
                        .iter()
                        .all(|arg| matches!(var_tys.get(arg), Some(VarType::Const(_))));
                    let var_ty = if can_be_folded {
                        let const_args: Vec<_> =
                            args.iter().map(|arg| var_tys.get(arg).unwrap()).collect();
                        match op.as_str() {
                            "add" => crate::const_eval!(const_args[0], +, const_args[1]),
                            "sub" => crate::const_eval!(const_args[0], -, const_args[1]),
                            "mul" => crate::const_eval!(const_args[0], *, const_args[1]),
                            "div" => crate::const_eval!(const_args[0], /, const_args[1]),
                            "id" => *const_args[0],
                            _ => unreachable!(),
                        }
                    } else if args
                        .iter()
                        .any(|arg| matches!(var_tys.get(arg), Some(VarType::NonConst)))
                    {
                        VarType::NonConst
                    } else {
                        VarType::Unknown
                    };
                    var_tys.insert(dest.clone(), var_ty);
                }
            }
        }
        var_tys
    }

    fn merge(out_flows: Vec<Self::OutFlowType>) -> Self::InFlowType {
        dbg!(&out_flows);
        out_flows
            .into_iter()
            .reduce(|mut out_a, out_b| {
                for (var, ty) in out_b.into_iter() {
                    out_a
                        .entry(var)
                        .and_modify(|prev_ty| match (*prev_ty, ty) {
                            (VarType::Unknown, _) => *prev_ty = ty,
                            (VarType::Const(_), VarType::NonConst) => *prev_ty = VarType::NonConst,
                            (VarType::Const(ref val_a), VarType::Const(ref val_b))
                                if !val_b.eq(val_a) =>
                            {
                                *prev_ty = VarType::NonConst
                            }
                            _ => {}
                        })
                        .or_insert(ty);
                }
                out_a
            })
            .unwrap_or_default()
    }
}

#[macro_export]
macro_rules! const_eval {
    ($lhs: expr, $op: tt, $rhs: expr) => {
        match ($lhs, $rhs) {
            (VarType::Const(ValueLit::Int(lhs)), VarType::Const(ValueLit::Int(rhs))) => VarType::Const(ValueLit::Int(lhs $op rhs)),
            _ => unreachable!()
        }
    }
}

pub fn uninitialized_var_detection(cfg: &Cfg) -> Result<(), String> {
    let mut algo = UninitDetectAlgo::new(cfg);
    algo.execute(cfg);
    let mut fault_msg = String::new();
    for node in &cfg.nodes {
        let node_ptr = Arc::as_ptr(node);
        let node_lock = node.lock().unwrap();
        let blk_uninit = algo.per_blk_uninit.get(&node_ptr).unwrap();
        if !blk_uninit.is_empty() {
            writeln!(
                &mut fault_msg,
                "Label: .{}",
                node_lock.label.as_ref().unwrap_or(&"".to_string())
            )
            .unwrap();
            for (line, vars) in blk_uninit {
                if !vars.is_empty() {
                    writeln!(
                        &mut fault_msg,
                        "line {}: {}",
                        line,
                        vars.to_vec().join(", ")
                    )
                    .unwrap();
                }
            }
        }
    }

    if fault_msg.is_empty() {
        Ok(())
    } else {
        Err(fault_msg)
    }
}

/// Initialized Variable Detection Algo
/// backward pass
/// merge:
///     U {out[p] p in predecessor},
///     if in any of the predecessor, a variable is marked as uninit, it will be uninit after the merge
/// transfer:
///     kill variables defined in blk
///     add variables defined in upperstream
struct UninitDetectAlgo {
    root_ptr: NodePtr,
    func_args: Option<Vec<String>>,
    per_blk_uninit: HashMap<NodePtr, BTreeMap<usize, Vec<String>>>,
}

impl UninitDetectAlgo {
    fn new(cfg: &Cfg) -> Self {
        let root_ptr = Weak::as_ptr(&cfg.root);
        let func_args = cfg.func_ctx.args_name();
        Self {
            root_ptr,
            func_args,
            per_blk_uninit: HashMap::new(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VarInitState {
    Uninit,
    Unknown,
    Init,
}

impl WorkListAlgo for UninitDetectAlgo {
    const FORWARD_PASS: bool = true;
    type InFlowType = HashMap<String, VarInitState>;
    type OutFlowType = HashMap<String, VarInitState>;

    fn transfer(&mut self, node: &NodeRef, in_flow: Option<Self::InFlowType>) -> Self::OutFlowType {
        let node_ptr = Arc::as_ptr(node);
        let node_lock = node.lock().unwrap();
        let mut in_flow = if node_ptr == self.root_ptr {
            let mut undefed = optim::dce::global::used_but_not_defed(&node_lock.blk);
            let mut defed = self.func_args.as_ref().map_or(HashMap::new(), |args| {
                args.iter()
                    .map(|arg| (String::clone(arg), VarInitState::Init))
                    .collect()
            });
            undefed.retain(|var| !defed.contains_key(var));
            defed.extend(undefed.into_iter().map(|var| (var, VarInitState::Uninit)));
            defed
        } else {
            in_flow.unwrap_or_default()
        };

        let book_keeping = self.per_blk_uninit.entry(node_ptr).or_default();

        for (idx, inst) in node_lock.blk.instrs.iter().enumerate() {
            if let LabelOrInst::Inst {
                dest: Some(dest),
                op,
                args,
                ..
            } = inst
            {
                let dest = dest.clone();
                let mut uninit_per_line = vec![];
                if op.as_str() == "const" {
                    in_flow.insert(dest, VarInitState::Init);
                } else {
                    let args = args.as_ref().unwrap();
                    let mut dest_state = VarInitState::Init;
                    for arg in args {
                        if let Some(arg_state) = in_flow.get(arg) {
                            if matches!(arg_state, VarInitState::Uninit) {
                                dest_state = VarInitState::Uninit;
                            }
                        } else {
                            uninit_per_line.push(arg.clone());
                            in_flow.insert(arg.clone(), VarInitState::Unknown);
                            if dest_state == VarInitState::Init {
                                dest_state = VarInitState::Unknown;
                            }
                        }
                    }
                    if matches!(dest_state, VarInitState::Uninit) {
                        uninit_per_line.push(dest.clone());
                    }
                    in_flow.insert(dest, dest_state);
                }
                if uninit_per_line.is_empty() {
                    book_keeping.remove(&idx);
                } else {
                    book_keeping.insert(idx, uninit_per_line);
                }
            }
        }
        in_flow
    }

    fn merge(out_flow: Vec<Self::OutFlowType>) -> Self::InFlowType {
        out_flow
            .into_iter()
            .reduce(|mut acc, flow| {
                for (var, state) in flow.into_iter() {
                    acc.entry(var)
                        .and_modify(|prev_state| match (*prev_state, state) {
                            (VarInitState::Unknown, _) => *prev_state = state,
                            (VarInitState::Init, VarInitState::Uninit) => *prev_state = state,
                            _ => {}
                        })
                        .or_insert(state);
                }
                acc
            })
            .unwrap_or_default()
    }
}
