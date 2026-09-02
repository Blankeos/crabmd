//! Document-level undo/redo. Snapshots of the live tree + source + caret.

use std::ops::Range;

use crate::mode::Mode;
use crate::tree::Doc;

const DEFAULT_CAP: usize = 100;

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub source: String,
    pub doc: Doc,
    pub caret: usize,
    pub sel: Option<Range<usize>>,
    pub mode: Mode,
}

impl PartialEq for Snapshot {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && self.caret == other.caret
            && self.sel == other.sel
            && self.mode == other.mode
            && self.doc.links == other.doc.links
            && self.doc.nodes.len() == other.doc.nodes.len()
            && self
                .doc
                .nodes
                .iter()
                .zip(&other.doc.nodes)
                .all(|(a, b)| a.kind == b.kind)
    }
}

impl Eq for Snapshot {}

impl Snapshot {
    pub fn new(source: &str, caret: usize, sel: Option<Range<usize>>, mode: Mode) -> Self {
        Self {
            source: source.to_string(),
            doc: Doc::from_gfm(source),
            caret,
            sel,
            mode,
        }
    }

    pub fn of(
        doc: &Doc,
        source: &str,
        caret: usize,
        sel: Option<Range<usize>>,
        mode: Mode,
    ) -> Self {
        Self {
            source: source.to_string(),
            doc: doc.clone(),
            caret,
            sel,
            mode,
        }
    }
}

#[derive(Debug)]
pub struct UndoStack {
    past: Vec<Snapshot>,
    future: Vec<Snapshot>,
    cap: usize,
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new(DEFAULT_CAP)
    }
}

impl UndoStack {
    pub fn new(cap: usize) -> Self {
        Self {
            past: Vec::new(),
            future: Vec::new(),
            cap: cap.max(1),
        }
    }

    pub fn push(&mut self, snap: Snapshot) {
        if self.past.last() == Some(&snap) {
            return;
        }
        if self.past.len() >= self.cap {
            self.past.remove(0);
        }
        self.past.push(snap);
        self.future.clear();
    }

    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let prev = self.past.pop()?;
        self.future.push(current);
        Some(prev)
    }

    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let next = self.future.pop()?;
        self.past.push(current);
        Some(next)
    }

    #[allow(dead_code)]
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    #[allow(dead_code)]
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    #[allow(dead_code)]
    pub fn len_past(&self) -> usize {
        self.past.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(n: usize) -> Snapshot {
        Snapshot::new(&format!("b{n}"), 0, None, Mode::Normal)
    }

    #[test]
    fn push_undo_redo() {
        let mut s = UndoStack::new(10);
        s.push(snap(1));
        s.push(snap(2));
        assert!(s.can_undo());
        assert!(!s.can_redo());
        let u = s.undo(snap(3)).unwrap();
        assert_eq!(u, snap(2));
        assert!(s.can_redo());
        let r = s.redo(snap(2)).unwrap();
        assert_eq!(r, snap(3));
    }

    #[test]
    fn cannot_undo_past_start() {
        let mut s = UndoStack::new(10);
        assert!(s.undo(snap(0)).is_none());
        s.push(snap(1));
        assert!(s.undo(snap(2)).is_some());
        assert!(s.undo(snap(1)).is_none());
        assert!(!s.can_undo());
    }

    #[test]
    fn push_clears_redo() {
        let mut s = UndoStack::new(10);
        s.push(snap(1));
        s.undo(snap(2));
        assert!(s.can_redo());
        s.push(snap(9));
        assert!(!s.can_redo());
    }

    #[test]
    fn cap_drops_oldest() {
        let mut s = UndoStack::new(2);
        s.push(snap(1));
        s.push(snap(2));
        s.push(snap(3));
        assert_eq!(s.len_past(), 2);
        let a = s.undo(snap(4)).unwrap();
        assert_eq!(a, snap(3));
        let b = s.undo(snap(3)).unwrap();
        assert_eq!(b, snap(2));
        assert!(s.undo(snap(2)).is_none());
    }

    #[test]
    fn skip_duplicate_push() {
        let mut s = UndoStack::new(10);
        s.push(snap(1));
        s.push(snap(1));
        assert_eq!(s.len_past(), 1);
    }

    #[test]
    fn same_source_different_tree_is_not_duplicate() {
        use crate::tree::NodeKind;

        let a = crate::tree::Doc::from_gfm("- bullet");
        let src = a.to_gfm();
        let mut b = a.clone();
        let len = b.project().display.len();
        b.tab_selection(0..len, false);
        assert_eq!(a.to_gfm(), b.to_gfm(), "normalized GFM stays a list");
        let NodeKind::List { items, .. } = &b.nodes[0].kind else {
            panic!();
        };
        assert_eq!(items[0].indent, 1);

        let mut s = UndoStack::new(10);
        s.push(Snapshot::of(&a, &src, 0, None, Mode::Normal));
        s.push(Snapshot::of(&a, &src, 0, None, Mode::Normal));
        assert_eq!(s.len_past(), 1, "identical tree is a duplicate");
        s.push(Snapshot::of(&b, &src, 0, None, Mode::Normal));
        assert_eq!(s.len_past(), 2, "indent-only change must be undoable");

        let mut s = UndoStack::new(10);
        s.push(Snapshot::of(&a, &src, 0, None, Mode::Normal));
        let prev = s
            .undo(Snapshot::of(&b, &src, 0, None, Mode::Normal))
            .unwrap();
        let NodeKind::List { items, .. } = &prev.doc.nodes[0].kind else {
            panic!("undo restored a list");
        };
        assert_eq!(items[0].indent, 0);
    }
}
