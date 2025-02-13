use crate::bril::{LabelOrInst, ValueLit};
use crate::cfg::{Cfg, FuncCtx, NodePtr, NodeRef};
use crate::optim::dflow::WorkListAlgo;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use std::sync::{Arc, Weak};

/// const propagation
/// merge:
///     U {out[p]} but exclude those conflict variables
/// transfer:
///       in[n] add const var, exclude non-const var
struct GlobalConstPropAlgo;

pub fn global_const_prop(cfg: &Cfg) -> Vec<(Option<String>, HashMap<String, ValueLit>)> {
    let ret = GlobalConstPropAlgo.execute(cfg);
    let mut outputs = vec![];
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
        if !reached_consts.is_empty() {
            let label = node.lock().unwrap().blk.label.clone();
            outputs.push((label, reached_consts));
        }
    }
    outputs
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VarType {
    Unknown,
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
        let mut var_tys = in_flows.unwrap_or_default();
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
        out_flows
            .into_iter()
            .reduce(|mut out_a, out_b| {
                for (var, ty) in out_b.into_iter() {
                    out_a
                        .entry(var)
                        .and_modify(|prev_ty| match (*prev_ty, ty) {
                            (VarType::Const(_), VarType::Unknown) => *prev_ty = VarType::Unknown,
                            (VarType::Const(ref val_a), VarType::Const(ref val_b))
                                if !val_b.eq(val_a) =>
                            {
                                *prev_ty = VarType::Unknown
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

pub fn uninitialized_var_detection(cfg: &Cfg, func_ctx: &FuncCtx) -> Result<(), String> {
    let mut algo = UninitDetectAlgo::new(cfg, func_ctx);
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
struct UninitDetectAlgo<'a> {
    root_ptr: NodePtr,
    func_args: Option<Vec<&'a String>>,
    per_blk_uninit: HashMap<NodePtr, BTreeMap<usize, Vec<String>>>,
}

impl<'a> UninitDetectAlgo<'a> {
    fn new(cfg: &'a Cfg, func_ctx: &'a FuncCtx) -> Self {
        let root_ptr = Weak::as_ptr(&cfg.root);
        let func_args = func_ctx
            .args
            .as_ref()
            .map(|args_ty| args_ty.iter().map(|arg_ty| &arg_ty.name).collect());
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
    Init,
}

impl WorkListAlgo for UninitDetectAlgo<'_> {
    const FORWARD_PASS: bool = true;
    type InFlowType = HashMap<String, VarInitState>;
    type OutFlowType = HashMap<String, VarInitState>;

    fn transfer(&mut self, node: &NodeRef, in_flow: Option<Self::InFlowType>) -> Self::OutFlowType {
        let node_ptr = Arc::as_ptr(node);
        let node_lock = node.lock().unwrap();
        let mut in_flow = if node_ptr == self.root_ptr {
            self.func_args.as_ref().map_or(HashMap::new(), |args| {
                args.iter()
                    .map(|arg| (String::clone(arg), VarInitState::Init))
                    .collect()
            })
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
                    let mut uninit = false;
                    for arg in args {
                        if let Some(arg_state) = in_flow.get(arg) {
                            if matches!(arg_state, VarInitState::Uninit) {
                                uninit = true;
                            }
                        } else {
                            uninit_per_line.push(arg.clone());
                            in_flow.insert(arg.clone(), VarInitState::Uninit);
                            uninit = true;
                        }
                    }
                    if uninit {
                        uninit_per_line.push(dest.clone());
                        in_flow.insert(dest, VarInitState::Uninit);
                    } else {
                        in_flow.insert(dest, VarInitState::Init);
                    }
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
                        .and_modify(|prev_state| {
                            if let VarInitState::Uninit = *prev_state {
                                *prev_state = state
                            }
                        })
                        .or_insert(state);
                }
                acc
            })
            .unwrap_or_default()
    }
}
