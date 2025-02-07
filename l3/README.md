### Dead-Code-Elimination & Local-Value-Numbering

A brief summary of implementation choices can be found in [disscussion thread](https://github.com/sampsyo/cs6120/discussions/451). Impl can be found in [optim module](https://github.com/zihan0822/advanced-compiler-6120/tree/main/bril-rs/src/optim).
LVN implementation supports **copy propagation**, **commutativity exploit** and **const folding**


#### How to run
```shell
$ cargo build --release
$ turnt -vp examples/*.bril    # some may also try to test this against bril/benchmarks
```

This optimizer is a pure local optimizer with no knowledge of the behavior of other basic blocks in the CFG and the position of current block in the global CFG (eg. whether it's a leaf node or not).
Here we discuss some of conservative assumptions we made to ensure the correctness of this local optimizer in global context
- **live-on-exit variables**: we always assume that live-on-exit variables will be used somewhere else in other basic blocks. Under this assumption, DCE could only run in rare circumstances, for example
```
a: int = 5;    # 1
a: int = 10;   # 2
b: int = a;    # 3
print b;       # 4
```
in this code, only `#1` will be eliminated, `#3` will be const folded into `b: int = 10`. `#2` will only be deleted if current block does not have any successor in cfg 

- **live-on-entry variables**: optimizer will not run on the entire block if any live-on-entry variable is re-assigned within the block. For example
```
a: int = id z;
b: int = id a;
...
z: int = const 5;
print b;
```
We can not rename first `z` because it comes from ancestor blocks nor the second `z` because it will be potentially used by descendant. The general algorithm we implemented can not handle this case, so
we choose to disable it for the entire block

- **function call**: we introduce a new numbering for every return value of a function call even if all the numbering of its arguments are the same