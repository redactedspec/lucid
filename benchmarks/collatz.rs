fn main() {
    let limit = std::env::args().count() as i64 + 2999999;
    let mut longest = 0;
    for start in 1..limit {
        let mut n = start;
        let mut steps = 0;
        while n != 1 {
            if n.rem_euclid(2) == 0 {
                n = n.div_euclid(2);
            } else {
                n = 3 * n + 1;
            }
            steps += 1;
        }
        if steps > longest {
            longest = steps;
        }
    }
    println!("{longest}");
}
