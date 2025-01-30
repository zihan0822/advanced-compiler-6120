var pieces: number = 10;
var res: number = hanoi_solver(1, 2, 3, pieces);
console.log(res);

function hanoi_solver(src: number, dst: number, helper: number, n: number): number {
	if (n == 0)
	   return 0;
        let step1: number = hanoi_solver(src, helper, dst, n-1);
	let step2: number = 1;
	let step3: number = hanoi_solver(helper, dst, src, n-1);
	return step1 + step2 + step3;
}

