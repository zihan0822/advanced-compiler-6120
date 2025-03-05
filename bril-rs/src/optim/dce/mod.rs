//! basic dead-code-elimination algorithm, which is able to
//!   - delete unused var
//!   - compile time const folding
pub mod global;
use crate::analyzer;
use crate::bril::{LabelOrInst, ValueLit};
use crate::cfg::{BasicBlock, Cfg, NodePtr};

use std::collections::{HashMap, HashSet};
use std::default::Default;
use std::sync::{
    atomic::{self, AtomicUsize},
    Arc,
};

static RENAME_COUNTER: AtomicUsize = AtomicUsize::new(7654);

pub fn dce(cfg: Cfg, global_const_folding: bool) -> Cfg {
    let global_const_folding_ctx = if global_const_folding {
        Some(analyzer::find_global_const_folding_ctx(&cfg))
    } else {
        None
    };

    // expose dead code elimination opportunity
    for node in &cfg.nodes {
        let node_ptr = Arc::as_ptr(node);
        let mut node_lock = node.lock().unwrap();

        let vn_ctx_builder = ValueNumberingCtxBuilder::new();
        let vn_ctx = if global_const_folding {
            let const_folding_ctx = global_const_folding_ctx
                .as_ref()
                .unwrap()
                .get(&node_ptr)
                .unwrap();
            vn_ctx_builder.global_const_ctx(const_folding_ctx).finish()
        } else {
            vn_ctx_builder.const_folding().finish()
        };
        node_lock.blk = value_numbering(node_lock.blk.clone(), vn_ctx);
    }

    let unused_dangling_vars: HashMap<NodePtr, HashSet<String>> =
        global::find_unused_variables_per_node(&cfg);

    for node in &cfg.nodes {
        let node_ptr = Arc::as_ptr(node);
        let mut node_lock = node.lock().unwrap();
        let delete_live_on_exit = unused_dangling_vars.get(&node_ptr).unwrap();
        node_lock.blk = dce_on_blk(node_lock.blk.clone(), delete_live_on_exit);
    }
    cfg
}

fn dce_on_blk(mut blk: BasicBlock, delete_live_on_exit: &HashSet<String>) -> BasicBlock {
    let mut updated;
    loop {
        (blk, updated) = dce_on_blk_one_pass(blk, delete_live_on_exit);
        if !updated {
            break blk;
        }
    }
}

fn dce_on_blk_one_pass(
    mut blk: BasicBlock,
    delete_live_on_exit: &HashSet<String>,
) -> (BasicBlock, bool) {
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
    unused_variable.retain(|var, _| delete_live_on_exit.contains(var));
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

pub fn value_numbering(mut blk: BasicBlock, mut ctx: ValueNumberingCtx) -> BasicBlock {
    let dangling_playback = conservative_var_renaming(&mut blk);
    if dangling_playback
        .into_values()
        .flat_map(|v| v.into_iter())
        .collect::<Vec<_>>()
        .is_empty()
    {
        // no optimization performed if there is incoming variable carried over from ancestor basic block
        ctx.numbering_scan(blk)
    } else {
        blk
    }
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

pub struct ValueNumberingCtxBuilder(ValueNumberingCtx);
impl Default for ValueNumberingCtxBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ValueNumberingCtxBuilder {
    pub fn new() -> Self {
        Self(ValueNumberingCtx {
            const_folding: false,
            ..Default::default()
        })
    }
    pub fn const_folding(mut self) -> Self {
        self.0.const_folding = true;
        self
    }
    pub fn global_const_ctx(mut self, in_flow: &HashMap<String, ValueLit>) -> Self {
        self.0.const_folding = true;
        for (var, const_lit) in in_flow {
            let entry = Arc::new(NumTableEntry {
                canonical_var: var.clone(),
                const_lit: Some(*const_lit),
                numbering: self.0.next_number,
            });
            self.0.var2numbering.insert(var.clone(), entry);
            self.0.next_number += 1;
        }
        self
    }

    pub fn finish(self) -> ValueNumberingCtx {
        self.0
    }
}

impl ValueNumberingCtx {
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
                    let dest = dest.clone().unwrap();
                    let new_entry = Arc::new(NumTableEntry {
                        numbering: self.next_number,
                        canonical_var: dest.clone(),
                        const_lit: *value,
                    });
                    let _ = self.var2numbering.insert(dest, new_entry);
                    self.next_number += 1;
                } else if op == "id" {
                    let arg = &args.as_ref().unwrap()[0];
                    let dest = dest.clone().unwrap();
                    if let Some(num_entry) = self.var2numbering.get(arg) {
                        if self.const_folding && num_entry.const_lit.is_some() {
                            *args = None;
                            *op = "const".to_string();
                            *value = num_entry.const_lit;
                        } else {
                            *args = Some(vec![num_entry.canonical_var.clone()]);
                        }
                        let _ = self.var2numbering.insert(dest, Arc::clone(num_entry));
                    } else {
                        // otherwise, arg coming from upperstream basic block, we do not do anything
                        let new_entry = Arc::new(NumTableEntry {
                            numbering: self.next_number,
                            canonical_var: arg.clone(),
                            const_lit: None,
                        });
                        let _ = self.var2numbering.insert(dest, new_entry);
                        self.next_number += 1;
                    }
                } else if let Some(dest) = dest {
                    // for function call, return value may be different even the (call, func, *args) tuple is the same
                    if op == "call" {
                        let new_entry = Arc::new(NumTableEntry {
                            numbering: self.next_number,
                            canonical_var: dest.clone(),
                            const_lit: None,
                        });
                        self.next_number += 1;
                        let _ = self.var2numbering.insert(dest.clone(), new_entry);
                        continue;
                    }

                    // remaining are deterministic ops, for example add, lt, mul, ...
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
                        Err(canon_form) => {
                            // not found entry
                            let const_lit = if self.const_folding {
                                self.try_eval_const_expr(op, args_lit)
                            } else {
                                None
                            };
                            self.insert_new_numbering(dest, canon_form, const_lit);
                            // this might fallback to another branch if any of the args can not be const evaled
                            if let Some(const_lit) = const_lit {
                                *op = "const".to_string();
                                *args = None;
                                *value = Some(const_lit);
                            } else {
                                let reduced_args = args_lit
                                    .iter()
                                    .map(|arg| {
                                        self.var2numbering.get(arg).map_or(arg.clone(), |entry| {
                                            entry.canonical_var.clone()
                                        })
                                    })
                                    .collect();
                                *args = Some(reduced_args);
                            }
                        }
                    };
                } else if let Some(ref mut args) = args {
                    *args = args
                        .iter()
                        .map(|arg| {
                            self.var2numbering
                                .get(arg)
                                .map_or(arg.clone(), |entry| entry.canonical_var.clone())
                        })
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
    ) -> Result<Arc<NumTableEntry>, CanonicalForm> {
        let mut renumbered_args: Vec<String> = vec![];
        for arg in args {
            // if we can find arg in current block's var2numbering map, we use its numbering
            // otherwise, arg should be defined in upper stream ancestor block, we keep its name
            renumbered_args.push(
                self.var2numbering
                    .get(arg)
                    .map_or(arg.clone(), |entry| entry.numbering.to_string()),
            );
        }
        let num_table_key = CanonicalForm::from_op_and_numbered_args(op, &renumbered_args);
        self.num_table
            .get(&num_table_key)
            .cloned()
            .ok_or(num_table_key)
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

#[derive(Eq, Hash, PartialEq, Debug)]
struct CanonicalForm {
    op: String,
    // associativity is exploited
    numbered_args: Vec<String>,
}

#[derive(Clone, Debug)]
struct NumTableEntry {
    canonical_var: String,
    const_lit: Option<ValueLit>,
    numbering: usize,
}

impl CanonicalForm {
    fn from_op_and_numbered_args(op: &str, numbered_args: &[String]) -> Self {
        let mut numbered_args: Vec<String> = numbered_args.to_vec();
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
pub fn conservative_var_renaming(blk: &mut BasicBlock) -> HashMap<String, Vec<&mut String>> {
    // first pass found all variable that needs renaming
    let mut rename_scheme: HashMap<String, Vec<&mut String>> = HashMap::new();
    for inst in blk.instrs.iter_mut().rev() {
        if let LabelOrInst::Inst {
            dest: Some(ref mut dest),
            ..
        } = inst
        {
            if let Some(playback) = rename_scheme.insert(dest.clone(), vec![]) {
                let mangled_name = var_mangle_scheme(dest);
                // if dest is never accessed we leave its name intact, so that it can be eliminated by later dce
                if !playback.is_empty() {
                    *dest = mangled_name.clone();
                }
                for arg in playback {
                    *arg = mangled_name.clone();
                }
            }
        }
        if let LabelOrInst::Inst {
            args: Some(args), ..
        } = inst
        {
            for arg in args.iter_mut() {
                if let Some(playback) = rename_scheme.get_mut(arg) {
                    playback.push(arg);
                }
            }
        }
    }
    rename_scheme
}

fn var_mangle_scheme(origin_name: &str) -> String {
    format!(
        "__{origin_name}_{}",
        RENAME_COUNTER.fetch_add(1, atomic::Ordering::Relaxed)
    )
}
