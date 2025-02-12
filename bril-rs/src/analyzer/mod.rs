use crate::bril::{LabelOrInst, ValueLit};
use crate::cfg::{Cfg, NodeRef};
use crate::optim::dflow::WorkListAlgo;
use std::collections::HashMap;
use std::sync::Arc;

/// const propagation
/// merge:
///     U {out[p]} but exclude those conflict variables
/// transfer:
///       in[n] add const var, exclude non-const var
struct GlobalConstPropAlgo;

pub fn global_const_prop(cfg: &Cfg) -> Vec<(Option<String>, HashMap<String, ValueLit>)> {
    let ret = GlobalConstPropAlgo::execute(cfg);
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

    fn transfer(node: &NodeRef, in_flows: Option<Self::InFlowType>) -> Self::OutFlowType {
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
