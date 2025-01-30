var n: bigint = 3n;
nested_if_else(n);
function nested_if_else(n: bigint) {
    var a: bigint = 3n;
    if (n < 10n) {
        if (n > 5n) {
            n = n + 1n;
        } else {
            n = n * 10n;
        }
    } else {
        if (n > 20n) {
            n = n * 100n;
        } else {
            n = n;
        }
    }
    console.log(n)
}