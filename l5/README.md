#### Global Analysis - Dominators, Dominance Tree and Dominance Frontier
This lesson covers iterative method for finding dominator set, constructing dominance tree for cfg and computing dominance frontier.
Source code can be found in [analyzer/dom.rs](https://github.com/zihan0822/advanced-compiler-6120/blob/main/bril-rs/src/analyzer/dom.rs)
and some design choices can be found in [discussion](https://github.com/sampsyo/cs6120/discussions/453)

#### How to run
```bash
$ cargo build --release
$ turnt -vp *.bril
```

#### Remarks on Definition of Dominance Frontier:
A dominance frontier is the set of nodes that are just “one edge away” from being dominated by a given node.
Put differently, `A`’s dominance frontier contains `B` iff `A` does not strictly dominate `B`, but `A` does dominate some predecessor of `B`.
- `A` dominates predecessor of `B`: remember that dominance is reflexive, so the predecessor of `B` can be `A` itself
- `B` iff `A` does not strictly dominate `B`: strict dominance is defined as: `A` dominates `B` and `A` is not equal to `B`, according to this, the dominance frontier
of `A` can contain `A` itself

Here is sample graphic output for benchmark [digit-root](https://github.com/sampsyo/bril/blob/main/benchmarks/core/digital-root.bril). Dominance tree is plotted alongside the original cfg graph.
The target node for which we are trying to find frontier is marked in red and its frontier is marked in green. 
![digit-root-dom-graph](https://github.com/zihan0822/advanced-compiler-6120/blob/main/l5/frontier.png)
