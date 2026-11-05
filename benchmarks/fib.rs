fn fib(n: i64) -> i64 {
    if n < 2 {
        return n;
    }
    fib(n - 1) + fib(n - 2)
}

fn main() {
    let n = std::env::args().count() as i64 + 39;
    println!("{}", fib(n));
}
