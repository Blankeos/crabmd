//! Source-buffer helpers. Motions run on the file bytes; paint re-parses.

use crate::document::splice;

/// Backspace at column 0 of a logical line in the full buffer.
///
/// - Mid-line: `None` (ordinary character delete).
/// - At a `\n\n` separator: delete both newlines so the next line is
///   concatenated onto the previous (`hello` + `## Quota` → `hello## Quota`).
/// - Otherwise delete the single preceding newline (in-buffer join).
pub fn backspace_join_doc(source: &str, offset: usize) -> Option<(String, usize)> {
    if offset == 0 || offset > source.len() {
        return None;
    }
    let line_start = source[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
    if offset != line_start {
        return None;
    }
    let bytes = source.as_bytes();
    if offset >= 2 && bytes[offset - 1] == b'\n' && bytes[offset - 2] == b'\n' {
        let caret = offset - 2;
        return Some((splice(source, caret..offset, ""), caret));
    }
    if bytes[offset - 1] == b'\n' {
        let skip = if offset >= 2 && bytes[offset - 2] == b'\r' {
            2
        } else {
            1
        };
        let caret = offset - skip;
        return Some((splice(source, caret..offset, ""), caret));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::{apply_motion, visual_line_range, Motion};

    #[test]
    fn backspace_at_heading_joins_previous_line() {
        let src = "hello\n\n## Quota and hsh\n";
        let at = src.find('#').unwrap();
        let (out, caret) = backspace_join_doc(src, at).expect("join");
        assert_eq!(out, "hello## Quota and hsh\n");
        assert_eq!(caret, 5);
        assert!(out.contains("hello## Quota"));
    }

    #[test]
    fn backspace_at_hash_heading_after_blank() {
        let src = "hello\n\n# H";
        let at = src.find('#').unwrap();
        let (out, caret) = backspace_join_doc(src, at).expect("join");
        assert_eq!(out, "hello# H");
        assert_eq!(caret, 5);
    }

    #[test]
    fn wrap_does_not_affect_hl() {
        let s = "abcdefghij\nxy";
        assert_eq!(apply_motion(s, 0, Motion::Right, 20, Some(4)), 10);
        assert_eq!(apply_motion(s, 11, Motion::Left, 9, Some(4)), 11);
        assert_eq!(apply_motion(s, 0, Motion::Left, 3, Some(4)), 0);
    }

    #[test]
    fn j_heading_then_paragraph() {
        let s = "# H\n\npara";
        let a = apply_motion(s, 0, Motion::Down, 1, None);
        assert_eq!(a, 4);
        let b = apply_motion(s, a, Motion::Down, 1, None);
        assert!(s[b..].starts_with("para"), "{:?}", &s[b..]);
    }

    #[test]
    fn v_is_one_logical_line() {
        let s = "# H\n\npara";
        let range = visual_line_range(s, 0);
        assert_eq!(&s[range.clone()], "# H\n");
        assert_eq!(range, 0..4);
    }
}
