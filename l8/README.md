#### Loop Invariant Code Motion (LICM)

In this task, we implemented licm of bril in SSA form. Impl can be found in [optim/loops.rs](https://github.com/zihan0822/advanced-compiler-6120/blob/main/bril-rs/src/optim/loops.rs)
and some design choice can be found in [discussion](https://github.com/sampsyo/cs6120/discussions/456).

##### How to run
``` bash
$ cargo b --release
$ brench brench.toml
```
We compare the performance (evaluated as the number of dyn instr executed) of bril program after SSA round-trip (shown as `baseline`) and after licm pass on `bril/benchmarks/core`. Dce passes are inserted
in the same manner for those two passes.
![licm](https://github.com/zihan0822/advanced-compiler-6120/blob/main/l8/licm.png).
