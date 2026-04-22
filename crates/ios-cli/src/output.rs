use comfy_table::{Cell, Table};
use serde::Serialize;

/// Print a serializable value as JSON, writing to stderr on serialization failure.
#[allow(dead_code)]
pub fn print_json<T: Serialize>(value: &T, _json: bool) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("error: failed to serialize output: {e}"),
    }
}

/// Build a simple two-column key-value table.
#[allow(dead_code)]
pub fn kv_table(pairs: &[(&str, String)]) -> String {
    let mut table = Table::new();
    for (k, v) in pairs {
        table.add_row(vec![Cell::new(k), Cell::new(v)]);
    }
    table.to_string()
}
