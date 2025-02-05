//! basic dead-code-elimination algorithm, which is able to
//!   - delete unused var
use crate::bril::LabelOrInst;
use crate::cfg::BasicBlock;
use std::collections::{hash_map::Entry, HashMap};
use std::sync::Arc;

pub fn dce(mut blk: BasicBlock) -> BasicBlock {
    let mut updated = false;
    loop {
        (blk, updated) = dce_scan(blk);
        if !updated {
            break blk;
        }
    }
}

fn dce_scan(mut blk: BasicBlock) -> (BasicBlock, bool) {
    let mut to_be_deleted = vec![];
    let mut unused_variable: HashMap<String, usize> = HashMap::new();

    for (i, inst) in blk.instrs.iter().enumerate() {
        if let LabelOrInst::Inst {
            args: Some(args), ..
        } = &inst
        {
            for arg in args {
                let _ = unused_variable.remove(arg);
            }
        }
        if let LabelOrInst::Inst {
            dest: Some(dest), ..
        } = &inst
        {
            if let Some(last_assign_idx) = unused_variable.insert(dest.clone(), i) {
                to_be_deleted.push(last_assign_idx);
            }
        }
    }
    to_be_deleted.extend(unused_variable.into_iter().map(|(_, idx)| idx));
    let updated = !to_be_deleted.is_empty();

    if updated {
        blk.instrs = blk
            .instrs
            .into_iter()
            .enumerate()
            .filter_map(|(idx, inst)| {
                if to_be_deleted.iter().position(|&x| x == idx).is_some() {
                    None
                } else {
                    Some(inst)
                }
            })
            .collect()
    }
    (blk, updated)
}


pub fn value_numbering(blk: BasicBlock) -> BasicBlock {
    let mut ctx = ValueNumberingCtx::new();
    ctx.numbering_scan(blk)
}
/// data structs used in local numbering
///
///     - var2numbering: map variable name to its numbering and canonical representative
///     - num_table: map canonical form of exprs to (number, caonical_var)
///
/// procedure:
///     For each inst:
///         1. canonicalize it with respective to current numbering and op, reduced to T
///         2. try to find T in current global num table
///                 - if found:
///                     - bind inst.dest to new numbering
///                 - else:
///                     - insert new entry then bind
/// Extension:
///     1. op aware
///     2. compile time const eval
///     3. intra-block dependency
///     4. variable renumbering
///
#[derive(Default)]
pub struct ValueNumberingCtx {
    num_table: HashMap<CanonicalForm, NumTableEntry>,
    var2numbering: HashMap<String, NumTableEntry>,
    next_number: usize,
}

impl ValueNumberingCtx {
    pub fn new() -> Self {
        std::default::Default::default()
    }

    pub fn numbering_scan(&mut self, mut blk: BasicBlock) -> BasicBlock {
        for inst in &mut blk.instrs {
            if let LabelOrInst::Inst { op, args, dest, .. } = inst {
                if op == "const" {
                    // op of const is considered to be `id`, so that later query of id `dest` will
                    // be routed here
                    self.insert_new_numbering(
                        &dest.as_ref().unwrap(),
                        CanonicalForm::from_op_and_args("id", &vec![]),
                    );
                } else if Self::require_numbering(op) {
                    let dest = dest.clone().unwrap();
                    match self.numbering_query(op, &args.as_ref().unwrap()) {
                        Ok(num_entry) => {
                            *op = "id".to_string();
                            *args = Some(vec![num_entry.canonical_var.clone()]);
                            self.var2numbering.insert(dest.clone(), num_entry);
                        }
                        Err(canon_form) => {
                            // not found entry
                            self.insert_new_numbering(&dest, canon_form);
                            let reduced_args = args
                                .as_ref()
                                .unwrap()
                                .iter()
                                .map(|arg| {
                                    self.var2numbering.get(arg).unwrap().canonical_var.clone()
                                })
                                .collect();
                            *args = Some(reduced_args);
                        }
                    };
                }
            }
        }
        blk
    }

    #[inline]
    fn require_numbering(op: &str) -> bool {
        matches!(op, "id" | "add" | "sub" | "mul" | "div")
    }

    /// create new numbering for variable `dest`
    fn insert_new_numbering(&mut self, dest: &str, canon_form: CanonicalForm) {
        let new_entry = NumTableEntry {
            numbering: self.next_number,
            canonical_var: dest.to_string(),
        };
        self.var2numbering
            .insert(dest.to_string(), new_entry.clone());
        self.num_table.insert(canon_form, new_entry);
        self.next_number += 1;
    }
    /// given bril inst as input
    /// may canonicalize it depending op type
    /// returns the numbering of dest var, if failed, returns a tuple of (canonical form, dest)
    ///
    /// Layout of Numbering Table
    ///     Numbering           Expr           Canonical Var
    ///       #1                var              "x"
    ///                         ...
    ///       #5               (add, #1, #2)     "sum"
    ///
    fn numbering_query(&self, op: &str, args: &[String]) -> Result<NumTableEntry, CanonicalForm> {
        dbg!(args);
        dbg!(&self.var2numbering);
        let renumbered_args: Vec<usize> = args
            .iter()
            .map(|arg| self.var2numbering.get(arg).unwrap().numbering)
            .collect();
        let num_table_key = CanonicalForm::from_op_and_args(op, &renumbered_args);
        self.num_table
            .get(&num_table_key)
            .cloned()
            .ok_or(num_table_key)
    }
}

#[derive(Eq, Hash, PartialEq, Debug)]
struct CanonicalForm {
    op: String,
    // associativity is exploited
    numbered_args: Vec<usize>,
}

#[derive(Clone, Debug)]
struct NumTableEntry {
    canonical_var: String,
    numbering: usize,
}

impl CanonicalForm {
    fn from_op_and_args(op: &str, numbered_args: &[usize]) -> Self {
        let mut numbered_args: Vec<_> = numbered_args.iter().cloned().collect();
        if matches!(op, "add" | "mul") {
            numbered_args.sort()
        }
        Self {
            op: op.to_string(),
            numbered_args,
        }
    }
}
