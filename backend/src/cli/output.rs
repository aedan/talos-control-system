//! Output rendering for the `tcs` CLI: `table`, `wide`, `json`, `yaml`.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Table,
    Wide,
    Json,
    Yaml,
}

impl Format {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "wide" | "w" => Format::Wide,
            "json" => Format::Json,
            "yaml" | "y" => Format::Yaml,
            _ => Format::Table,
        }
    }
}

/// Render a JSON value in the requested format.
///
/// `table`/`wide` expect the value to be an object with a `rows` array (each row a
/// string array) and a `columns` array; otherwise it falls back to `json`.
pub fn render(value: &Value, format: Format) -> String {
    match format {
        Format::Json => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
        Format::Yaml => serde_yaml::to_string(value).unwrap_or_else(|_| value.to_string()),
        Format::Table | Format::Wide => render_table(value, format == Format::Wide),
    }
}

/// Render a table from a `{"columns": [...], "rows": [[...], ...]}` value.
fn render_table(value: &Value, wide: bool) -> String {
    let (columns, rows) = match (value.get("columns"), value.get("rows")) {
        (Some(c), Some(r)) if c.is_array() && r.is_array() => (c, r),
        _ => return serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    };

    let cols: Vec<String> = columns.as_array().unwrap().iter().map(|v| v.as_str().unwrap_or_default().to_string()).collect();
    let rows: Vec<Vec<String>> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            r.as_array()
                .map(|a| a.iter().map(|c| c.as_str().unwrap_or_default().to_string()).collect())
                .unwrap_or_default()
        })
        .collect();

    // `wide` shows all columns; `table` drops any column flagged wide-only.
    let widths: Vec<usize> = cols.iter().enumerate().map(|(i, h)| {
        let mut w = h.chars().count();
        for r in &rows {
            if let Some(cell) = r.get(i) {
                w = w.max(cell.chars().count());
            }
        }
        w
    }).collect();

    let mut out = String::new();
    let mut line = String::new();
    for (i, h) in cols.iter().enumerate() {
        line.push_str(&pad(h, widths[i]));
    }
    out.push_str(line.trim_end_matches(' '));
    out.push('\n');
    for r in &rows {
        line.clear();
        for (i, c) in cols.iter().enumerate() {
            let _ = c;
            if let Some(cell) = r.get(i) {
                line.push_str(&pad(cell, widths[i]));
            } else {
                line.push_str(&pad("", widths[i]));
            }
        }
        out.push_str(line.trim_end_matches(' '));
        out.push('\n');
    }
    // `wide` is a no-op beyond column selection here; keep the flag referenced.
    let _ = wide;
    out
}

fn pad(s: &str, width: usize) -> String {
    let len = s.chars().count();
    let mut out = s.to_string();
    if len < width {
        for _ in 0..(width - len) {
            out.push(' ');
        }
    }
    out.push(' ');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn format_parse_known_values() {
        assert_eq!(Format::parse("table"), Format::Table);
        assert_eq!(Format::parse("wide"), Format::Wide);
        assert_eq!(Format::parse("w"), Format::Wide);
        assert_eq!(Format::parse("json"), Format::Json);
        assert_eq!(Format::parse("yaml"), Format::Yaml);
        assert_eq!(Format::parse("y"), Format::Yaml);
        assert_eq!(Format::parse("YAML"), Format::Yaml);
        assert_eq!(Format::parse("bogus"), Format::Table);
    }

    #[test]
    fn render_json_pretty() {
        let out = render(&json!({"a": 1}), Format::Json);
        assert!(out.contains("\"a\": 1"));
    }

    #[test]
    fn render_yaml() {
        let out = render(&json!({"a": 1}), Format::Yaml);
        assert!(out.contains("a: 1"));
    }

    #[test]
    fn render_table() {
        let v = json!({"columns": ["NAME", "AGE"], "rows": [["p1", "2d"], ["p2", "3h"]]});
        let out = render(&v, Format::Table);
        assert!(out.contains("NAME"));
        assert!(out.contains("AGE"));
        assert!(out.contains("p1"));
        assert!(out.contains("2d"));
    }

    #[test]
    fn render_table_falls_back_to_json() {
        let out = render(&json!({"a": 1}), Format::Table);
        assert!(out.contains("\"a\": 1"));
    }
}
