use std::collections::HashMap;

enum Json {
    Null,
    Flag(bool),
    Num(i64),
    Str(String),
    Arr(Vec<Json>),
    Obj(HashMap<String, Json>),
}

fn skip(b: &[u8], at: &mut usize) {
    while b[*at] == b' ' || b[*at] == b'\n' {
        *at += 1;
    }
}

fn read_text(src: &str, b: &[u8], at: &mut usize) -> String {
    *at += 1;
    let start = *at;
    while b[*at] != b'"' {
        *at += 1;
    }
    *at += 1;
    src[start..*at - 1].to_string()
}

fn read_value(src: &str, b: &[u8], at: &mut usize) -> Json {
    skip(b, at);
    let c = b[*at];
    match c {
        b'"' => Json::Str(read_text(src, b, at)),
        b'[' => {
            *at += 1;
            let mut items = Vec::new();
            skip(b, at);
            if b[*at] == b']' {
                *at += 1;
                return Json::Arr(items);
            }
            loop {
                items.push(read_value(src, b, at));
                skip(b, at);
                *at += 1;
                if b[*at - 1] == b']' {
                    return Json::Arr(items);
                }
            }
        }
        b'{' => {
            *at += 1;
            let mut fields = HashMap::new();
            skip(b, at);
            if b[*at] == b'}' {
                *at += 1;
                return Json::Obj(fields);
            }
            loop {
                skip(b, at);
                let key = read_text(src, b, at);
                skip(b, at);
                *at += 1;
                let value = read_value(src, b, at);
                fields.insert(key, value);
                skip(b, at);
                *at += 1;
                if b[*at - 1] == b'}' {
                    return Json::Obj(fields);
                }
            }
        }
        b't' => {
            *at += 4;
            Json::Flag(true)
        }
        b'f' => {
            *at += 5;
            Json::Flag(false)
        }
        b'-' | b'0'..=b'9' => {
            let negative = c == b'-';
            if negative {
                *at += 1;
            }
            let mut total = 0i64;
            while b[*at].is_ascii_digit() {
                total = total * 10 + i64::from(b[*at] - b'0');
                *at += 1;
            }
            Json::Num(if negative { -total } else { total })
        }
        _ => {
            *at += 4;
            Json::Null
        }
    }
}

fn total(value: &Json) -> i64 {
    match value {
        Json::Null => 0,
        Json::Flag(on) => i64::from(*on),
        Json::Num(n) => *n,
        Json::Str(s) => s.chars().count() as i64,
        Json::Arr(items) => items.iter().map(total).sum(),
        Json::Obj(fields) => {
            fields.iter().map(|(key, value)| key.chars().count() as i64 + total(value)).sum()
        }
    }
}

fn main() {
    let record = "{\"id\": 12345, \"name\": \"Ada Lovelace\", \"active\": true, \"tags\": [\"math\", \"code\", \"notes\"], \"score\": -42, \"next\": null},\n";
    let count = std::env::args().count() * 20000;
    let doc = format!("[{}1]", record.repeat(count));
    let mut sum = 0;
    for _ in 0..5 {
        let mut at = 0;
        sum += total(&read_value(&doc, doc.as_bytes(), &mut at));
    }
    println!("{sum}");
}
