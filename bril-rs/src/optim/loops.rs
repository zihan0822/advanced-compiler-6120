use crate::analyzer::{dom::*, scc::*};
use crate::bril::LabelOrInst;
use crate::cfg::prelude::*;
use std::collections::HashSet;
use std::sync::{Arc, Mutex, Weak};

pub struct NaturalLoop<'a> {
    pub entry: NodeRef,
    pub comp: &'a CompRef,
    pub exits: Vec<NodeRef>,
    pub preheader: Option<NodeRef>,
}

pub fn find_natural_loops<'a>(cfg: &mut Cfg, comps: &'a Vec<CompRef>) -> Vec<NaturalLoop<'a>> {
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
            let entry_idx = cfg
                .nodes
                .iter()
                .position(|node| Arc::as_ptr(node) == Arc::as_ptr(&entry))
                .unwrap();
            assert!(validate_backedges(&entry, &comp.lock().unwrap(), &dom_tree));
            let mut natural_loop = NaturalLoop {
                entry,
                comp,
                exits,
                preheader: None,
            };
            let preheader_node = natural_loop.inject_preheader_node();
            // TODO: don't maintain the invariant here, change how we deserialize into bril::Prog
            cfg.nodes.insert(entry_idx, preheader_node);
            loops.push(natural_loop);
        }
    }
    loops
}

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
    visitor
        .backedges
        .into_iter()
        .all(|(src, dst)| dom_tree.is_dominator_of(dst, src))
}

impl<'a> NaturalLoop<'a> {
    fn inject_preheader_node(&mut self) -> NodeRef {
        let entry_label = self.entry.lock().unwrap().label.clone().unwrap();
        let preheader_label = format!("{}.preheader", entry_label);

        let (header_preds, preheader_node) = {
            let mut entry_lock = self.entry.lock().unwrap();
            // excluding in-component backedge
            let entry_preds: Vec<WeakNodeRef> = entry_lock
                    .predecessors
                    .iter()
                    .filter(|pred| !self.comp.lock().unwrap().contains(&Weak::as_ptr(pred)))
                    .cloned()
                    .collect();

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
                predecessors: entry_preds.clone()
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
        self.preheader = Some(preheader_node.clone());
        preheader_node
    }
}
