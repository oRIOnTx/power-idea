//! Exporting result sets.

use super::ResultSet;

/// RFC 4180 CSV with a header row; NULL becomes an empty field.
pub fn to_csv(set: &ResultSet) -> String {
    fn field(out: &mut String, value: &str) {
        if value.contains([',', '"', '\n', '\r']) {
            out.push('"');
            out.push_str(&value.replace('"', "\"\""));
            out.push('"');
        } else {
            out.push_str(value);
        }
    }
    let mut out = String::new();
    let line = |out: &mut String, cells: &mut dyn Iterator<Item = &str>| {
        for (ix, cell) in cells.enumerate() {
            if ix > 0 {
                out.push(',');
            }
            field(out, cell);
        }
        out.push_str("\r\n");
    };
    line(&mut out, &mut set.columns.iter().map(String::as_str));
    for row in &set.rows {
        line(&mut out, &mut row.iter().map(|c| c.as_deref().unwrap_or("")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::to_csv;
    use crate::ResultSet;

    #[test]
    fn quotes_special_fields() {
        let set = ResultSet {
            columns: vec!["id".into(), "note".into()],
            rows: vec![
                vec![Some("1".into()), Some("a, \"b\"".into())],
                vec![Some("2".into()), None],
                vec![Some("3".into()), Some("x\ny".into())],
            ],
            truncated: false,
        };
        assert_eq!(
            to_csv(&set),
            "id,note\r\n1,\"a, \"\"b\"\"\"\r\n2,\r\n3,\"x\ny\"\r\n"
        );
    }
}
