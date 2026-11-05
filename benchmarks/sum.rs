// `black_box` stops LLVM from folding the loop, which it does otherwise.
use std::hint::black_box;

fn main() {
    let limit = std::env::args().count() as i64 + 999999999;
    let mut total: i64 = 0;
    for i in 0..limit {
        total += black_box(i) * 3;
        total -= black_box(i) * 2;
    }
    println!("{total}");
}
