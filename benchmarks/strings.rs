use std::fmt::Write;

fn main() {
    let size = std::env::args().count() * 300000;
    let mut out = String::new();
    for i in 0..size {
        write!(out, "{i},").unwrap();
    }
    println!("{}", out.chars().count());
}
