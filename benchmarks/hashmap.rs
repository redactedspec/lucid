use std::collections::HashMap;

fn main() {
    let size = std::env::args().count() as i64 * 1000000;
    let mut numbers: HashMap<i64, i64> = HashMap::new();
    for i in 0..size {
        numbers.insert(i * 7, i);
    }
    let mut found = 0;
    for i in 0..size * 2 {
        if let Some(value) = numbers.get(&i) {
            found += value;
        }
    }
    for i in 0..size {
        if i % 2 == 0 {
            numbers.remove(&(i * 7));
        }
    }
    println!("{found}");
    println!("{}", numbers.len());

    let mut words: HashMap<String, i64> = HashMap::new();
    for i in 0..size / 4 {
        words.insert(format!("key{}", i % 5000), i);
    }
    let mut hits = 0;
    for i in 0..size {
        let word = format!("key{}", i % 10000);
        if words.contains_key(&word) {
            hits += 1;
        }
    }
    println!("{hits}");
}
