use std::{env, fs, path::PathBuf};

// Compile the same declarations as the engine, without parsing Rust source.
macro_rules! params {
    ($($id:ident = $def:expr, $min:expr, $max:expr;)*) => {
        const DEFS: &[(&str, i32, i32, i32)] = &[$((stringify!($id), $def, $min, $max)),*];
    };
}
include!("src/param_defs.rs");

fn main() {
    println!("cargo:rerun-if-changed=src/param_defs.rs");
    println!("cargo:rerun-if-changed=frozen.options");
    if env::var_os("CARGO_FEATURE_FROZEN").is_none() {
        return;
    }
    let input = fs::read_to_string("frozen.options").expect("read frozen.options");
    let mut values: Vec<i32> = DEFS.iter().map(|d| d.1).collect();
    for (line, text) in input.lines().enumerate() {
        let text = text.trim();
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        let (name, value) = text.split_once('=').unwrap_or_else(|| {
            panic!("frozen.options:{}: expected Name=value", line + 1)
        });
        let i = DEFS.iter().position(|d| d.0.eq_ignore_ascii_case(name.trim()))
            .unwrap_or_else(|| panic!("frozen.options:{}: unknown parameter {}", line + 1, name));
        let value: i32 = value.trim().parse().unwrap_or_else(|_| {
            panic!("frozen.options:{}: invalid integer", line + 1)
        });
        let (name, _, min, max) = DEFS[i];
        assert!((min..=max).contains(&value),
            "frozen.options:{}: {name}={value} outside {min}..={max}", line + 1);
        values[i] = value;
    }
    let output = format!("const FROZEN_VALUES: [i32; DEFS.len()] = {values:?};\n");
    fs::write(PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("frozen_values.rs"), output)
        .expect("write frozen parameter table");
}
