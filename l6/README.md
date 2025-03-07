#### Static-Single-Assignment (SSA)
In this lesson, we implemented a dominance free version of ssa tranformation, codes can be found in [src/transfrom/ssa.rs](https://github.com/zihan0822/advanced-compiler-6120/blob/main/bril-rs/src/transform/ssa.rs). 
Some disscussion of impl choices can be found in [discussion](https://github.com/sampsyo/cs6120/discussions/454).

#### How to run
```bash
$ cargo build --release
$ turnt -vp *.bril
$ brench brench.toml    # one should configure the bril folder accordingly when running this    
```

We compared the perf of our dom-free impl with the dom-based impl provided in bril [examples](https://github.com/sampsyo/bril/tree/main/examples)

The following is the relative increase of the number of dyn inst executed compared to baseline of two algos, there is no dce involved in between the round trip.
There seems to be a consistent decrease in the number of `set/get` inserted with our algo.
![round-trip-wo-dce](https://github.com/zihan0822/advanced-compiler-6120/blob/main/l6/ssa-round-trip-wo-dce.png)

In the setting where three [dce passes](https://github.com/sampsyo/bril/blob/main/examples/tdce.py) (run with `tdce+` flags) are inserted, the gap between those two algos is reduced a lot. However, for some benchmarks, we can still
see promising improvement brought by dom-free ssa transform. 
![round-trip-w-dce](https://github.com/zihan0822/advanced-compiler-6120/blob/main/l6/ssa-round-trip-w-dce.png)
