#### DataFlow Analysis
We implement a [generic worklist algorithm solver](https://github.com/zihan0822/advanced-compiler-6120/blob/main/bril-rs/src/optim/dflow.rs) and some use cases of it, including
[Liveness Analysis](https://github.com/zihan0822/advanced-compiler-6120/blob/main/bril-rs/src/optim/dce/global.rs), [Global Const Propagation](https://github.com/zihan0822/advanced-compiler-6120/blob/main/bril-rs/src/analyzer/mod.rs) and [Initialization Detection](https://github.com/zihan0822/advanced-compiler-6120/blob/main/bril-rs/src/analyzer/mod.rs).
A bit of implementation detail and choice can be found in [discussion](https://github.com/sampsyo/cs6120/discussions/452)

#### How to run
```bash
$ cargo build --release
$ turnt -e uninit/const_prop examples/*bril   # some could also run this against bril/benchmarks/core
```

The following is a sample output from our initailization(declaration in bril) detector of a malformed bril program
```
@main
Label: .
line 0: a
line 1: b
Label: .L
line 1: c      // dest c is computed from some undeclared vars, it is located on line 1 of block .L
```

With data flow analysis, we are able to see great improvements on reduction of dyn inst executed brought by global dce compared with our pure local version in l3.
Some could see our updated result with
```
$ cd l3
$ cargo build --release
$ brench brench.toml
```
![dce_benchmark](https://github.com/zihan0822/advanced-compiler-6120/blob/main/l4/benchmark.png)
