### Tracing JIT
I implemented a tracing jit in this. More implementation details can be found in [discussion](https://github.com/sampsyo/cs6120/discussions/458#discussioncomment-12941617) 
and the code can be found in my jit [bril fork](https://github.com/zihan0822/bril/tree/jit). 

I compared the performance of my tracing jit with the baseline brili implementation on bril `benchmarks/core`. Not perform that well tho. 

<img src="https://github.com/zihan0822/advanced-compiler-6120/blob/main/l12/jit_perf.png" width=80%>
