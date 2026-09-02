//! Structured block document (ProseMirror / Tiptap / Notion model).
//!
//! The live document is a tree of nodes + inline runs with marks.
//! GFM is import (`from_gfm`) and export (`to_gfm`) only.

use std::ops::Range;

use crate::display::{
    project as project_gfm, Affinity, BlockExtra, ListItem as ProjListItem, Marks, ProjBlock,
    Projection, Segment, TableCell,
};
use crate::document::{next_id, AlertKind, BlockKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Bold,
    Italic,
    Strike,
    Code,
    Underline,
}

impl Mark {
    pub fn has(self, marks: Marks) -> bool {
        match self {
            Self::Bold => marks.bold,
            Self::Italic => marks.italic,
            Self::Strike => marks.strike,
            Self::Code => marks.code,
            Self::Underline => marks.underline,
        }
    }

    pub fn wrap(self) -> (&'static str, &'static str) {
        match self {
            Self::Bold => ("**", "**"),
            Self::Italic => ("*", "*"),
            Self::Strike => ("~~", "~~"),
            Self::Code => ("`", "`"),
            Self::Underline => ("<u>", "</u>"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inline {
    pub text: String,
    pub marks: Marks,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    pub indent: usize,
    pub checked: Option<bool>,
    pub inlines: Vec<Inline>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Paragraph {
        inlines: Vec<Inline>,
    },
    Heading {
        level: u8,
        inlines: Vec<Inline>,
    },
    Quote {
        inlines: Vec<Inline>,
    },
    Alert {
        kind: AlertKind,
        inlines: Vec<Inline>,
    },
    List {
        ordered: bool,
        items: Vec<ListItem>,
    },
    Code {
        lang: String,
        text: String,
    },
    Table {
        headers: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    Rule,
    Image {
        alt: String,
        src: String,
    },
    Html {
        raw: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub id: u64,
    pub kind: NodeKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doc {
    pub nodes: Vec<Node>,
}

#[derive(Clone, Copy, Debug)]
struct Loc {
    node: usize,
    item: Option<usize>,
    cell: Option<(usize, usize)>,
    offset: usize,
}

impl Doc {
    pub fn empty() -> Self {
        Self {
            nodes: vec![Node {
                id: next_id(),
                kind: NodeKind::Paragraph { inlines: vec![] },
            }],
        }
    }

    pub fn from_gfm(src: &str) -> Self {
        if src.trim().is_empty() {
            // Match GFM projection: `""`/`"\n"` → 1 empty, `"\n\n"` → 2, `"\n\n\n"` → 3.
            let newlines = src.as_bytes().iter().filter(|&&b| b == b'\n').count();
            let n = match newlines {
                0 | 1 => 1,
                n => n,
            };
            let mut nodes = Vec::with_capacity(n);
            for _ in 0..n {
                nodes.push(Node {
                    id: next_id(),
                    kind: NodeKind::Paragraph { inlines: vec![] },
                });
            }
            return Self { nodes };
        }
        let p = project_gfm(src);
        let mut nodes = Vec::with_capacity(p.blocks.len());
        for b in &p.blocks {
            nodes.push(node_from_proj(&p, b, src));
        }
        if nodes.is_empty() {
            return Self::empty();
        }
        Self { nodes }
    }

    pub fn project(&self) -> Projection {
        flatten(self)
    }

    pub fn to_gfm(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for n in &self.nodes {
            parts.push(node_to_gfm(n));
        }
        join_gfm(&self.nodes, &parts)
    }

    pub fn insert_text(
        &mut self,
        caret: usize,
        sel: Option<Range<usize>>,
        text: &str,
        sticky: Marks,
    ) -> usize {
        if let Some(sel) = sel.filter(|s| s.start != s.end) {
            let a = sel.start.min(sel.end);
            let b = sel.end.max(sel.start);
            self.delete_display(a..b);
            return self.insert_text(a, None, text, sticky);
        }
        if text.is_empty() {
            return caret;
        }
        let loc = self.loc(caret);
        let code_off = match &mut self.nodes[loc.node].kind {
            NodeKind::Code { text: body, .. } => {
                let at = loc.offset.min(body.len());
                body.insert_str(at, text);
                Some(at + text.len())
            }
            NodeKind::Html { raw } => {
                raw.push_str(text);
                Some(raw.len())
            }
            _ => None,
        };
        if let Some(off) = code_off {
            return self.caret_after(loc.node, None, None, off);
        }
        if matches!(
            self.nodes[loc.node].kind,
            NodeKind::Rule | NodeKind::Image { .. }
        ) {
            let ix = loc.node;
            self.nodes.insert(
                ix + 1,
                Node {
                    id: next_id(),
                    kind: NodeKind::Paragraph {
                        inlines: vec![Inline {
                            text: text.to_string(),
                            marks: sticky,
                        }],
                    },
                },
            );
            return self.caret_after(ix + 1, None, None, text.len());
        }
        let inlines = self.inlines_at_mut(loc);
        insert_inlines(inlines, loc.offset, text, sticky);
        let node_ix = loc.node;
        let item = loc.item;
        self.maybe_shortcut(node_ix, item);
        let loc = {
            // offset may have been consumed by a shortcut
            let inlines = self.inlines_at(Loc {
                node: node_ix,
                item,
                cell: loc.cell,
                offset: 0,
            });
            let len = inlines_len(inlines);
            Loc {
                node: node_ix,
                item,
                cell: loc.cell,
                offset: len.min(loc.offset + text.len()),
            }
        };
        self.caret_after(loc.node, loc.item, loc.cell, loc.offset)
    }

    pub fn delete_display(&mut self, range: Range<usize>) -> usize {
        let a = range.start.min(range.end);
        let b = range.end.max(range.start);
        if a == b {
            return a;
        }
        let start = self.loc(a);
        let end = self.loc(b.saturating_sub(1));
        if start.node == end.node && start.item == end.item && start.cell == end.cell {
            let off0 = start.offset;
            let off1 = {
                let loc_b = self.loc(b);
                if loc_b.node == start.node && loc_b.item == start.item && loc_b.cell == start.cell {
                    loc_b.offset
                } else {
                    inlines_len(self.inlines_at(start))
                }
            };
            match &mut self.nodes[start.node].kind {
                NodeKind::Code { text, .. } => {
                    let s = off0.min(text.len());
                    let e = off1.min(text.len()).max(s);
                    text.replace_range(s..e, "");
                }
                _ => {
                    let inlines = self.inlines_at_mut(start);
                    delete_inlines(inlines, off0, off1);
                }
            }
            return self.caret_after(start.node, start.item, start.cell, off0);
        }
        // Multi-node: delete within start, drop middle nodes, delete prefix of end, maybe merge.
        self.delete_display(a..self.node_end(start.node, start.item));
        let mut i = start.node + 1;
        while i < end.node {
            self.nodes.remove(i);
            // end.node shifts
        }
        let end = self.loc(b.saturating_sub(1).min(self.project().display.len().saturating_sub(1)));
        if end.node > start.node {
            let end_off = self.loc(b).offset;
            match &mut self.nodes[end.node].kind {
                NodeKind::Code { text, .. } => {
                    let e = end_off.min(text.len());
                    text.replace_range(0..e, "");
                }
                _ => {
                    let loc = Loc {
                        node: end.node,
                        item: end.item,
                        cell: end.cell,
                        offset: 0,
                    };
                    let inlines = self.inlines_at_mut(loc);
                    delete_inlines(inlines, 0, end_off);
                }
            }
            self.merge_nodes(start.node, end.node);
        }
        self.caret_after(start.node, start.item, start.cell, start.offset)
    }

    pub fn delete_char(&mut self, caret: usize) -> usize {
        if caret == 0 {
            return 0;
        }
        let p = self.project();
        let d = caret.min(p.display.len());
        let prev = p.display[..d]
            .chars()
            .next_back()
            .map(|c| d - c.len_utf8())
            .unwrap_or(0);
        self.delete_display(prev..d)
    }

    pub fn enter(&mut self, caret: usize, hard: bool) -> usize {
        let loc = self.loc(caret);
        match &self.nodes[loc.node].kind {
            NodeKind::Code { .. } => {
                return self.insert_text(caret, None, "\n", Marks::default());
            }
            NodeKind::List { .. } => return self.enter_list(loc),
            NodeKind::Table { .. } => return caret,
            NodeKind::Rule | NodeKind::Image { .. } => {
                self.nodes.insert(
                    loc.node + 1,
                    Node {
                        id: next_id(),
                        kind: NodeKind::Paragraph { inlines: vec![] },
                    },
                );
                return self.caret_after(loc.node + 1, None, None, 0);
            }
            _ => {}
        }
        if hard {
            return self.insert_text(caret, None, "\n", Marks::default());
        }
        // Notion: Enter at the start of a heading inserts an empty block above
        // and keeps the heading text (does not demote the body to a paragraph).
        // Caret stays on the heading (`|Table`), not on the new empty line.
        if loc.offset == 0 {
            if matches!(self.nodes[loc.node].kind, NodeKind::Heading { .. }) {
                let ni = loc.node;
                self.nodes.insert(
                    ni,
                    Node {
                        id: next_id(),
                        kind: NodeKind::Paragraph { inlines: vec![] },
                    },
                );
                return self.caret_after(ni + 1, None, None, 0);
            }
        }
        let inlines = self.inlines_at(loc).to_vec();
        let (left, right) = split_inlines(&inlines, loc.offset);
        let empty_right = inlines_len(&right) == 0;
        match &mut self.nodes[loc.node].kind {
            NodeKind::Heading { inlines, .. } => *inlines = left,
            NodeKind::Quote { inlines } | NodeKind::Alert { inlines, .. } => {
                if empty_right && loc.offset == inlines_len(inlines) {
                    // exit quote/alert
                } else {
                    *inlines = left;
                    self.nodes.insert(
                        loc.node + 1,
                        Node {
                            id: next_id(),
                            kind: NodeKind::Paragraph { inlines: right },
                        },
                    );
                    return self.caret_after(loc.node + 1, None, None, 0);
                }
            }
            NodeKind::Paragraph { inlines } => *inlines = left,
            _ => {}
        }
        // Convert heading/quote at end-enter into a following paragraph.
        let ni = loc.node;
        if matches!(
            self.nodes[ni].kind,
            NodeKind::Heading { .. } | NodeKind::Quote { .. } | NodeKind::Alert { .. }
        ) && empty_right
        {
            if matches!(self.nodes[ni].kind, NodeKind::Quote { .. } | NodeKind::Alert { .. })
                && inlines_len(self.inlines_at(loc)) == 0
            {
                self.nodes[ni].kind = NodeKind::Paragraph { inlines: vec![] };
                return self.caret_after(ni, None, None, 0);
            }
        }
        self.nodes.insert(
            ni + 1,
            Node {
                id: next_id(),
                kind: NodeKind::Paragraph { inlines: right },
            },
        );
        self.caret_after(ni + 1, None, None, 0)
    }

    fn enter_list(&mut self, loc: Loc) -> usize {
        let NodeKind::List { items, ordered } = &mut self.nodes[loc.node].kind else {
            return 0;
        };
        let ix = loc.item.unwrap_or(0).min(items.len().saturating_sub(1));
        let empty = inlines_len(&items[ix].inlines) == 0;
        if empty {
            let following: Vec<ListItem> = items.drain(ix + 1..).collect();
            items.remove(ix);
            let ordered = *ordered;
            let ni = loc.node;
            if items.is_empty() {
                self.nodes[ni].kind = NodeKind::Paragraph { inlines: vec![] };
                if !following.is_empty() {
                    self.nodes.insert(
                        ni + 1,
                        Node {
                            id: next_id(),
                            kind: NodeKind::List {
                                ordered,
                                items: following,
                            },
                        },
                    );
                }
                return self.caret_after(ni, None, None, 0);
            }
            self.nodes.insert(
                ni + 1,
                Node {
                    id: next_id(),
                    kind: NodeKind::Paragraph { inlines: vec![] },
                },
            );
            if !following.is_empty() {
                self.nodes.insert(
                    ni + 2,
                    Node {
                        id: next_id(),
                        kind: NodeKind::List {
                            ordered,
                            items: following,
                        },
                    },
                );
            }
            return self.caret_after(ni + 1, None, None, 0);
        }
        let (left, right) = split_inlines(&items[ix].inlines, loc.offset);
        items[ix].inlines = left;
        let indent = items[ix].indent;
        let checked = items[ix].checked.map(|_| false);
        items.insert(
            ix + 1,
            ListItem {
                indent,
                checked,
                inlines: right,
            },
        );
        self.caret_after(loc.node, Some(ix + 1), None, 0)
    }

    pub fn backspace(&mut self, caret: usize) -> Option<usize> {
        let loc = self.loc(caret);
        if loc.offset > 0 {
            return None;
        }
        match &self.nodes[loc.node].kind {
            NodeKind::List { .. } => return Some(self.backspace_list(loc)),
            NodeKind::Heading { .. }
            | NodeKind::Quote { .. }
            | NodeKind::Alert { .. }
            | NodeKind::Code { .. } => {
                if loc.node > 0 {
                    // Empty block above a heading: drop the empty slot and keep
                    // the heading (`# Table`). Merging into an empty paragraph
                    // would demote the heading to plain text.
                    let prev_empty = matches!(
                        &self.nodes[loc.node - 1].kind,
                        NodeKind::Paragraph { inlines } if inlines_len(inlines) == 0
                    );
                    if prev_empty && matches!(self.nodes[loc.node].kind, NodeKind::Heading { .. }) {
                        let heading = loc.node;
                        self.nodes.remove(loc.node - 1);
                        return Some(self.caret_after(heading - 1, None, None, 0));
                    }
                    return Some(self.merge_nodes(loc.node - 1, loc.node));
                }
                let inlines = match &self.nodes[loc.node].kind {
                    NodeKind::Heading { inlines, .. }
                    | NodeKind::Quote { inlines }
                    | NodeKind::Alert { inlines, .. } => inlines.clone(),
                    NodeKind::Code { text, .. } => vec![Inline {
                        text: text.clone(),
                        marks: Marks::default(),
                    }],
                    _ => vec![],
                };
                self.nodes[loc.node].kind = NodeKind::Paragraph { inlines };
                return Some(self.caret_after(loc.node, None, None, 0));
            }
            NodeKind::Paragraph { inlines } => {
                if inlines_len(inlines) == 0 {
                    if loc.node == 0 {
                        return Some(0);
                    }
                    self.nodes.remove(loc.node);
                    let prev = loc.node - 1;
                    let len = self.node_text_len(prev);
                    return Some(self.caret_after(prev, self.last_item(prev), None, len));
                }
                if loc.node == 0 {
                    return None;
                }
                return Some(self.merge_nodes(loc.node - 1, loc.node));
            }
            NodeKind::Rule | NodeKind::Image { .. } | NodeKind::Html { .. } => {
                if loc.node == 0 {
                    self.nodes[0].kind = NodeKind::Paragraph { inlines: vec![] };
                    return Some(0);
                }
                self.nodes.remove(loc.node);
                let prev = loc.node - 1;
                let len = self.node_text_len(prev);
                return Some(self.caret_after(prev, self.last_item(prev), None, len));
            }
            NodeKind::Table { .. } => {
                if loc.node > 0 {
                    return Some(self.merge_nodes(loc.node - 1, loc.node));
                }
            }
        }
        None
    }

    fn backspace_list(&mut self, loc: Loc) -> usize {
        let NodeKind::List { items, ordered } = &mut self.nodes[loc.node].kind else {
            return 0;
        };
        let ix = loc.item.unwrap_or(0);
        if items[ix].indent > 0 {
            items[ix].indent -= 1;
            return self.caret_after(loc.node, Some(ix), None, 0);
        }
        let inlines = items[ix].inlines.clone();
        let following: Vec<ListItem> = items.drain(ix + 1..).collect();
        items.remove(ix);
        let ordered = *ordered;
        let ni = loc.node;
        if items.is_empty() {
            self.nodes[ni].kind = NodeKind::Paragraph { inlines };
            if !following.is_empty() {
                self.nodes.insert(
                    ni + 1,
                    Node {
                        id: next_id(),
                        kind: NodeKind::List {
                            ordered,
                            items: following,
                        },
                    },
                );
            }
            return self.caret_after(ni, None, None, 0);
        }
        self.nodes.insert(
            ni + 1,
            Node {
                id: next_id(),
                kind: NodeKind::Paragraph { inlines },
            },
        );
        if !following.is_empty() {
            self.nodes.insert(
                ni + 2,
                Node {
                    id: next_id(),
                    kind: NodeKind::List {
                        ordered,
                        items: following,
                    },
                },
            );
        }
        self.caret_after(ni + 1, None, None, 0)
    }

    pub fn toggle_mark(&mut self, sel: Range<usize>, mark: Mark) -> Option<Range<usize>> {
        let a = sel.start.min(sel.end);
        let b = sel.end.max(sel.start);
        if a >= b {
            return None;
        }
        let p = self.project();
        let all = (a..b).all(|i| mark.has(p.marks_at(i, Affinity::Inside)));
        let any = (a..b).any(|i| mark.has(p.marks_at(i, Affinity::Inside)));
        let wordish = p
            .display
            .get(a..b)
            .is_some_and(|s| !s.chars().any(char::is_whitespace));
        // Partially-marked contiguous words clear instead of stacking.
        let turn_on = !(all || (any && wordish));
        let loc = self.loc(a);
        let loc_b = self.loc(b);
        if loc.node != loc_b.node || loc.item != loc_b.item {
            return None;
        }
        let inlines = self.inlines_at_mut(loc);
        apply_mark_range(inlines, loc.offset, loc_b.offset, mark, turn_on);
        Some(a..b)
    }

    pub fn apply_slash(&mut self, caret: usize, template: &str) -> usize {
        let loc = self.loc(caret);
        let ni = loc.node;
        if template.is_empty() {
            self.nodes[ni].kind = NodeKind::Paragraph { inlines: vec![] };
            return self.caret_after(ni, None, None, 0);
        }
        let imported = Doc::from_gfm(template);
        if imported.nodes.is_empty() {
            return caret;
        }
        let first = imported.nodes[0].clone();
        self.nodes[ni] = first;
        for (i, n) in imported.nodes.into_iter().skip(1).enumerate() {
            self.nodes.insert(ni + 1 + i, n);
        }
        self.caret_after(ni, self.first_item(ni), None, 0)
    }

    pub fn apply_link(&mut self, sel: Range<usize>, url: &str) -> usize {
        let a = sel.start.min(sel.end);
        let b = sel.end.max(sel.start);
        if a >= b {
            return b;
        }
        let loc = self.loc(a);
        let loc_b = self.loc(b);
        if loc.node != loc_b.node || loc.item != loc_b.item {
            return b;
        }
        let p = self.project();
        let id = p.links.len() as u32;
        let inlines = self.inlines_at_mut(loc);
        set_link_range(inlines, loc.offset, loc_b.offset, if url.is_empty() { None } else { Some(id) });
        b
    }

    pub fn set_code_lang(&mut self, caret: usize, lang: &str) -> Option<usize> {
        let loc = self.loc(caret);
        match &mut self.nodes[loc.node].kind {
            NodeKind::Code { lang: l, .. } => {
                *l = lang.to_string();
                Some(self.caret_after(loc.node, None, None, 0))
            }
            _ => None,
        }
    }

    pub fn toggle_task(&mut self, node: usize, item: usize) {
        if let NodeKind::List { items, .. } = &mut self.nodes[node].kind {
            if let Some(it) = items.get_mut(item) {
                if let Some(c) = it.checked {
                    it.checked = Some(!c);
                }
            }
        }
    }

    pub fn delete_unit(&mut self, unit: usize) -> usize {
        let row_units = units(&self.project());
        if unit >= row_units.len() {
            return 0;
        }
        let u = row_units[unit];
        match u.item {
            Some(i) => {
                if let NodeKind::List { items, .. } = &mut self.nodes[u.block].kind {
                    if items.len() <= 1 {
                        self.nodes.remove(u.block);
                    } else {
                        items.remove(i);
                    }
                }
            }
            None => {
                if self.nodes.len() == 1 {
                    self.nodes[0].kind = NodeKind::Paragraph { inlines: vec![] };
                    return 0;
                }
                self.nodes.remove(u.block);
            }
        }
        if self.nodes.is_empty() {
            *self = Self::empty();
        }
        let p = self.project();
        let us = units(&p);
        let at = unit.min(us.len().saturating_sub(1));
        us.get(at)
            .map(|u| unit_display(&p, *u).start)
            .unwrap_or(0)
    }

    pub fn duplicate_unit(&mut self, unit: usize) -> usize {
        let row_units = units(&self.project());
        if unit >= row_units.len() {
            return 0;
        }
        let u = row_units[unit];
        match u.item {
            Some(i) => {
                if let NodeKind::List { items, .. } = &mut self.nodes[u.block].kind {
                    let copy = items[i].clone();
                    items.insert(i + 1, copy);
                }
            }
            None => {
                let copy = self.nodes[u.block].clone();
                let copy = Node {
                    id: next_id(),
                    kind: copy.kind,
                };
                self.nodes.insert(u.block + 1, copy);
            }
        }
        let p = self.project();
        let us = units(&p);
        us.get(unit + 1)
            .map(|u| unit_display(&p, *u).start)
            .unwrap_or(0)
    }

    pub fn move_unit(&mut self, from: usize, gap: usize) -> Option<usize> {
        let row_units = units(&self.project());
        let n = row_units.len();
        if from >= n || gap > n || gap == from || gap == from + 1 {
            return None;
        }
        // Extract payload as a node or list item, then insert at gap.
        let u = row_units[from];
        let payload = match u.item {
            Some(i) => {
                let ordered = match &self.nodes[u.block].kind {
                    NodeKind::List { ordered, .. } => *ordered,
                    _ => return None,
                };
                let NodeKind::List { items, .. } = &mut self.nodes[u.block].kind else {
                    return None;
                };
                let item = items.remove(i);
                if items.is_empty() {
                    self.nodes.remove(u.block);
                }
                Payload::Item { ordered, item }
            }
            None => {
                let node = self.nodes.remove(u.block);
                Payload::Node(node)
            }
        };
        let p = self.project();
        let us = units(&p);
        let insert_at = if from < gap { gap - 1 } else { gap };
        let insert_at = insert_at.min(us.len());
        match payload {
            Payload::Node(node) => {
                if insert_at >= us.len() {
                    self.nodes.push(node);
                } else {
                    let t = us[insert_at];
                    self.nodes.insert(t.block, node);
                }
            }
            Payload::Item { ordered, item } => {
                if insert_at >= us.len() {
                    self.nodes.push(Node {
                        id: next_id(),
                        kind: NodeKind::List {
                            ordered,
                            items: vec![item],
                        },
                    });
                } else {
                    let t = us[insert_at];
                    match t.item {
                        Some(i) => {
                            if let NodeKind::List { items, .. } = &mut self.nodes[t.block].kind {
                                items.insert(i, item);
                            }
                        }
                        None => {
                            self.nodes.insert(
                                t.block,
                                Node {
                                    id: next_id(),
                                    kind: NodeKind::List {
                                        ordered,
                                        items: vec![item],
                                    },
                                },
                            );
                        }
                    }
                }
            }
        }
        let p = self.project();
        let us = units(&p);
        Some(
            us.get(insert_at.min(us.len().saturating_sub(1)))
                .map(|u| unit_display(&p, *u).start)
                .unwrap_or(0),
        )
    }

    pub fn open_line(&mut self, caret: usize, above: bool) -> usize {
        let loc = self.loc(caret);
        let at = if above { loc.node } else { loc.node + 1 };
        self.nodes.insert(
            at,
            Node {
                id: next_id(),
                kind: NodeKind::Paragraph { inlines: vec![] },
            },
        );
        self.caret_after(at, None, None, 0)
    }

    pub fn tab(&mut self, caret: usize, outdent: bool) -> Option<usize> {
        let loc = self.loc(caret);
        let NodeKind::List { items, .. } = &mut self.nodes[loc.node].kind else {
            return None;
        };
        let ix = loc.item?;
        if outdent {
            if items[ix].indent == 0 {
                return None;
            }
            items[ix].indent -= 1;
        } else {
            let max = if ix == 0 {
                0
            } else {
                items[ix - 1].indent + 1
            };
            if items[ix].indent >= max {
                return None;
            }
            items[ix].indent += 1;
        }
        Some(self.caret_after(loc.node, Some(ix), None, loc.offset))
    }

    fn maybe_shortcut(&mut self, node: usize, item: Option<usize>) {
        if item.is_some() {
            return;
        }
        let NodeKind::Paragraph { inlines } = &self.nodes[node].kind else {
            return;
        };
        let text = inlines_text(inlines);
        if let Some((kind, rest)) = parse_shortcut(&text) {
            self.nodes[node].kind = kind;
            if let NodeKind::Paragraph { .. } = &self.nodes[node].kind {
                let _ = rest;
            }
            match &mut self.nodes[node].kind {
                NodeKind::Heading { inlines, .. }
                | NodeKind::Quote { inlines }
                | NodeKind::Alert { inlines, .. }
                | NodeKind::Paragraph { inlines } => {
                    *inlines = if rest.is_empty() {
                        vec![]
                    } else {
                        vec![Inline {
                            text: rest,
                            marks: Marks::default(),
                        }]
                    };
                }
                NodeKind::List { items, .. } => {
                    if let Some(it) = items.get_mut(0) {
                        it.inlines = if rest.is_empty() {
                            vec![]
                        } else {
                            vec![Inline {
                                text: rest,
                                marks: Marks::default(),
                            }]
                        };
                    }
                }
                NodeKind::Code { text, .. } => *text = rest,
                _ => {}
            }
        }
    }

    fn loc(&self, d: usize) -> Loc {
        let p = self.project();
        let d = d.min(p.display.len());
        let Some(b) = p.block_at_display(d) else {
            return Loc {
                node: self.nodes.len().saturating_sub(1),
                item: None,
                cell: None,
                offset: 0,
            };
        };
        let node = p
            .blocks
            .iter()
            .position(|x| x.display == b.display)
            .unwrap_or(0);
        if let BlockExtra::List { items, .. } = &b.extra {
            if let Some((i, it)) = items
                .iter()
                .enumerate()
                .find(|(_, it)| d >= it.display.start && d <= it.display.end)
            {
                return Loc {
                    node,
                    item: Some(i),
                    cell: None,
                    offset: d.saturating_sub(it.display.start),
                };
            }
        }
        if let BlockExtra::Table { cells, .. } = &b.extra {
            if let Some(c) = cells
                .iter()
                .find(|c| d >= c.display.start && d <= c.display.end)
            {
                return Loc {
                    node,
                    item: None,
                    cell: Some((c.row, c.col)),
                    offset: d.saturating_sub(c.display.start),
                };
            }
        }
        Loc {
            node,
            item: None,
            cell: None,
            offset: d.saturating_sub(b.display.start),
        }
    }

    fn inlines_at(&self, loc: Loc) -> &[Inline] {
        match &self.nodes.get(loc.node).map(|n| &n.kind) {
            Some(NodeKind::Paragraph { inlines })
            | Some(NodeKind::Heading { inlines, .. })
            | Some(NodeKind::Quote { inlines })
            | Some(NodeKind::Alert { inlines, .. }) => inlines,
            Some(NodeKind::List { items, .. }) => items
                .get(loc.item.unwrap_or(0))
                .map(|i| i.inlines.as_slice())
                .unwrap_or(&[]),
            _ => &[],
        }
    }

    fn inlines_at_mut(&mut self, loc: Loc) -> &mut Vec<Inline> {
        let convertible = !matches!(
            self.nodes[loc.node].kind,
            NodeKind::Paragraph { .. }
                | NodeKind::Heading { .. }
                | NodeKind::Quote { .. }
                | NodeKind::Alert { .. }
                | NodeKind::List { .. }
                | NodeKind::Table { .. }
        );
        if convertible {
            self.nodes[loc.node].kind = NodeKind::Paragraph { inlines: vec![] };
        }
        match &mut self.nodes[loc.node].kind {
            NodeKind::Paragraph { inlines }
            | NodeKind::Heading { inlines, .. }
            | NodeKind::Quote { inlines }
            | NodeKind::Alert { inlines, .. } => inlines,
            NodeKind::List { items, .. } => {
                let i = loc.item.unwrap_or(0).min(items.len().saturating_sub(1));
                &mut items[i].inlines
            }
            NodeKind::Table { headers, rows } => {
                let (r, c) = loc.cell.unwrap_or((0, 0));
                if r == 0 {
                    if headers.is_empty() {
                        headers.push(vec![]);
                    }
                    let c = c.min(headers.len().saturating_sub(1));
                    &mut headers[c]
                } else {
                    let rr = r.saturating_sub(1);
                    if rows.is_empty() {
                        rows.push(vec![vec![]]);
                    }
                    let rr = rr.min(rows.len().saturating_sub(1));
                    if rows[rr].is_empty() {
                        rows[rr].push(vec![]);
                    }
                    let c = c.min(rows[rr].len().saturating_sub(1));
                    &mut rows[rr][c]
                }
            }
            _ => unreachable!(),
        }
    }

    fn caret_after(
        &self,
        node: usize,
        item: Option<usize>,
        _cell: Option<(usize, usize)>,
        offset: usize,
    ) -> usize {
        let p = self.project();
        let Some(b) = p.blocks.get(node) else {
            return p.display.len();
        };
        if let (Some(i), BlockExtra::List { items, .. }) = (item, &b.extra) {
            if let Some(it) = items.get(i) {
                let len = it.display.end.saturating_sub(it.display.start);
                return it.display.start + offset.min(len);
            }
        }
        let len = b.display.end.saturating_sub(b.display.start);
        b.display.start + offset.min(len)
    }

    fn node_end(&self, node: usize, item: Option<usize>) -> usize {
        let p = self.project();
        let Some(b) = p.blocks.get(node) else {
            return p.display.len();
        };
        if let (Some(i), BlockExtra::List { items, .. }) = (item, &b.extra) {
            if let Some(it) = items.get(i) {
                return it.display.end;
            }
        }
        b.display.end
    }

    fn node_text_len(&self, node: usize) -> usize {
        match &self.nodes[node].kind {
            NodeKind::Code { text, .. } => text.len(),
            NodeKind::List { items, .. } => items.last().map(|i| inlines_len(&i.inlines)).unwrap_or(0),
            NodeKind::Paragraph { inlines }
            | NodeKind::Heading { inlines, .. }
            | NodeKind::Quote { inlines }
            | NodeKind::Alert { inlines, .. } => inlines_len(inlines),
            _ => 0,
        }
    }

    fn last_item(&self, node: usize) -> Option<usize> {
        match &self.nodes[node].kind {
            NodeKind::List { items, .. } if !items.is_empty() => Some(items.len() - 1),
            _ => None,
        }
    }

    fn first_item(&self, node: usize) -> Option<usize> {
        match &self.nodes[node].kind {
            NodeKind::List { items, .. } if !items.is_empty() => Some(0),
            _ => None,
        }
    }

    fn merge_nodes(&mut self, keep: usize, drop: usize) -> usize {
        if keep >= drop || drop >= self.nodes.len() {
            return self.caret_after(keep, None, None, self.node_text_len(keep));
        }
        let keep_len = self.node_text_len(keep);
        let right = match &self.nodes[drop].kind {
            NodeKind::Paragraph { inlines }
            | NodeKind::Heading { inlines, .. }
            | NodeKind::Quote { inlines }
            | NodeKind::Alert { inlines, .. } => inlines.clone(),
            NodeKind::Code { text, .. } => vec![Inline {
                text: text.clone(),
                marks: Marks::default(),
            }],
            _ => vec![],
        };
        match &mut self.nodes[keep].kind {
            NodeKind::Paragraph { inlines }
            | NodeKind::Heading { inlines, .. }
            | NodeKind::Quote { inlines }
            | NodeKind::Alert { inlines, .. } => inlines.extend(right),
            NodeKind::Code { text, .. } => text.push_str(&inlines_text(&right)),
            NodeKind::List { items, .. } => {
                if let Some(last) = items.last_mut() {
                    last.inlines.extend(right);
                }
            }
            _ => {}
        }
        self.nodes.remove(drop);
        self.caret_after(keep, self.last_item(keep), None, keep_len)
    }
}

enum Payload {
    Node(Node),
    Item { ordered: bool, item: ListItem },
}

fn node_from_proj(p: &Projection, b: &ProjBlock, src: &str) -> Node {
    let kind = match &b.extra {
        BlockExtra::Heading(level) => NodeKind::Heading {
            level: *level,
            inlines: inlines_in(p, b.display.clone()),
        },
        BlockExtra::Quote => NodeKind::Quote {
            inlines: inlines_in(p, b.display.clone()),
        },
        BlockExtra::Alert(k) => NodeKind::Alert {
            kind: *k,
            inlines: inlines_in(p, b.display.clone()),
        },
        BlockExtra::List { ordered, items } => NodeKind::List {
            ordered: *ordered,
            items: items
                .iter()
                .map(|it| ListItem {
                    indent: it.indent,
                    checked: it.checked,
                    inlines: inlines_in(p, it.display.clone()),
                })
                .collect(),
        },
        BlockExtra::Code { lang } => NodeKind::Code {
            lang: lang.clone(),
            text: p.display.get(b.display.clone()).unwrap_or("").to_string(),
        },
        BlockExtra::Table { cells, rows, cols } => {
            let mut headers = vec![vec![]; (*cols).max(1)];
            let mut body = vec![vec![vec![]; (*cols).max(1)]; (*rows).saturating_sub(1)];
            for c in cells {
                let ins = inlines_in(p, c.display.clone());
                if c.header {
                    if c.col < headers.len() {
                        headers[c.col] = ins;
                    }
                } else {
                    let r = c.row.saturating_sub(1);
                    if r < body.len() && c.col < body[r].len() {
                        body[r][c.col] = ins;
                    }
                }
            }
            NodeKind::Table {
                headers,
                rows: body,
            }
        }
        BlockExtra::Rule => NodeKind::Rule,
        BlockExtra::Image { alt, src } => NodeKind::Image {
            alt: alt.clone(),
            src: src.clone(),
        },
        BlockExtra::Html => NodeKind::Html {
            raw: src.get(b.source.clone()).unwrap_or("").to_string(),
        },
        BlockExtra::Text => NodeKind::Paragraph {
            inlines: inlines_in(p, b.display.clone()),
        },
    };
    Node {
        id: next_id(),
        kind,
    }
}

fn inlines_in(p: &Projection, range: Range<usize>) -> Vec<Inline> {
    let mut out = Vec::new();
    for seg in &p.segments {
        let a = seg.display.start.max(range.start);
        let b = seg.display.end.min(range.end);
        if a >= b {
            continue;
        }
        let text = p.display[a..b].to_string();
        if text == "\n" {
            continue;
        }
        out.push(Inline {
            text,
            marks: seg.marks,
        });
    }
    merge_inlines(out)
}

fn merge_inlines(runs: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    for r in runs {
        if r.text.is_empty() {
            continue;
        }
        if let Some(last) = out.last_mut() {
            if last.marks == r.marks {
                last.text.push_str(&r.text);
                continue;
            }
        }
        out.push(r);
    }
    out
}

fn inlines_text(inlines: &[Inline]) -> String {
    let mut s = String::new();
    for i in inlines {
        s.push_str(&i.text);
    }
    s
}

fn inlines_len(inlines: &[Inline]) -> usize {
    inlines.iter().map(|i| i.text.len()).sum()
}

fn insert_inlines(inlines: &mut Vec<Inline>, offset: usize, text: &str, marks: Marks) {
    if text.is_empty() {
        return;
    }
    let mut at = 0usize;
    for i in 0..inlines.len() {
        let len = inlines[i].text.len();
        if offset <= at + len {
            let local = offset - at;
            if inlines[i].marks == marks {
                inlines[i].text.insert_str(local, text);
                return;
            }
            let rest = inlines[i].text.split_off(local);
            let m = inlines[i].marks;
            inlines.insert(
                i + 1,
                Inline {
                    text: text.to_string(),
                    marks,
                },
            );
            if !rest.is_empty() {
                inlines.insert(
                    i + 2,
                    Inline {
                        text: rest,
                        marks: m,
                    },
                );
            }
            if inlines[i].text.is_empty() {
                inlines.remove(i);
            }
            return;
        }
        at += len;
    }
    inlines.push(Inline {
        text: text.to_string(),
        marks,
    });
}

fn delete_inlines(inlines: &mut Vec<Inline>, start: usize, end: usize) {
    if start >= end {
        return;
    }
    let mut at = 0usize;
    let mut i = 0;
    while i < inlines.len() {
        let len = inlines[i].text.len();
        let a = at;
        let b = at + len;
        if b <= start || a >= end {
            at = b;
            i += 1;
            continue;
        }
        let local0 = start.saturating_sub(a).min(len);
        let local1 = end.saturating_sub(a).min(len);
        inlines[i].text.replace_range(local0..local1, "");
        if inlines[i].text.is_empty() {
            inlines.remove(i);
        } else {
            i += 1;
        }
        at = b;
    }
}

fn split_inlines(inlines: &[Inline], offset: usize) -> (Vec<Inline>, Vec<Inline>) {
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut at = 0usize;
    for run in inlines {
        let len = run.text.len();
        if at + len <= offset {
            left.push(run.clone());
        } else if at >= offset {
            right.push(run.clone());
        } else {
            let local = offset - at;
            if local > 0 {
                left.push(Inline {
                    text: run.text[..local].to_string(),
                    marks: run.marks,
                });
            }
            if local < len {
                right.push(Inline {
                    text: run.text[local..].to_string(),
                    marks: run.marks,
                });
            }
        }
        at += len;
    }
    (merge_inlines(left), merge_inlines(right))
}

fn apply_mark_range(inlines: &mut Vec<Inline>, start: usize, end: usize, mark: Mark, on: bool) {
    let text = inlines_text(inlines);
    let end = end.min(text.len());
    let start = start.min(end);
    if start == end {
        return;
    }
    let mut next = Vec::new();
    let mut at = 0usize;
    for run in inlines.drain(..) {
        let len = run.text.len();
        let a = at;
        let b = at + len;
        if b <= start || a >= end {
            next.push(run);
            at = b;
            continue;
        }
        let l0 = start.saturating_sub(a).min(len);
        let l1 = end.saturating_sub(a).min(len);
        if l0 > 0 {
            next.push(Inline {
                text: run.text[..l0].to_string(),
                marks: run.marks,
            });
        }
        let mut m = run.marks;
        set_mark(&mut m, mark, on);
        next.push(Inline {
            text: run.text[l0..l1].to_string(),
            marks: m,
        });
        if l1 < len {
            next.push(Inline {
                text: run.text[l1..].to_string(),
                marks: run.marks,
            });
        }
        at = b;
    }
    *inlines = merge_inlines(next);
}

fn set_link_range(inlines: &mut Vec<Inline>, start: usize, end: usize, link: Option<u32>) {
    let mut at = 0usize;
    let mut next = Vec::new();
    for run in inlines.drain(..) {
        let len = run.text.len();
        let a = at;
        let b = at + len;
        if b <= start || a >= end {
            next.push(run);
            at = b;
            continue;
        }
        let l0 = start.saturating_sub(a).min(len);
        let l1 = end.saturating_sub(a).min(len);
        if l0 > 0 {
            next.push(Inline {
                text: run.text[..l0].to_string(),
                marks: run.marks,
            });
        }
        let mut m = run.marks;
        m.link = link;
        next.push(Inline {
            text: run.text[l0..l1].to_string(),
            marks: m,
        });
        if l1 < len {
            next.push(Inline {
                text: run.text[l1..].to_string(),
                marks: run.marks,
            });
        }
        at = b;
    }
    *inlines = merge_inlines(next);
}

fn set_mark(m: &mut Marks, mark: Mark, on: bool) {
    match mark {
        Mark::Bold => m.bold = on,
        Mark::Italic => m.italic = on,
        Mark::Strike => m.strike = on,
        Mark::Code => m.code = on,
        Mark::Underline => m.underline = on,
    }
}

fn parse_shortcut(text: &str) -> Option<(NodeKind, String)> {
    if let Some(rest) = text.strip_prefix("# ") {
        return Some((
            NodeKind::Heading {
                level: 1,
                inlines: vec![],
            },
            rest.to_string(),
        ));
    }
    for n in (2..=6).rev() {
        let prefix = format!("{} ", "#".repeat(n));
        if let Some(rest) = text.strip_prefix(&prefix) {
            return Some((
                NodeKind::Heading {
                    level: n as u8,
                    inlines: vec![],
                },
                rest.to_string(),
            ));
        }
    }
    for (prefix, ordered, checked) in [
        ("- [ ] ", false, Some(false)),
        ("- [x] ", false, Some(true)),
        ("- ", false, None),
        ("* ", false, None),
        ("+ ", false, None),
        ("1. ", true, None),
        ("1) ", true, None),
    ] {
        if let Some(rest) = text.strip_prefix(prefix) {
            return Some((
                NodeKind::List {
                    ordered,
                    items: vec![ListItem {
                        indent: 0,
                        checked,
                        inlines: vec![],
                    }],
                },
                rest.to_string(),
            ));
        }
    }
    if let Some(rest) = text.strip_prefix("> ") {
        return Some((NodeKind::Quote { inlines: vec![] }, rest.to_string()));
    }
    None
}

fn node_to_gfm(n: &Node) -> String {
    match &n.kind {
        NodeKind::Paragraph { inlines } => inlines_to_gfm(inlines),
        NodeKind::Heading { level, inlines } => {
            format!("{} {}", "#".repeat((*level).clamp(1, 6) as usize), inlines_to_gfm(inlines))
        }
        NodeKind::Quote { inlines } => wrap_lines("> ", &inlines_to_gfm(inlines)),
        NodeKind::Alert { kind, inlines } => {
            let body = inlines_to_gfm(inlines);
            if body.is_empty() {
                format!("> [!{}]\n> ", kind.as_str())
            } else {
                format!("> [!{}]\n{}", kind.as_str(), wrap_lines("> ", &body))
            }
        }
        NodeKind::List { ordered, items } => items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                let pad = "  ".repeat(it.indent);
                let marker = if let Some(c) = it.checked {
                    if c {
                        "- [x] ".to_string()
                    } else {
                        "- [ ] ".to_string()
                    }
                } else if *ordered {
                    // Sibling index within this indent under the nearest shallower parent.
                    let mut n = 0usize;
                    for j in (0..i).rev() {
                        if items[j].indent < it.indent {
                            break;
                        }
                        if items[j].indent == it.indent {
                            n += 1;
                        }
                    }
                    format!("{}. ", n + 1)
                } else {
                    "- ".to_string()
                };
                format!("{pad}{marker}{}", inlines_to_gfm(&it.inlines))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        NodeKind::Code { lang, text } => {
            format!("```{lang}\n{text}\n```")
        }
        NodeKind::Table { headers, rows } => {
            let hs: Vec<String> = headers.iter().map(|c| inlines_to_gfm(c)).collect();
            let rs: Vec<Vec<String>> = rows
                .iter()
                .map(|r| r.iter().map(|c| inlines_to_gfm(c)).collect())
                .collect();
            crate::display::serialize_table(&hs, &rs)
        }
        NodeKind::Rule => "---".into(),
        NodeKind::Image { alt, src } => format!("![{alt}]({src})"),
        NodeKind::Html { raw } => raw.clone(),
    }
}

fn wrap_lines(prefix: &str, body: &str) -> String {
    if body.is_empty() {
        return format!("{prefix}");
    }
    body.lines()
        .map(|l| format!("{prefix}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn inlines_to_gfm(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for run in inlines {
        let mut s = escape_md(&run.text);
        if run.marks.code {
            s = format!("`{s}`");
        }
        if run.marks.bold {
            s = format!("**{s}**");
        }
        if run.marks.italic {
            s = format!("*{s}*");
        }
        if run.marks.strike {
            s = format!("~~{s}~~");
        }
        if run.marks.underline {
            s = format!("<u>{s}</u>");
        }
        if let Some(_id) = run.marks.link {
            s = format!("[{s}]()");
        }
        out.push_str(&s);
    }
    out
}

fn escape_md(s: &str) -> String {
    s.replace('\\', "\\\\")
}

fn join_gfm(nodes: &[Node], parts: &[String]) -> String {
    let mut out = String::new();
    let mut pending_empty = 0usize;
    for (n, part) in nodes.iter().zip(parts.iter()) {
        let empty_para = matches!(n.kind, NodeKind::Paragraph { ref inlines } if inlines.is_empty());
        if empty_para {
            pending_empty += 1;
            continue;
        }
        if !out.is_empty() || pending_empty > 0 {
            // Between content: `a\n\nb` = 0 empties, `a\n\n\nb` = 1, … → 2 + pending.
            // Leading empties: `\n\nLists` = 1 empty, `\n\n\nLists` = 2 → pending + 1.
            let need = if out.is_empty() {
                pending_empty + 1
            } else {
                2 + pending_empty
            };
            let have = out.bytes().rev().take_while(|&b| b == b'\n').count();
            let extra = need.saturating_sub(have);
            for _ in 0..extra {
                out.push('\n');
            }
        }
        out.push_str(part);
        pending_empty = 0;
    }
    if pending_empty > 0 {
        let need = if out.is_empty() {
            // All-empty doc: 1 → "", 2 → "\n\n", 3 → "\n\n\n" (matches from_gfm).
            if pending_empty <= 1 {
                0
            } else {
                pending_empty
            }
        } else {
            pending_empty + 1
        };
        let have = out.bytes().rev().take_while(|&b| b == b'\n').count();
        for _ in have..need {
            out.push('\n');
        }
    }
    out
}

fn flatten(doc: &Doc) -> Projection {
    let mut display = String::new();
    let mut segments = Vec::new();
    let mut blocks = Vec::new();
    let mut links = Vec::new();
    for (i, n) in doc.nodes.iter().enumerate() {
        if i > 0 {
            let d0 = display.len();
            display.push('\n');
            segments.push(Segment {
                display: d0..display.len(),
                source: d0..display.len(),
                marks: Marks::default(),
            });
        }
        let d0 = display.len();
        let (extra, kind) = emit_node(n, &mut display, &mut segments, &mut links);
        if display.len() == d0 && extra.is_atomic() {
            let s0 = display.len();
            display.push('\u{00A0}');
            segments.push(Segment {
                display: s0..display.len(),
                source: s0..display.len(),
                marks: Marks::default(),
            });
        }
        if display.len() == d0 {
            segments.push(Segment {
                display: d0..d0,
                source: d0..d0,
                marks: Marks::default(),
            });
        }
        blocks.push(ProjBlock {
            kind,
            source: d0..display.len(),
            display: d0..display.len(),
            extra,
        });
    }
    if blocks.is_empty() {
        blocks.push(ProjBlock {
            kind: BlockKind::Paragraph,
            source: 0..0,
            display: 0..0,
            extra: BlockExtra::Text,
        });
    }
    let source_len = display.len();
    Projection {
        display,
        segments,
        blocks,
        links,
        source_len,
    }
}

fn emit_node(
    n: &Node,
    display: &mut String,
    segments: &mut Vec<Segment>,
    links: &mut Vec<String>,
) -> (BlockExtra, BlockKind) {
    match &n.kind {
        NodeKind::Paragraph { inlines } => {
            emit_inlines(inlines, display, segments, links);
            (BlockExtra::Text, BlockKind::Paragraph)
        }
        NodeKind::Heading { level, inlines } => {
            emit_inlines(inlines, display, segments, links);
            (BlockExtra::Heading(*level), BlockKind::Heading(*level))
        }
        NodeKind::Quote { inlines } => {
            emit_inlines(inlines, display, segments, links);
            (BlockExtra::Quote, BlockKind::Quote)
        }
        NodeKind::Alert { kind, inlines } => {
            emit_inlines(inlines, display, segments, links);
            (BlockExtra::Alert(*kind), BlockKind::Alert(*kind))
        }
        NodeKind::List { ordered, items } => {
            let mut proj_items = Vec::new();
            for (j, it) in items.iter().enumerate() {
                if j > 0 {
                    let d0 = display.len();
                    display.push('\n');
                    segments.push(Segment {
                        display: d0..display.len(),
                        source: d0..display.len(),
                        marks: Marks::default(),
                    });
                }
                let d0 = display.len();
                emit_inlines(&it.inlines, display, segments, links);
                proj_items.push(ProjListItem {
                    display: d0..display.len(),
                    source: d0..display.len(),
                    indent: it.indent,
                    checked: it.checked,
                });
            }
            (
                BlockExtra::List {
                    ordered: *ordered,
                    items: proj_items,
                },
                BlockKind::List {
                    ordered: *ordered,
                },
            )
        }
        NodeKind::Code { lang, text } => {
            let d0 = display.len();
            display.push_str(text);
            segments.push(Segment {
                display: d0..display.len(),
                source: d0..display.len(),
                marks: Marks::default(),
            });
            (
                BlockExtra::Code { lang: lang.clone() },
                BlockKind::Code,
            )
        }
        NodeKind::Table { headers, rows } => {
            let cols = headers.len().max(1);
            let mut cells = Vec::new();
            for (c, h) in headers.iter().enumerate() {
                if c > 0 {
                    let d0 = display.len();
                    display.push('\t');
                    segments.push(Segment {
                        display: d0..display.len(),
                        source: d0..display.len(),
                        marks: Marks::default(),
                    });
                }
                let d0 = display.len();
                emit_inlines(h, display, segments, links);
                cells.push(TableCell {
                    display: d0..display.len(),
                    source: d0..display.len(),
                    header: true,
                    row: 0,
                    col: c,
                });
            }
            for (ri, row) in rows.iter().enumerate() {
                let d0 = display.len();
                display.push('\n');
                segments.push(Segment {
                    display: d0..display.len(),
                    source: d0..display.len(),
                    marks: Marks::default(),
                });
                for (c, cell) in row.iter().enumerate() {
                    if c > 0 {
                        let d0 = display.len();
                        display.push('\t');
                        segments.push(Segment {
                            display: d0..display.len(),
                            source: d0..display.len(),
                            marks: Marks::default(),
                        });
                    }
                    let d0 = display.len();
                    emit_inlines(cell, display, segments, links);
                    cells.push(TableCell {
                        display: d0..display.len(),
                        source: d0..display.len(),
                        header: false,
                        row: ri + 1,
                        col: c,
                    });
                }
            }
            (
                BlockExtra::Table {
                    cells,
                    rows: rows.len() + 1,
                    cols,
                },
                BlockKind::Table,
            )
        }
        NodeKind::Rule => (BlockExtra::Rule, BlockKind::Rule),
        NodeKind::Image { alt, src } => (
            BlockExtra::Image {
                alt: alt.clone(),
                src: src.clone(),
            },
            BlockKind::Paragraph,
        ),
        NodeKind::Html { raw } => {
            let d0 = display.len();
            display.push_str(raw);
            segments.push(Segment {
                display: d0..display.len(),
                source: d0..display.len(),
                marks: Marks::default(),
            });
            (BlockExtra::Html, BlockKind::Html)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unit {
    pub block: usize,
    pub item: Option<usize>,
}

impl Unit {
    pub fn is_list_item(self) -> bool {
        self.item.is_some()
    }
}

pub fn units(p: &Projection) -> Vec<Unit> {
    let mut out = Vec::new();
    for (bi, b) in p.blocks.iter().enumerate() {
        if let BlockExtra::List { items, .. } = &b.extra {
            if items.is_empty() {
                out.push(Unit {
                    block: bi,
                    item: None,
                });
            } else {
                for i in 0..items.len() {
                    out.push(Unit {
                        block: bi,
                        item: Some(i),
                    });
                }
            }
        } else {
            out.push(Unit {
                block: bi,
                item: None,
            });
        }
    }
    out
}

pub fn unit_display(p: &Projection, u: Unit) -> Range<usize> {
    match u.item {
        Some(i) => {
            if let Some(BlockExtra::List { items, .. }) = p.blocks.get(u.block).map(|b| &b.extra) {
                if let Some(it) = items.get(i) {
                    return it.display.clone();
                }
            }
            p.blocks[u.block].display.clone()
        }
        None => p.blocks[u.block].display.clone(),
    }
}

fn emit_inlines(
    inlines: &[Inline],
    display: &mut String,
    segments: &mut Vec<Segment>,
    links: &mut Vec<String>,
) {
    for run in inlines {
        let mut marks = run.marks;
        if let Some(id) = marks.link {
            while links.len() as u32 <= id {
                links.push(String::new());
            }
        }
        let d0 = display.len();
        display.push_str(&run.text);
        segments.push(Segment {
            display: d0..display.len(),
            source: d0..display.len(),
            marks,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_space_converts() {
        let mut d = Doc::empty();
        let mut c = d.insert_text(0, None, "#", Marks::default());
        assert_eq!(d.project().display, "#");
        c = d.insert_text(c, None, " ", Marks::default());
        assert!(matches!(d.nodes[0].kind, NodeKind::Heading { level: 1, .. }));
        assert_eq!(d.project().display, "");
        let _ = c;
    }

    #[test]
    fn type_between_code_and_table() {
        let mut d = Doc::from_gfm("```\nfn main() {}\n```\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n");
        let p = d.project();
        let empty = p
            .blocks
            .iter()
            .position(|b| b.display.start == b.display.end)
            .expect("empty");
        let caret = p.blocks[empty].display.start;
        let n = p.blocks.len();
        let c = d.insert_text(caret, None, "hi", Marks::default());
        let p2 = d.project();
        assert_eq!(p2.blocks.len(), n, "{:?}", p2.blocks.iter().map(|b| b.kind).collect::<Vec<_>>());
        assert!(p2.display.contains("hi"));
        let _ = c;
    }

    #[test]
    fn list_typing_stays_on_item() {
        let mut d = Doc::from_gfm("- a\n- b\n- c");
        let p = d.project();
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!();
        };
        let caret = items[2].display.end;
        let mut at = caret;
        for ch in ["a", "r", "l", "o"] {
            at = d.insert_text(at, None, ch, Marks::default());
        }
        assert_eq!(d.project().display, "a\nb\ncarlo");
    }
}

#[cfg(test)]
mod feature_tests {
    use super::*;

    #[test]
    fn tab_indents_list_item() {
        let mut d = Doc::from_gfm("- a\n- b");
        let p = d.project();
        // caret on second item
        let caret = p.blocks[0]
            .extra
            .clone();
        let BlockExtra::List { items, .. } = caret else { panic!() };
        let c = items[1].display.start;
        let c = d.tab(c, false).expect("indent");
        let NodeKind::List { items, .. } = &d.nodes[0].kind else { panic!() };
        assert_eq!(items[1].indent, 1);
        let c = d.tab(c, true).expect("outdent");
        let NodeKind::List { items, .. } = &d.nodes[0].kind else { panic!() };
        assert_eq!(items[1].indent, 0);
        assert!(d.tab(c, false).is_some());
    }

    #[test]
    fn toggle_task_flips_checked() {
        let mut d = Doc::from_gfm("- [ ] todo");
        d.toggle_task(0, 0);
        let NodeKind::List { items, .. } = &d.nodes[0].kind else { panic!() };
        assert_eq!(items[0].checked, Some(true));
        assert!(d.to_gfm().contains("- [x]"));
    }

    #[test]
    fn underline_mark_roundtrips_gfm() {
        let mut d = Doc::empty();
        let c = d.insert_text(0, None, "hi", Marks::default());
        d.toggle_mark(0..c, Mark::Underline).unwrap();
        let gfm = d.to_gfm();
        assert!(gfm.contains("<u>hi</u>"), "{gfm}");
        let d2 = Doc::from_gfm(&gfm);
        let p = d2.project();
        assert!(p.marks_at(0, Affinity::Inside).underline);
    }
}
