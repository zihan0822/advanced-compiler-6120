function count_path(x: bigint, y: bigint): bigint {
   if (x == 0n)
           return 1n;
   if (y == 0n)
           return 1n;
   return count_path(x-1n,y) + count_path(x, y-1n) + count_path(x-1n, y-1n);
}


var n: bigint = 7n;
var ans = count_path(n, n); 
console.log(ans);


