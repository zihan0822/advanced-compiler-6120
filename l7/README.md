#### LLVM Pass
In this lesson, we implemented a heap instrument pass that replaces original malloc/free call with a customzied version to keep track of the heap statistics per function.
More implementation details can be found in [discussion](https://github.com/sampsyo/cs6120/discussions/455)

##### How to run
adapted from [llvm-pass-skeleton](https://github.com/sampsyo/llvm-pass-skeleton)

```bash
$ mkdir build
$ cd build && cmake ..
$ make    // compile llvm plugin

$ cargo build // compile rust crate

$ clang -fpass-plugin=build/skeleton/HeapHookPass.* \
      <c program path> -L<rust lib path> -lhooked_heap // put it together
```
Example output from `a.c`
```
main: FuncAllocStat {
    num_alloc: 1,
    total_alloc_size: 4,
}
one_step_in: FuncAllocStat {
    num_alloc: 1,
    total_alloc_size: 8,
}
```
