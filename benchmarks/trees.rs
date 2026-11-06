enum Tree {
    Leaf,
    Node(Box<Tree>, Box<Tree>),
}

fn build(depth: i64) -> Tree {
    if depth == 0 {
        return Tree::Leaf;
    }
    Tree::Node(Box::new(build(depth - 1)), Box::new(build(depth - 1)))
}

fn check(tree: &Tree) -> i64 {
    match tree {
        Tree::Leaf => 1,
        Tree::Node(left, right) => 1 + check(left) + check(right),
    }
}

fn main() {
    let max = std::env::args().count() as i64 + 17;
    let longest = build(max);
    let mut total = 0;
    for depth in 4..=max {
        if depth % 2 == 0 {
            let mut trees = 1;
            for _ in 0..max - depth + 4 {
                trees *= 2;
            }
            for _ in 0..trees {
                total += check(&build(depth));
            }
        }
    }
    println!("{total}");
    println!("{}", check(&longest));
}
