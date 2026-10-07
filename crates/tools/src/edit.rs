//! Changing part of a text: replace an exact string (once, or all), or
//! append / insert / delete by lines or an exact passage (a passage must be
//! there exactly once). Lines count from 1.

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    /// `old` becomes `new` (the default when `old` is given).
    Replace,
    /// `text` at the end, on a line of its own.
    Append,
    /// `text` before line `line`, or right after the passage `after`.
    Insert,
    /// Lines `line` to `to`, or the passage `text`.
    Delete,
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
pub struct Edit {
    /// replace (with old/new), append, insert or delete.
    pub op: Option<Op>,
    /// replace: the exact text to replace (must be there once, unless `all`).
    pub old: Option<String>,
    /// replace: what it becomes.
    pub new: Option<String>,
    /// replace: every occurrence.
    #[serde(default)]
    pub all: bool,
    /// append, insert: what to add; delete: the exact passage to take out.
    pub text: Option<String>,
    /// insert: the line it goes before (one past the last: the end); delete: the first line.
    pub line: Option<usize>,
    /// delete: the last line (default: `line`).
    pub to: Option<usize>,
    /// insert: the exact passage it goes right after.
    pub after: Option<String>,
}

fn find_once(body: &str, passage: &str) -> Result<usize, String> {
    if passage.is_empty() {
        return Err("an empty passage".into());
    }
    let mut at = body.match_indices(passage).map(|(i, _)| i);
    match (at.next(), at.next()) {
        (Some(i), None) => Ok(i),
        (None, _) => Err(format!("{:?} isn't in it (the text must match exactly, spaces too)", short(passage))),
        (Some(_), Some(_)) => Err(format!("{:?} is in it {} times: give more of it so it's unique, or all = true", short(passage), body.matches(passage).count())),
    }
}

fn short(s: &str) -> String {
    if s.chars().count() <= 80 { s.to_string() } else { format!("{}…", s.chars().take(80).collect::<String>()) }
}

fn line_start(body: &str, n: usize) -> Result<usize, String> {
    let lines = body.lines().count();
    if n == 0 || n > lines + 1 {
        return Err(format!("line {n}: it has lines 1 to {lines}"));
    }
    Ok(body.split_inclusive('\n').take(n - 1).map(str::len).sum())
}

/// `body` changed by `e`.
pub fn apply(body: &str, e: &Edit) -> Result<String, String> {
    let op = e.op.unwrap_or(if e.old.is_some() { Op::Replace } else { Op::Append });
    let text = || e.text.as_deref().filter(|t| !t.is_empty()).ok_or_else(|| format!("{op:?} needs text").to_lowercase());
    match op {
        Op::Replace => {
            let old = e.old.as_deref().ok_or("replace needs old")?;
            let new = e.new.as_deref().ok_or("replace needs new")?;
            if e.all {
                if old.is_empty() || !body.contains(old) {
                    return Err(format!("{:?} isn't in it", short(old)));
                }
                return Ok(body.replace(old, new));
            }
            let at = find_once(body, old)?;
            Ok(format!("{}{new}{}", &body[..at], &body[at + old.len()..]))
        }
        Op::Append => {
            let t = text()?;
            Ok(if body.is_empty() || body.ends_with('\n') { format!("{body}{t}") } else { format!("{body}\n{t}") })
        }
        Op::Insert => {
            let t = text()?;
            match (e.line, e.after.as_deref()) {
                (Some(n), None) => {
                    let (head, tail) = body.split_at(line_start(body, n)?);
                    let sep = if head.is_empty() || head.ends_with('\n') { "" } else { "\n" };
                    let end = if tail.is_empty() || t.ends_with('\n') { "" } else { "\n" };
                    Ok(format!("{head}{sep}{t}{end}{tail}"))
                }
                (None, Some(p)) => {
                    let at = find_once(body, p)? + p.len();
                    Ok(format!("{}{t}{}", &body[..at], &body[at..]))
                }
                _ => Err("insert goes before a line or after a passage (one of them)".into()),
            }
        }
        Op::Delete => match (e.line, e.text.as_deref()) {
            (Some(n), None) => {
                let to = e.to.unwrap_or(n);
                let lines = body.lines().count();
                if to < n || to > lines {
                    return Err(format!("lines {n} to {to}: it has lines 1 to {lines}"));
                }
                let (from, until) = (line_start(body, n)?, line_start(body, to + 1)?);
                let mut out = format!("{}{}", &body[..from], &body[until..]);
                if until == body.len() && !body.ends_with('\n') && out.ends_with('\n') {
                    out.pop();
                }
                Ok(out)
            }
            (None, Some(p)) => {
                let at = find_once(body, p)?;
                Ok(format!("{}{}", &body[..at], &body[at + p.len()..]))
            }
            _ => Err("delete takes lines (line, to) or a passage (text): one of them".into()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed(op: Op) -> Edit {
        Edit { op: Some(op), ..Default::default() }
    }

    #[test]
    fn replaces_appends_inserts_and_deletes() {
        let b = "one\ntwo\nthree";
        assert_eq!(apply(b, &Edit { old: Some("two".into()), new: Some("2".into()), ..Default::default() }).unwrap(), "one\n2\nthree");
        assert_eq!(apply("a a", &Edit { old: Some("a".into()), new: Some("b".into()), all: true, ..Default::default() }).unwrap(), "b b");
        assert!(apply("a a", &Edit { old: Some("a".into()), new: Some("b".into()), ..Default::default() }).unwrap_err().contains("2 times"));
        assert_eq!(apply(b, &Edit { text: Some("four".into()), ..ed(Op::Append) }).unwrap(), "one\ntwo\nthree\nfour");
        assert_eq!(apply(b, &Edit { text: Some("zero".into()), line: Some(1), ..ed(Op::Insert) }).unwrap(), "zero\none\ntwo\nthree");
        assert_eq!(apply(b, &Edit { text: Some("four".into()), line: Some(4), ..ed(Op::Insert) }).unwrap(), "one\ntwo\nthree\nfour");
        assert_eq!(apply(b, &Edit { text: Some("!".into()), after: Some("two".into()), ..ed(Op::Insert) }).unwrap(), "one\ntwo!\nthree");
        assert_eq!(apply(b, &Edit { line: Some(2), to: Some(3), ..ed(Op::Delete) }).unwrap(), "one");
        assert_eq!(apply(b, &Edit { text: Some("two\n".into()), ..ed(Op::Delete) }).unwrap(), "one\nthree");
    }

    #[test]
    fn says_what_is_wrong() {
        assert!(apply("a", &Edit { old: Some("z".into()), new: Some("".into()), ..Default::default() }).unwrap_err().contains("isn't in it"));
        assert!(apply("a\nb", &Edit { line: Some(2), to: Some(5), ..ed(Op::Delete) }).unwrap_err().contains("lines 1 to 2"));
        assert!(apply("a", &Edit { line: Some(1), ..ed(Op::Insert) }).unwrap_err().contains("needs text"));
        assert!(apply("a", &ed(Op::Delete)).is_err());
    }
}
