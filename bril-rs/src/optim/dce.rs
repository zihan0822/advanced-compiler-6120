//! basic dead-code-elimination algorithm, which is able to
//!   - delete unused var
//!   - compile time const folding
use crate::bril::{LabelOrInst, ValueLit};
use crate::cfg::BasicBlock;
use std::collections::HashMap;
use std::sync::{
    atomic::{self, AtomicUsize},
    Arc,
};

static RENAME_COUNTER: AtomicUsize = AtomicUsize::new(7654);

pub fn dce(mut blk: BasicBlock) -> BasicBlock {
    let mut updated;
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
    to_be_deleted.extend(unused_variable.into_values());
    let updated = !to_be_deleted.is_empty();

    if updated {
        blk.instrs = blk
            .instrs
            .into_iter()
            .enumerate()
            .filter_map(|(idx, inst)| {
                if to_be_deleted.iter().any(|&x| x == idx) {
                    None
                } else {
                    Some(inst)
                }
            })
            .collect()
    }
    (blk, updated)
}

pub fn value_numbering(blk: BasicBlock, const_folding: bool) -> BasicBlock {
    let mut ctx = ValueNumberingCtx::new(const_folding);
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
///     1. op aware [ok]
///     2. compile time const eval [ok]
///     3. intra-block dependency
///     4. variable renaming [ok?]
#[derive(Default)]
pub struct ValueNumberingCtx {
    num_table: HashMap<CanonicalForm, Arc<NumTableEntry>>,
    var2numbering: HashMap<String, Arc<NumTableEntry>>,
    next_number: usize,
    const_folding: bool,
}

impl ValueNumberingCtx {
    pub fn new(const_folding: bool) -> Self {
        Self {
            const_folding,
            ..std::default::Default::default()
        }
    }

    pub fn numbering_scan(&mut self, mut blk: BasicBlock) -> BasicBlock {
        for inst in &mut blk.instrs {
            if let LabelOrInst::Inst {
                op,
                args,
                dest,
                value,
                ..
            } = inst
            {
                if op == "const" {
                    // op of const is considered to be `id`, so that later query of id `dest` will
                    // be routed here
                    self.insert_new_numbering(
                        dest.as_ref().unwrap(),
                        CanonicalForm::from_op_and_args("id", &[self.next_number]),
                        *value,
                    );
                } else if let Some(dest) = dest {
                    let dest = dest.clone();
                    // vector of args literal
                    let args_lit = &args.as_ref().unwrap();
                    match self.numbering_query(op, args_lit) {
                        Ok(num_entry) => {
                            if self.const_folding && num_entry.const_lit.is_some() {
                                *args = None;
                                *op = "const".to_string();
                                *value = num_entry.const_lit;
                            } else {
                                *op = "id".to_string();
                                *args = Some(vec![num_entry.canonical_var.clone()]);
                            }
                            self.var2numbering.insert(dest.clone(), num_entry);
                        }
                        Err(NumTableErr::EntryNotFound(canon_form)) => {
                            // not found entry
                            let const_lit = if self.const_folding {
                                self.try_eval_const_expr(op, args_lit)
                            } else {
                                None
                            };
                            self.insert_new_numbering(&dest, canon_form, const_lit);
                            // this might fallback to another branch if any of the args can not be const evaled
                            if let Some(const_lit) = const_lit {
                                *op = "const".to_string();
                                *args = None;
                                *value = Some(const_lit);
                            } else {
                                let reduced_args = args_lit
                                    .iter()
                                    .map(|arg| {
                                        self.var2numbering.get(arg).unwrap().canonical_var.clone()
                                    })
                                    .collect();
                                *args = Some(reduced_args);
                            }
                        }
                        // yet to impl, this handles the case where some of the argument comes from
                        Err(NumTableErr::ArgNotNumbered) => unreachable!(),
                    };
                } else if let Some(ref mut args) = args {
                    *args = args
                        .iter()
                        .map(|arg| self.var2numbering.get(arg).unwrap().canonical_var.clone())
                        .collect();
                }
            }
        }
        blk
    }

    /// create new numbering for variable `dest`
    fn insert_new_numbering(
        &mut self,
        dest: &str,
        canon_form: CanonicalForm,
        const_lit: Option<ValueLit>,
    ) {
        let new_entry = Arc::new(NumTableEntry {
            numbering: self.next_number,
            const_lit,
            canonical_var: dest.to_string(),
        });
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
    fn numbering_query(
        &self,
        op: &str,
        args: &[String],
    ) -> Result<Arc<NumTableEntry>, NumTableErr> {
        dbg!(args);
        dbg!(&self.var2numbering);
        dbg!(&self.num_table);

        let mut renumbered_args: Vec<usize> = vec![];
        for arg in args {
            renumbered_args.push(
                self.var2numbering
                    .get(arg)
                    .ok_or(NumTableErr::ArgNotNumbered)?
                    .numbering,
            );
        }
        let num_table_key = CanonicalForm::from_op_and_args(op, &renumbered_args);
        self.num_table
            .get(&num_table_key)
            .cloned()
            .ok_or(NumTableErr::EntryNotFound(num_table_key))
    }

    fn try_eval_const_expr(&self, op: &str, args: &[String]) -> Option<ValueLit> {
        let mut const_binding = vec![];
        for arg in args {
            const_binding.push(self.var2numbering.get(arg)?.const_lit?);
        }
        match op {
            "id" => {
                assert!(const_binding.len() == 1);
                Some(const_binding[0])
            }
            "add" => {
                assert!(const_binding.len() == 2);
                if let (ValueLit::Int(a1), ValueLit::Int(a2)) = (const_binding[0], const_binding[1])
                {
                    Some(ValueLit::Int(a1 + a2))
                } else {
                    unreachable!()
                }
            }
            "sub" => {
                assert!(const_binding.len() == 2);
                if let (ValueLit::Int(a1), ValueLit::Int(a2)) = (const_binding[0], const_binding[1])
                {
                    Some(ValueLit::Int(a1 - a2))
                } else {
                    unreachable!()
                }
            }
            "mul" => {
                assert!(const_binding.len() == 2);
                if let (ValueLit::Int(a1), ValueLit::Int(a2)) = (const_binding[0], const_binding[1])
                {
                    Some(ValueLit::Int(a1 * a2))
                } else {
                    unreachable!()
                }
            }
            "div" => {
                assert!(const_binding.len() == 2);
                if let (ValueLit::Int(a1), ValueLit::Int(a2)) = (const_binding[0], const_binding[1])
                {
                    Some(ValueLit::Int(a1 / a2))
                } else {
                    unreachable!()
                }
            }
            _ => None,
        }
    }
}

enum NumTableErr {
    EntryNotFound(CanonicalForm),
    // this might happen if argument is defined outside this basic block
    ArgNotNumbered,
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
    const_lit: Option<ValueLit>,
    numbering: usize,
}

impl CanonicalForm {
    fn from_op_and_args(op: &str, numbered_args: &[usize]) -> Self {
        let mut numbered_args: Vec<_> = numbered_args.to_vec();
        if matches!(op, "add" | "mul") {
            numbered_args.sort()
        }
        Self {
            op: op.to_string(),
            numbered_args,
        }
    }
}

/// this function scans through inst list
/// and renames every inst.dest which will be overwritten later in the basic block with a randomly generated name
/// we don't rename args which is not defined with the basic block
/// this resolves the problem of re-assigning canonical variable
/// it is considered to be safe even in inter-basic-block context
pub fn conservative_var_renaming(blk: &mut BasicBlock) {
    // first pass found all variable that needs renaming
    let mut rename_scheme = HashMap::new();
    for inst in blk.instrs.iter_mut().rev() {
        if let LabelOrInst::Inst {
            dest: Some(ref mut dest),
            ..
        } = inst
        {
            if let Some(mangled_name) = rename_scheme.insert(dest.clone(), var_mangle_scheme(dest))
            {
                *dest = mangled_name;
            }
        }
        if let LabelOrInst::Inst {
            args: Some(args), ..
        } = inst
        {
            for arg in args.iter_mut() {
                if let Some(mangled_name) = rename_scheme.get(arg) {
                    *arg = mangled_name.clone();
                }
            }
        }
    }
}

fn var_mangle_scheme(origin_name: &str) -> String {
    format!(
        "__{origin_name}_{}",
        RENAME_COUNTER.fetch_add(1, atomic::Ordering::Relaxed)
    )
}
