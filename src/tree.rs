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
        /// Fence indent in spaces (0 = top-level). Preserved on save so
        /// nested code stays nested under lists/details.
        #[allow(dead_code)]
        indent: usize,
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
    /// `<h1>…</h1>` … `<h6>…</h6>` — renders as a heading, round-trips tags.
    HtmlHeading {
        level: u8,
        inlines: Vec<Inline>,
    },
    /// `<details>` open row — summary inlines, editable.
    /// `open` mirrors GFM `<details open>` (expanded by default).
    Details {
        inlines: Vec<Inline>,
        open: bool,
    },
    /// `</details>` close — zero-height chrome, preserved on save.
    DetailsClose,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub id: u64,
    pub kind: NodeKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doc {
    pub nodes: Vec<Node>,
    pub links: Vec<String>,
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
            links: Vec::new(),
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
            return Self {
                nodes,
                links: Vec::new(),
            };
        }
        let p = project_gfm(src);
        let mut nodes = Vec::with_capacity(p.blocks.len());
        for b in &p.blocks {
            nodes.push(node_from_proj(&p, b, src));
        }
        if nodes.is_empty() {
            return Self::empty();
        }
        Self {
            nodes,
            links: p.links,
        }
    }

    pub fn project(&self) -> Projection {
        flatten(self)
    }

    pub fn to_gfm(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for n in &self.nodes {
            parts.push(node_to_gfm(n, &self.links));
        }
        join_gfm(&self.nodes, &parts)
    }

    pub fn gfm_range(&self, range: Range<usize>) -> String {
        let nodes = self.nodes_for_range(range);
        if nodes.is_empty() {
            return String::new();
        }
        let parts: Vec<String> = nodes.iter().map(|n| node_to_gfm(n, &self.links)).collect();
        join_gfm(&nodes, &parts)
    }

    pub fn paste_gfm(&mut self, caret: usize, gfm: &str) -> usize {
        if gfm.is_empty() {
            return caret;
        }
        let mut clip = Doc::from_gfm(gfm);
        let src_links = std::mem::take(&mut clip.links);
        for n in &mut clip.nodes {
            n.id = next_id();
            remap_node_links(n, &mut self.links, &src_links);
        }
        self.paste_nodes(caret, clip.nodes)
    }

    fn nodes_for_range(&self, range: Range<usize>) -> Vec<Node> {
        let a = range.start.min(range.end);
        let b = range.end.max(range.start);
        if a == b {
            return Vec::new();
        }
        let start = self.loc(a);
        let end = self.loc(b);
        if start.node == end.node {
            return match self.slice_same_node(start, end) {
                Some(n) => vec![n],
                None => Vec::new(),
            };
        }
        let mut out = Vec::new();
        if let Some(n) = self.slice_node_from(start) {
            out.push(n);
        }
        for i in start.node + 1..end.node {
            out.push(Node {
                id: next_id(),
                kind: self.nodes[i].kind.clone(),
            });
        }
        if end.offset > 0 || end.item.unwrap_or(0) > 0 {
            if let Some(n) = self.slice_node_to(end) {
                out.push(n);
            }
        }
        out
    }

    fn slice_same_node(&self, start: Loc, end: Loc) -> Option<Node> {
        let kind = match &self.nodes[start.node].kind {
            NodeKind::List { ordered, items } => {
                let i0 = start.item.unwrap_or(0).min(items.len().saturating_sub(1));
                let i1 = end.item.unwrap_or(i0).min(items.len().saturating_sub(1));
                if i0 == i1 {
                    let len = inlines_len(&items[i0].inlines);
                    let full = start.offset == 0 && end.offset >= len;
                    let sliced = slice_inlines(&items[i0].inlines, start.offset, end.offset);
                    if full {
                        NodeKind::List {
                            ordered: *ordered,
                            items: vec![ListItem {
                                indent: items[i0].indent,
                                checked: items[i0].checked,
                                inlines: sliced,
                            }],
                        }
                    } else {
                        NodeKind::Paragraph { inlines: sliced }
                    }
                } else {
                    let mut out = Vec::new();
                    for i in i0..=i1 {
                        let (off0, off1) = if i == i0 {
                            (start.offset, inlines_len(&items[i].inlines))
                        } else if i == i1 {
                            (0, end.offset)
                        } else {
                            (0, inlines_len(&items[i].inlines))
                        };
                        if i == i1 && end.offset == 0 {
                            continue;
                        }
                        out.push(ListItem {
                            indent: items[i].indent,
                            checked: items[i].checked,
                            inlines: slice_inlines(&items[i].inlines, off0, off1),
                        });
                    }
                    if out.is_empty() {
                        return None;
                    }
                    NodeKind::List {
                        ordered: *ordered,
                        items: out,
                    }
                }
            }
            NodeKind::Code { lang, text, indent, .. } => {
                let s = start.offset.min(text.len());
                let e = end.offset.min(text.len()).max(s);
                if s == 0 && e == text.len() {
                    NodeKind::Code {
                        lang: lang.clone(),
                        text: text.clone(),
                        indent: *indent,
                    }
                } else {
                    NodeKind::Paragraph {
                        inlines: vec![Inline {
                            text: text[s..e].to_string(),
                            marks: Marks::default(),
                        }],
                    }
                }
            }
            NodeKind::Paragraph { inlines }
            | NodeKind::Heading { inlines, .. }
            | NodeKind::HtmlHeading { inlines, .. }
            | NodeKind::Details { inlines, .. }
            | NodeKind::HtmlHeading { inlines, .. }
            | NodeKind::Details { inlines, .. }
            | NodeKind::Quote { inlines }
            | NodeKind::Alert { inlines, .. } => {
                let len = inlines_len(inlines);
                let full = start.offset == 0 && end.offset >= len;
                let sliced = slice_inlines(inlines, start.offset, end.offset);
                slice_kind_keep(&self.nodes[start.node].kind, sliced, full)
            }
            NodeKind::Table { .. }
            | NodeKind::Rule
            | NodeKind::Image { .. }
            | NodeKind::DetailsClose
            | NodeKind::Html { .. } => self.nodes[start.node].kind.clone(),
        };
        Some(Node {
            id: next_id(),
            kind,
        })
    }

    fn slice_node_from(&self, start: Loc) -> Option<Node> {
        self.slice_same_node(
            start,
            Loc {
                node: start.node,
                item: self.last_item(start.node),
                cell: None,
                offset: self.node_text_len(start.node),
            },
        )
    }

    fn slice_node_to(&self, end: Loc) -> Option<Node> {
        self.slice_same_node(
            Loc {
                node: end.node,
                item: self.first_item(end.node),
                cell: None,
                offset: 0,
            },
            end,
        )
    }

    fn paste_nodes(&mut self, caret: usize, mut nodes: Vec<Node>) -> usize {
        let keep_empty = nodes.len() == 1;
        nodes.retain(|n| !node_is_empty(n) || keep_empty);
        if nodes.is_empty() {
            return caret;
        }
        let loc = self.loc(caret);
        if matches!(
            self.nodes[loc.node].kind,
            NodeKind::Code { .. } | NodeKind::Html { .. } | NodeKind::Table { .. }
        ) {
            let text = nodes_plain_text(&nodes);
            return self.insert_text(caret, None, &text, Marks::default());
        }
        if matches!(self.nodes[loc.node].kind, NodeKind::List { .. })
            && nodes
                .iter()
                .all(|n| matches!(n.kind, NodeKind::List { .. }))
        {
            let items = flatten_list_items(nodes);
            return self.paste_list_items(loc, items);
        }
        if nodes.len() == 1 {
            if let NodeKind::Paragraph { inlines } = &nodes[0].kind {
                return self.insert_runs(caret, inlines.clone());
            }
        }
        if node_is_empty(&self.nodes[loc.node]) && loc.item.is_none() {
            let last = nodes.len() - 1;
            let last_len = node_content_len(&nodes[last]);
            let last_item = last_item_of(&nodes[last]);
            self.nodes[loc.node] = nodes.remove(0);
            let extra = nodes.len();
            for (i, n) in nodes.into_iter().enumerate() {
                self.nodes.insert(loc.node + 1 + i, n);
            }
            return self.caret_after(loc.node + extra, last_item, None, last_len);
        }

        let loc = self.loc(caret);
        let in_list = matches!(self.nodes[loc.node].kind, NodeKind::List { .. });
        let at_start = loc.offset == 0 && loc.item.unwrap_or(0) == 0;
        let at_end = loc.offset >= self.content_len_at(loc) && self.is_last_slot(loc);

        if in_list && !at_start && !at_end {
            self.split_list_at(loc);
        } else if !in_list && !at_start && !at_end {
            self.split_text_node(loc);
        }

        let mut insert_ix = if at_start { loc.node } else { loc.node + 1 };
        if at_end && can_merge_para(&self.nodes[loc.node], &nodes[0]) {
            self.merge_kind_into(loc.node, nodes.remove(0));
            insert_ix = loc.node + 1;
            if nodes.is_empty() {
                return self.caret_after(
                    loc.node,
                    self.last_item(loc.node),
                    None,
                    self.node_text_len(loc.node),
                );
            }
        }
        let last_len = nodes.last().map(node_content_len).unwrap_or(0);
        let last_item = nodes.last().and_then(last_item_of);
        let n = nodes.len();
        for (i, node) in nodes.into_iter().enumerate() {
            self.nodes.insert(insert_ix + i, node);
        }
        self.caret_after(insert_ix + n - 1, last_item, None, last_len)
    }

    fn insert_runs(&mut self, caret: usize, runs: Vec<Inline>) -> usize {
        if runs.is_empty() {
            return caret;
        }
        let loc = self.loc(caret);
        if matches!(
            self.nodes[loc.node].kind,
            NodeKind::Code { .. } | NodeKind::Html { .. } | NodeKind::Rule | NodeKind::Image { .. }
        ) {
            return self.insert_text(caret, None, &inlines_text(&runs), Marks::default());
        }
        let mut off = loc.offset;
        let node = loc.node;
        let item = loc.item;
        let cell = loc.cell;
        for run in &runs {
            if run.text.is_empty() {
                continue;
            }
            insert_inlines(
                self.inlines_at_mut(Loc {
                    node,
                    item,
                    cell,
                    offset: off,
                }),
                off,
                &run.text,
                run.marks,
            );
            off += run.text.len();
        }
        self.caret_after(node, item, cell, off)
    }

    fn paste_list_items(&mut self, loc: Loc, items: Vec<ListItem>) -> usize {
        if items.is_empty() {
            return self.caret_after(loc.node, loc.item, None, loc.offset);
        }
        let last_len = inlines_len(&items.last().unwrap().inlines);
        let n_items = items.len();
        let insert_at;
        {
            let NodeKind::List { items: dest, .. } = &mut self.nodes[loc.node].kind else {
                return self.caret_after(loc.node, loc.item, None, loc.offset);
            };
            let ix = loc.item.unwrap_or(0).min(dest.len().saturating_sub(1));
            let item_len = inlines_len(&dest[ix].inlines);
            insert_at = if item_len == 0 {
                dest.remove(ix);
                ix
            } else if loc.offset == 0 {
                ix
            } else if loc.offset >= item_len {
                ix + 1
            } else {
                let (left, right) = split_inlines(&dest[ix].inlines, loc.offset);
                dest[ix].inlines = left;
                let indent = dest[ix].indent;
                dest.insert(
                    ix + 1,
                    ListItem {
                        indent,
                        checked: dest[ix].checked.map(|_| false),
                        inlines: right,
                    },
                );
                ix + 1
            };
            for (i, it) in items.into_iter().enumerate() {
                dest.insert(insert_at + i, it);
            }
        }
        self.caret_after(loc.node, Some(insert_at + n_items - 1), None, last_len)
    }

    fn split_text_node(&mut self, loc: Loc) {
        let inlines = self.inlines_at(loc).to_vec();
        let (left, right) = split_inlines(&inlines, loc.offset);
        match &mut self.nodes[loc.node].kind {
            NodeKind::Paragraph { inlines }
            | NodeKind::Heading { inlines, .. }
            | NodeKind::HtmlHeading { inlines, .. }
            | NodeKind::Details { inlines, .. }
            | NodeKind::Quote { inlines }
            | NodeKind::Alert { inlines, .. } => *inlines = left,
            _ => return,
        }
        self.nodes.insert(
            loc.node + 1,
            Node {
                id: next_id(),
                kind: NodeKind::Paragraph { inlines: right },
            },
        );
    }

    fn split_list_at(&mut self, loc: Loc) {
        let NodeKind::List { items, ordered } = &mut self.nodes[loc.node].kind else {
            return;
        };
        let ix = loc.item.unwrap_or(0).min(items.len().saturating_sub(1));
        let ordered = *ordered;
        let (left, right) = split_inlines(&items[ix].inlines, loc.offset);
        let indent = items[ix].indent;
        let checked = items[ix].checked;
        items[ix].inlines = left;
        let mut after: Vec<ListItem> = items.drain(ix + 1..).collect();
        after.insert(
            0,
            ListItem {
                indent,
                checked: checked.map(|_| false),
                inlines: right,
            },
        );
        if inlines_len(&items[ix].inlines) == 0 {
            items.remove(ix);
        }
        let ni = loc.node;
        if items.is_empty() {
            self.nodes[ni].kind = NodeKind::Paragraph { inlines: vec![] };
        }
        self.nodes.insert(
            ni + 1,
            Node {
                id: next_id(),
                kind: NodeKind::List {
                    ordered,
                    items: after,
                },
            },
        );
    }

    fn merge_kind_into(&mut self, keep: usize, incoming: Node) {
        let right = match incoming.kind {
            NodeKind::Paragraph { inlines }
            | NodeKind::Heading { inlines, .. }
            | NodeKind::HtmlHeading { inlines, .. }
            | NodeKind::Details { inlines, .. }
            | NodeKind::Quote { inlines }
            | NodeKind::Alert { inlines, .. } => inlines,
            _ => return,
        };
        match &mut self.nodes[keep].kind {
            NodeKind::Paragraph { inlines }
            | NodeKind::Heading { inlines, .. }
            | NodeKind::HtmlHeading { inlines, .. }
            | NodeKind::Details { inlines, .. }
            | NodeKind::Quote { inlines }
            | NodeKind::Alert { inlines, .. } => inlines.extend(right),
            NodeKind::List { items, .. } => {
                if let Some(last) = items.last_mut() {
                    last.inlines.extend(right);
                }
            }
            _ => {}
        }
    }

    fn content_len_at(&self, loc: Loc) -> usize {
        match &self.nodes.get(loc.node).map(|n| &n.kind) {
            Some(NodeKind::List { items, .. }) => items
                .get(loc.item.unwrap_or(0))
                .map(|i| inlines_len(&i.inlines))
                .unwrap_or(0),
            Some(NodeKind::Code { text, .. }) => text.len(),
            Some(NodeKind::Html { raw }) => raw.len(),
            Some(NodeKind::Paragraph { inlines })
            | Some(NodeKind::Heading { inlines, .. })
            | Some(NodeKind::HtmlHeading { inlines, .. })
            | Some(NodeKind::Details { inlines, .. })
            | Some(NodeKind::Quote { inlines })
            | Some(NodeKind::Alert { inlines, .. }) => inlines_len(inlines),
            _ => 0,
        }
    }

    fn is_last_slot(&self, loc: Loc) -> bool {
        match &self.nodes.get(loc.node).map(|n| &n.kind) {
            Some(NodeKind::List { items, .. }) => loc.item.unwrap_or(0) + 1 >= items.len(),
            _ => true,
        }
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
        let mut offset = {
            // offset may have been consumed by a shortcut
            let inlines = self.inlines_at(Loc {
                node: node_ix,
                item,
                cell: loc.cell,
                offset: 0,
            });
            let len = inlines_len(inlines);
            len.min(loc.offset + text.len())
        };
        if text.contains('`') {
            offset = self.maybe_close_backtick_code(node_ix, item, offset);
        }
        self.caret_after(node_ix, item, loc.cell, offset)
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
                if loc_b.node == start.node && loc_b.item == start.item && loc_b.cell == start.cell
                {
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
        let num_to_drop = end.node.saturating_sub(start.node + 1);
        for _ in 0..num_to_drop {
            if start.node + 1 < self.nodes.len() {
                self.nodes.remove(start.node + 1);
            }
        }
        let p = self.project();
        let end = self.loc(b.saturating_sub(1).min(p.display.len().saturating_sub(1)));
        if end.node > start.node && end.node < self.nodes.len() {
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
        if self.nodes.is_empty() {
            self.nodes.push(Node {
                id: next_id(),
                kind: NodeKind::Paragraph { inlines: vec![] },
            });
            return 0;
        }
        self.caret_after(start.node, start.item, start.cell, start.offset)
    }

    /// Linewise delete for `V`+`d`, Helix `x`+`d` etc: remove every unit
    /// (block / list item) fully covered by `range`, no merging.
    /// The generic `delete_display` path empties the first node, no-ops the
    /// last (`end_off == 0` when the range ends at the next line's start),
    /// then merges — so deleting 2 full lines deleted only the first.
    /// Falls back to `delete_display` when nothing is fully covered (e.g. a
    /// single row inside a multi-line code block) or details are involved.
    pub fn delete_linewise_range(&mut self, range: Range<usize>) -> usize {
        let a = range.start.min(range.end);
        let b = range.end.max(range.start);
        if a == b {
            return a;
        }
        let p0 = self.project();
        let us = units(&p0);
        let mut full: Vec<usize> = Vec::new();
        let mut partial = false;
        for (idx, u) in us.iter().enumerate() {
            let r = unit_display(&p0, *u);
            if r.start >= a && r.end <= b {
                // Zero-length empty slots at the very edge (`r.start ==
                // r.end == b`) are separators, not covered units.
                if r.start < r.end || r.start < b {
                    full.push(idx);
                }
            } else if r.start < b && r.end > a {
                partial = true;
                break;
            }
        }
        if full.is_empty() {
            return self.delete_display(a..b);
        }
        // Details pairs stay on the generic path (never orphan `</details>`).
        let involves_details = full.iter().any(|&fi| {
            let u = us[fi];
            matches!(
                self.nodes.get(u.block).map(|n| &n.kind),
                Some(NodeKind::Details { .. }) | Some(NodeKind::DetailsClose)
            )
        });
        if involves_details {
            return self.delete_display(a..b);
        }
        if partial {
            // Mixed full + partial edges: drop the interior whole units,
            // then let the generic path handle the edge fragments.
            // Only interior units are strictly inside after trimming the
            // first/last overlapped units; simplest safe route: delete fully
            // covered units that don't touch the range edges... For now,
            // fall back — pure whole-unit ranges (the `x`/`V` case) are the
            // reported bug; partial+full mixes keep merge semantics.
            // Check whether the partials are just the trailing-newline edge:
            // if every partial is fully inside except separator, still safe.
            // Conservative: fall back to generic when any true partial.
            return self.delete_display(a..b);
        }
        for &fi in full.iter().rev() {
            let u = us[fi];
            match u.item {
                Some(i) => {
                    if let NodeKind::List { items, .. } = &mut self.nodes[u.block].kind {
                        if items.len() <= 1 {
                            self.nodes.remove(u.block);
                        } else if i < items.len() {
                            items.remove(i);
                        }
                    }
                }
                None => {
                    if u.block < self.nodes.len() {
                        self.nodes.remove(u.block);
                    }
                }
            }
        }
        if self.nodes.is_empty() {
            self.nodes.push(Node {
                id: next_id(),
                kind: NodeKind::Paragraph { inlines: vec![] },
            });
            return 0;
        }
        // `a` was the start of the first removed unit: the next unit now
        // sits there (or EOF when deleting at the end).
        let p1 = self.project();
        a.min(p1.display.len())
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

    /// Vim `r`: replace `count` display chars at `caret` with `ch`.
    /// Newlines are skipped (never replaced), mirroring motion semantics.
    /// Caret stays put.
    pub fn replace_chars(&mut self, caret: usize, count: usize, ch: char) -> usize {
        let p = self.project();
        let d = caret.min(p.display.len());
        let loc = self.loc(d);
        let text = inlines_text(self.inlines_at(loc));
        let mut end = loc.offset;
        let mut n = 0usize;
        for c in text[loc.offset.min(text.len())..].chars().take(count.max(1)) {
            if c == '\n' {
                end += c.len_utf8();
                continue;
            }
            end += c.len_utf8();
            n += 1;
        }
        if n == 0 {
            return d;
        }
        let repl: String = std::iter::repeat(ch).take(n).collect();
        let inlines = self.inlines_at_mut(loc);
        delete_inlines(inlines, loc.offset, end);
        insert_inlines(inlines, loc.offset, &repl, Marks::default());
        // Re-resolve: deletion/insertion may have merged runs.
        let p2 = self.project();
        let lb = Self::target_display(&p2, &loc);
        (lb.start + loc.offset).min(p2.display.len())
    }

    /// Helix replace: every non-newline char in `range` becomes `ch`.
    pub fn replace_range(&mut self, range: Range<usize>, ch: char) -> usize {
        let a = range.start.min(range.end);
        let b = range.end.max(range.start);
        if a >= b {
            return self.replace_chars(a, 1, ch);
        }
        let p = self.project();
        let targets = self.mark_targets(&p, a, b);
        let ranges: Vec<(Loc, usize, usize)> = targets
            .iter()
            .filter_map(|t| {
                let disp = Self::target_display(&p, t);
                let s = a.max(disp.start);
                let e = b.min(disp.end);
                (s < e).then(|| (*t, s - disp.start, e - disp.start))
            })
            .collect();
        drop(p);
        for (loc, off0, off1) in ranges {
            let text = inlines_text(self.inlines_at(loc));
            let end = off1.min(text.len());
            let mut out = String::new();
            for c in text[off0.min(text.len())..end].chars() {
                if c == '\n' {
                    out.push('\n');
                } else {
                    out.push(ch);
                }
            }
            let inlines = self.inlines_at_mut(loc);
            delete_inlines(inlines, off0, end);
            insert_inlines(inlines, off0, &out, Marks::default());
        }
        a
    }

    /// Vim `J`: join the unit under `caret` with the next `count` units.
    pub fn join_lines(&mut self, caret: usize, count: usize) -> usize {
        let mut caret = caret;
        for _ in 0..count.max(1) {
            let p = self.project();
            let us = units(&p);
            let Some(pos) = us.iter().position(|u| {
                let r = unit_display(&p, *u);
                caret >= r.start && caret <= r.end
            }) else {
                break;
            };
            if pos + 1 >= us.len() {
                break;
            }
            caret = self.join_unit_with_next(us[pos], us[pos + 1]);
        }
        caret
    }

    /// Visual `J`: join every unit overlapping `range`.
    pub fn join_range(&mut self, range: Range<usize>) -> usize {
        let a = range.start.min(range.end);
        let b = range.end.max(range.start);
        let p = self.project();
        let us = units(&p);
        let mut n = 0usize;
        for u in &us {
            let r = unit_display(&p, *u);
            if r.start < b && r.end > a {
                n += 1;
            }
        }
        drop(p);
        let mut caret = a;
        for _ in 0..n.saturating_sub(1).max(1) {
            let p = self.project();
            let us = units(&p);
            let Some(pos) = us.iter().position(|u| {
                let r = unit_display(&p, *u);
                caret >= r.start && caret <= r.end
            }) else {
                break;
            };
            if pos + 1 >= us.len() {
                break;
            }
            caret = self.join_unit_with_next(us[pos], us[pos + 1]);
        }
        caret
    }

    fn join_unit_with_next(&mut self, cur: Unit, next: Unit) -> usize {
        // Same list node, both items: fold next item text into current.
        if cur.block == next.block {
            if let (Some(i), Some(j)) = (cur.item, next.item) {
                let (left_len, joint) = {
                    let NodeKind::List { items, .. } = &self.nodes[cur.block].kind else {
                        return 0;
                    };
                    let left = inlines_text(&items[i.min(items.len() - 1)].inlines);
                    let right = inlines_text(&items[j.min(items.len() - 1)].inlines);
                    let space = (!left.is_empty() && !right.is_empty()).then_some(" ");
                    (
                        left.len() + space.map(|s: &str| s.len()).unwrap_or(0),
                        format!("{left}{}{right}", space.unwrap_or_default()),
                    )
                };
                let NodeKind::List { items, .. } = &mut self.nodes[cur.block].kind else {
                    return 0;
                };
                let j = j.min(items.len() - 1);
                let i = i.min(j);
                items[i].inlines = vec![Inline {
                    text: joint,
                    marks: Marks::default(),
                }];
                if j > i {
                    items.remove(j);
                }
                return self.caret_after(cur.block, Some(i), None, left_len);
            }
        }
        // Cross-node: append next unit's text to the end of current, drop next.
        let p0 = self.project();
        let join_at = unit_display(&p0, cur).end.min(p0.display.len());
        drop(p0);
        let right_text = match next.item {
            Some(j) => match &self.nodes[next.block].kind {
                NodeKind::List { items, .. } => items
                    .get(j)
                    .map(|it| inlines_text(&it.inlines))
                    .unwrap_or_default(),
                _ => String::new(),
            },
            None => match &self.nodes[next.block].kind {
                NodeKind::Paragraph { inlines }
                | NodeKind::Heading { inlines, .. }
                | NodeKind::HtmlHeading { inlines, .. }
                | NodeKind::Details { inlines, .. }
                | NodeKind::Quote { inlines }
                | NodeKind::Alert { inlines, .. } => inlines_text(inlines),
                NodeKind::Code { text, .. } => text.clone(),
                _ => String::new(),
            },
        };
        let keep_end = self.node_end(cur.block, cur.item);
        let node = cur.block;
        let item = cur.item;
        {
            let inlines = self.inlines_at_mut(Loc {
                node,
                item,
                cell: None,
                offset: 0,
            });
            let cur_text = inlines_text(inlines);
            let space = (!cur_text.is_empty() && !right_text.is_empty()).then_some(" ");
            let joint = format!("{cur_text}{}{right_text}", space.unwrap_or_default());
            *inlines = vec![Inline {
                text: joint,
                marks: Marks::default(),
            }];
        }
        if next.block != cur.block {
            if let Some(j) = next.item {
                if let NodeKind::List { items, .. } = &mut self.nodes[next.block].kind {
                    if j < items.len() {
                        if items.len() <= 1 {
                            self.nodes.remove(next.block);
                        } else {
                            items.remove(j);
                        }
                    }
                }
            } else if next.block < self.nodes.len() {
                // Only drop the node when it contributed nothing mergeable
                // (paragraph-like kinds are folded into `keep` by merge_nodes;
                // other kinds like tables/images must stay put).
                let mergeable = matches!(
                    self.nodes[next.block].kind,
                    NodeKind::Paragraph { .. }
                        | NodeKind::Heading { .. }
                        | NodeKind::HtmlHeading { .. }
                        | NodeKind::Details { .. }
                        | NodeKind::Quote { .. }
                        | NodeKind::Alert { .. }
                        | NodeKind::Code { .. }
                );
                if mergeable {
                    self.merge_nodes(cur.block, next.block);
                }
            }
        }
        let _ = keep_end;
        let n = self.project().display.len();
        join_at.min(n)
    }

    /// Toggle `mark` on exactly the cells of a table rectangle
    /// (display rows `r0..=r1`, cols `c0..=c1`). Returns true when applied.
    pub fn toggle_mark_table(
        &mut self,
        block: usize,
        r0: usize,
        r1: usize,
        c0: usize,
        c1: usize,
        mark: Mark,
    ) -> bool {
        let p = self.project();
        let Some(b) = p.blocks.get(block) else {
            return false;
        };
        let BlockExtra::Table { cells, .. } = &b.extra else {
            return false;
        };
        let (rlo, rhi) = (r0.min(r1), r0.max(r1));
        let (clo, chi) = (c0.min(c1), c0.max(c1));
        let targets: Vec<(Loc, Range<usize>)> = cells
            .iter()
            .filter(|c| c.row >= rlo && c.row <= rhi && c.col >= clo && c.col <= chi)
            .map(|c| {
                (
                    Loc {
                        node: block,
                        item: None,
                        cell: Some((c.row, c.col)),
                        offset: 0,
                    },
                    c.display.clone(),
                )
            })
            .collect();
        if targets.is_empty() {
            return false;
        }
        let mut total = 0usize;
        let mut marked = 0usize;
        for (_, disp) in &targets {
            for i in disp.start..disp.end {
                total += 1;
                if mark.has(p.marks_at(i, Affinity::Inside)) {
                    marked += 1;
                }
            }
        }
        let turn_on = !(total > 0 && marked == total);
        drop(p);
        for (loc, disp) in targets {
            let len = disp.end.saturating_sub(disp.start);
            let inlines = self.inlines_at_mut(loc);
            apply_mark_range(inlines, 0, len, mark, turn_on);
        }
        true
    }

    /// Insert an image block after the unit under `caret`.
    pub fn insert_image(&mut self, caret: usize, alt: String, src: String) -> usize {
        let loc = self.loc(caret);
        let at = loc.node + 1;
        self.nodes.insert(
            at.min(self.nodes.len()),
            Node {
                id: next_id(),
                kind: NodeKind::Image { alt, src },
            },
        );
        self.caret_after(at.min(self.nodes.len().saturating_sub(1)), None, None, 0)
    }

    /// Rewrite alt/src of the image block under display `caret` (media toolbar).
    pub fn update_image(&mut self, caret: usize, alt: String, src: String) -> usize {
        let loc = self.loc(caret);
        if let Some(node) = self.nodes.get_mut(loc.node) {
            if let NodeKind::Image {
                alt: a, src: s, ..
            } = &mut node.kind
            {
                *a = alt;
                *s = src;
            }
        }
        self.caret_after(loc.node, None, None, 0)
    }

    /// Rewrite the `src` attribute of an HTML `<video>` block under `caret`.
    pub fn update_html_video_src(&mut self, caret: usize, src: String) -> Option<usize> {
        let loc = self.loc(caret);
        let node = self.nodes.get_mut(loc.node)?;
        if let NodeKind::Html { raw } = &mut node.kind {
            *raw = replace_video_src(raw, &src)
                .or_else(|| replace_first_attr(raw, &["src", "href"], &src))?;
            return Some(self.caret_after(loc.node, None, None, 0));
        }
        None
    }

    pub fn enter(&mut self, caret: usize, hard: bool) -> usize {
        let loc = self.loc(caret);
        match &self.nodes[loc.node].kind {
            NodeKind::Code { .. } => {
                return self.insert_text(caret, None, "\n", Marks::default());
            }
            NodeKind::List { .. } => return self.enter_list(loc),
            NodeKind::Table { .. } => {
                return self.insert_text(caret, None, "\n", Marks::default());
            }
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
        // ```[lang] + Enter on an empty paragraph becomes a code block.
        // Any lang is accepted (unknown falls back to plain highlighting).
        if loc.item.is_none() {
            if let NodeKind::Paragraph { .. } = &self.nodes[loc.node].kind {
                let cur = self.inlines_at(loc).to_vec();
                if loc.offset == inlines_len(&cur) {
                    if let Some(lang) = parse_fence_lang(&inlines_text(&cur)) {
                        self.nodes[loc.node].kind = NodeKind::Code {
                            lang,
                            text: String::new(),
                            indent: 0,
                        };
                        return self.caret_after(loc.node, None, None, 0);
                    }
                }
            }
        }
        // Notion: Enter at the start of a heading inserts an empty block above
        // and keeps the heading text (does not demote the body to a paragraph).
        // Caret stays on the heading (`|Table`), not on the new empty line.
        if loc.offset == 0 {
            if matches!(self.nodes[loc.node].kind, NodeKind::Heading { .. } | NodeKind::HtmlHeading { .. } | NodeKind::Details { .. }) {
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
            NodeKind::Heading { inlines, .. }
            | NodeKind::HtmlHeading { inlines, .. }
            | NodeKind::Details { inlines, .. } => *inlines = left,
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
            NodeKind::Heading { .. }
            | NodeKind::HtmlHeading { .. }
            | NodeKind::Details { .. }
            | NodeKind::Quote { .. }
            | NodeKind::Alert { .. }
        ) && empty_right
        {
            if matches!(
                self.nodes[ni].kind,
                NodeKind::Quote { .. } | NodeKind::Alert { .. }
            ) && inlines_len(self.inlines_at(loc)) == 0
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

    /// Enter on a *collapsed* `<details>` summary: split the summary but
    /// place the new paragraph *after* the matching `</details>` so the
    /// caret lands on the visible block below instead of inside the hidden
    /// body (lost-cursor bug). Returns None when `caret` isn't on a Details
    /// node or is at offset 0 (offset 0 keeps the normal above-insert).
    pub fn enter_closed_details(&mut self, caret: usize) -> Option<usize> {
        let loc = self.loc(caret);
        if !matches!(self.nodes.get(loc.node)?.kind, NodeKind::Details { .. }) {
            return None;
        }
        if loc.offset == 0 {
            return None;
        }
        let inlines = self.inlines_at(loc).to_vec();
        let (left, right) = split_inlines(&inlines, loc.offset);
        if let NodeKind::Details { inlines, .. } = &mut self.nodes[loc.node].kind {
            *inlines = left;
        }
        // Depth-counted close so nested disclosures resolve correctly.
        let mut depth = 0usize;
        let mut close = None;
        for (j, n) in self.nodes.iter().enumerate().skip(loc.node) {
            match &n.kind {
                NodeKind::Details { .. } => depth += 1,
                NodeKind::DetailsClose => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        close = Some(j);
                        break;
                    }
                }
                _ => {}
            }
        }
        let at = close.map(|j| j + 1).unwrap_or(loc.node + 1);
        self.nodes.insert(
            at.min(self.nodes.len()),
            Node {
                id: next_id(),
                kind: NodeKind::Paragraph { inlines: right },
            },
        );
        Some(self.caret_after(at.min(self.nodes.len() - 1), None, None, 0))
    }

    /// `o` on a *collapsed* `<details>` summary: insert an empty paragraph
    /// *after* the matching `</details>` (summary left intact) so the new
    /// line lands outside the hidden body. Returns None when `caret` isn't
    /// on a Details node. When open, plain `open_line` already lands inside.
    pub fn open_after_details(&mut self, caret: usize) -> Option<usize> {
        let loc = self.loc(caret);
        if !matches!(self.nodes.get(loc.node)?.kind, NodeKind::Details { .. }) {
            return None;
        }
        // Depth-counted close so nested disclosures resolve correctly.
        let mut depth = 0usize;
        let mut close = None;
        for (j, n) in self.nodes.iter().enumerate().skip(loc.node) {
            match &n.kind {
                NodeKind::Details { .. } => depth += 1,
                NodeKind::DetailsClose => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        close = Some(j);
                        break;
                    }
                }
                _ => {}
            }
        }
        let at = close.map(|j| j + 1).unwrap_or(loc.node + 1);
        self.nodes.insert(
            at.min(self.nodes.len()),
            Node {
                id: next_id(),
                kind: NodeKind::Paragraph { inlines: vec![] },
            },
        );
        Some(self.caret_after(at.min(self.nodes.len() - 1), None, None, 0))
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
        // Inside a disclosure, backspace ejects the block after `</details>`
        // (same as shift-tab) instead of merging into the previous body node.
        // Lists keep their own item behavior.
        let is_list = matches!(self.nodes.get(loc.node).map(|n| &n.kind), Some(NodeKind::List { .. }));
        if !is_list {
            if let Some(nix) = self.eject_from_details(loc.node) {
                return Some(self.caret_after(nix, self.last_item(nix), None, 0));
            }
        }
        match &self.nodes[loc.node].kind {
            NodeKind::List { .. } => return Some(self.backspace_list(loc)),
            NodeKind::Heading { .. } | NodeKind::HtmlHeading { .. } | NodeKind::Code { .. } => {
                if loc.node > 0 {
                    // Empty block above a heading: drop the empty slot and keep
                    // the heading (`# Table`). Merging into an empty paragraph
                    // would demote the heading to plain text.
                    let prev_empty = matches!(
                        &self.nodes[loc.node - 1].kind,
                        NodeKind::Paragraph { inlines } if inlines_len(inlines) == 0
                    );
                    if prev_empty && matches!(self.nodes[loc.node].kind, NodeKind::Heading { .. } | NodeKind::HtmlHeading { .. }) {
                        let heading = loc.node;
                        self.nodes.remove(loc.node - 1);
                        return Some(self.caret_after(heading - 1, None, None, 0));
                    }
                    return Some(self.merge_nodes(loc.node - 1, loc.node));
                }
                let inlines = match &self.nodes[loc.node].kind {
                    NodeKind::Heading { inlines, .. } | NodeKind::HtmlHeading { inlines, .. } => inlines.clone(),
                    NodeKind::Code { text, .. } => vec![Inline {
                        text: text.clone(),
                        marks: Marks::default(),
                    }],
                    _ => vec![],
                };
                self.nodes[loc.node].kind = NodeKind::Paragraph { inlines };
                return Some(self.caret_after(loc.node, None, None, 0));
            }
            NodeKind::Details { .. } | NodeKind::Quote { .. } | NodeKind::Alert { .. } => {
                // Two-step erase (like list items): first backspace strips the
                // wrapper and keeps the text as a paragraph; a second one
                // merges/deletes. One-shot merging would nuke the disclosure.
                if loc.node > 0 {
                    let prev_empty = matches!(
                        &self.nodes[loc.node - 1].kind,
                        NodeKind::Paragraph { inlines } if inlines_len(inlines) == 0
                    );
                    if prev_empty && matches!(self.nodes[loc.node].kind, NodeKind::Details { .. }) {
                        let cur = loc.node;
                        self.nodes.remove(cur - 1);
                        return Some(self.caret_after(cur - 1, None, None, 0));
                    }
                }
                let inlines = match &self.nodes[loc.node].kind {
                    NodeKind::Details { inlines, .. }
                    | NodeKind::Quote { inlines }
                    | NodeKind::Alert { inlines, .. } => inlines.clone(),
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
                    // The node above may be `</details>` chrome (zero-height):
                    // land on the last body node, or the summary when empty.
                    let prev = self.visible_keep_above(loc.node - 1);
                    let len = self.node_text_len(prev);
                    return Some(self.caret_after(prev, self.last_item(prev), None, len));
                }
                if loc.node == 0 {
                    return None;
                }
                return Some(self.merge_nodes(loc.node - 1, loc.node));
            }
            NodeKind::Rule | NodeKind::Image { .. } | NodeKind::Html { .. } | NodeKind::DetailsClose => {
                if loc.node == 0 {
                    self.nodes[0].kind = NodeKind::Paragraph { inlines: vec![] };
                    return Some(0);
                }
                self.nodes.remove(loc.node);
                let prev = self.visible_keep_above(loc.node - 1);
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
        let targets = self.mark_targets(&p, a, b);
        if targets.is_empty() {
            return None;
        }

        let mut total_chars = 0usize;
        let mut marked_chars = 0usize;
        for t in &targets {
            let disp = Self::target_display(&p, t);
            let overlap_start = a.max(disp.start);
            let overlap_end = b.min(disp.end);
            if overlap_start < overlap_end {
                for i in overlap_start..overlap_end {
                    total_chars += 1;
                    if mark.has(p.marks_at(i, Affinity::Inside)) {
                        marked_chars += 1;
                    }
                }
            }
        }
        let all = total_chars > 0 && marked_chars == total_chars;
        let any = marked_chars > 0;
        let wordish = p
            .display
            .get(a..b)
            .is_some_and(|s| !s.chars().any(char::is_whitespace));
        // Partially-marked contiguous words clear instead of stacking.
        let turn_on = !(all || (any && wordish));

        let ranges: Vec<(Loc, usize, usize)> = targets
            .iter()
            .filter_map(|t| {
                let disp = Self::target_display(&p, t);
                let overlap_start = a.max(disp.start);
                let overlap_end = b.min(disp.end);
                if overlap_start < overlap_end {
                    Some((
                        *t,
                        overlap_start - disp.start,
                        overlap_end - disp.start,
                    ))
                } else {
                    None
                }
            })
            .collect();
        drop(p);

        let mut affected = 0;
        for (loc, off0, off1) in ranges {
            let inlines = self.inlines_at_mut(loc);
            apply_mark_range(inlines, off0, off1, mark, turn_on);
            affected += 1;
        }
        if affected == 0 {
            None
        } else {
            Some(a..b)
        }
    }

    fn mark_targets(&self, p: &Projection, a: usize, b: usize) -> Vec<Loc> {
        let mut out = Vec::new();
        for (bi, block) in p.blocks.iter().enumerate() {
            match &block.extra {
                BlockExtra::Table { cells, .. } => {
                    for c in cells {
                        if a < c.display.end && b > c.display.start {
                            out.push(Loc {
                                node: bi,
                                item: None,
                                cell: Some((c.row, c.col)),
                                offset: 0,
                            });
                        }
                    }
                }
                BlockExtra::List { items, .. } => {
                    for (i, it) in items.iter().enumerate() {
                        if a < it.display.end && b > it.display.start {
                            out.push(Loc {
                                node: bi,
                                item: Some(i),
                                cell: None,
                                offset: 0,
                            });
                        }
                    }
                }
                _ => {
                    if a < block.display.end && b > block.display.start {
                        out.push(Loc {
                            node: bi,
                            item: None,
                            cell: None,
                            offset: 0,
                        });
                    }
                }
            }
        }
        out
    }

    fn target_display(p: &Projection, loc: &Loc) -> Range<usize> {
        let Some(block) = p.blocks.get(loc.node) else {
            return 0..0;
        };
        if let (Some((r, c)), BlockExtra::Table { cells, .. }) = (loc.cell, &block.extra) {
            if let Some(cell) = cells.iter().find(|x| x.row == r && x.col == c) {
                return cell.display.clone();
            }
        }
        if let (Some(i), BlockExtra::List { items, .. }) = (loc.item, &block.extra) {
            if let Some(it) = items.get(i) {
                return it.display.clone();
            }
        }
        block.display.clone()
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
        if loc.node != loc_b.node || loc.item != loc_b.item || loc.cell != loc_b.cell {
            return b;
        }
        let id = if url.is_empty() {
            None
        } else {
            let id = self.links.len() as u32;
            self.links.push(url.to_string());
            Some(id)
        };
        let inlines = self.inlines_at_mut(loc);
        set_link_range(inlines, loc.offset, loc_b.offset, id);
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

    /// Insert a column before/after the cell under `caret`. Caret stays in the
    /// focused cell (shifted if a column was inserted to the left).
    pub fn insert_table_col(&mut self, caret: usize, before: bool) -> Option<usize> {
        let loc = self.loc(caret);
        let (r, c) = loc.cell?;
        {
            let NodeKind::Table { headers, rows } = &mut self.nodes[loc.node].kind else {
                return None;
            };
            let at = (if before { c } else { c + 1 }).min(headers.len());
            headers.insert(at, vec![]);
            for row in rows.iter_mut() {
                while row.len() < at {
                    row.push(vec![]);
                }
                row.insert(at.min(row.len()), vec![]);
            }
        }
        let new_c = if before { c + 1 } else { c };
        Some(self.caret_after(loc.node, None, Some((r, new_c)), loc.offset))
    }

    /// Insert a body row before/after the cell under `caret`. Header row stays
    /// put; "above" the header inserts the first body row.
    pub fn insert_table_row(&mut self, caret: usize, before: bool) -> Option<usize> {
        let loc = self.loc(caret);
        let (r, c) = loc.cell?;
        let new_r = {
            let NodeKind::Table { headers, rows } = &mut self.nodes[loc.node].kind else {
                return None;
            };
            let cols = headers.len().max(1);
            let empty = vec![vec![]; cols];
            if r == 0 {
                rows.insert(0, empty);
                1
            } else {
                let body = r - 1;
                let at = if before { body } else { body + 1 };
                rows.insert(at.min(rows.len()), empty);
                if before {
                    r + 1
                } else {
                    r
                }
            }
        };
        Some(self.caret_after(loc.node, None, Some((new_r, c)), loc.offset))
    }

    pub fn delete_table_col(&mut self, caret: usize) -> Option<usize> {
        let loc = self.loc(caret);
        let (r, c) = loc.cell?;
        {
            let NodeKind::Table { headers, rows } = &mut self.nodes[loc.node].kind else {
                return None;
            };
            if headers.len() <= 1 {
                headers.clear();
                headers.push(vec![]);
                for row in rows.iter_mut() {
                    row.clear();
                    row.push(vec![]);
                }
            } else {
                if c < headers.len() {
                    headers.remove(c);
                }
                for row in rows.iter_mut() {
                    if c < row.len() {
                        row.remove(c);
                    }
                }
            }
        }
        let max_c = match &self.nodes[loc.node].kind {
            NodeKind::Table { headers, .. } => headers.len().saturating_sub(1),
            _ => 0,
        };
        Some(self.caret_after(loc.node, None, Some((r, c.min(max_c))), 0))
    }

    pub fn delete_table_row(&mut self, caret: usize) -> Option<usize> {
        let loc = self.loc(caret);
        let (r, c) = loc.cell?;
        {
            let NodeKind::Table { rows, .. } = &mut self.nodes[loc.node].kind else {
                return None;
            };
            if r == 0 {
                if !rows.is_empty() {
                    rows.remove(0);
                }
            } else {
                let body = r - 1;
                if body < rows.len() {
                    rows.remove(body);
                }
            }
        }
        let max_r = match &self.nodes[loc.node].kind {
            NodeKind::Table { rows, .. } => rows.len(),
            _ => 0,
        };
        let new_r = if r == 0 { 0 } else { r.saturating_sub(1).min(max_r) };
        Some(self.caret_after(loc.node, None, Some((new_r, c)), 0))
    }

    /// Delete body rows in `[r0, r1]` (display rows; 0 is the header). Header is
    /// skipped. Caret lands in the row that replaced the first deleted body row.
    pub fn delete_table_rows(&mut self, caret: usize, r0: usize, r1: usize) -> Option<usize> {
        let loc = self.loc(caret);
        let (_, c) = loc.cell?;
        let (lo, hi) = (r0.min(r1), r0.max(r1));
        {
            let NodeKind::Table { rows, .. } = &mut self.nodes[loc.node].kind else {
                return None;
            };
            let body_lo = lo.saturating_sub(1).min(rows.len());
            let body_hi = if hi == 0 {
                // Header-only: same as deleting the first body row.
                0
            } else {
                hi.saturating_sub(1)
            };
            if rows.is_empty() {
                return Some(self.caret_after(loc.node, None, Some((0, c)), 0));
            }
            let from = body_lo.min(rows.len().saturating_sub(1));
            let to = body_hi.min(rows.len().saturating_sub(1));
            if from <= to {
                rows.drain(from..=to);
            }
        }
        let max_r = match &self.nodes[loc.node].kind {
            NodeKind::Table { rows, .. } => rows.len(),
            _ => 0,
        };
        let new_r = if lo == 0 {
            0
        } else {
            lo.saturating_sub(1).min(max_r).max(1).min(max_r)
        };
        Some(self.caret_after(loc.node, None, Some((new_r, c)), 0))
    }

    pub fn delete_table_cols(&mut self, caret: usize, c0: usize, c1: usize) -> Option<usize> {
        let loc = self.loc(caret);
        let (r, _) = loc.cell?;
        let (lo, hi) = (c0.min(c1), c0.max(c1));
        {
            let NodeKind::Table { headers, rows } = &mut self.nodes[loc.node].kind else {
                return None;
            };
            let last = headers.len().saturating_sub(1);
            let from = lo.min(last);
            let to = hi.min(last);
            if from == 0 && to == last {
                headers.clear();
                headers.push(vec![]);
                for row in rows.iter_mut() {
                    row.clear();
                    row.push(vec![]);
                }
            } else {
                for i in (from..=to).rev() {
                    if headers.len() <= 1 {
                        break;
                    }
                    if i < headers.len() {
                        headers.remove(i);
                    }
                    for row in rows.iter_mut() {
                        if i < row.len() {
                            row.remove(i);
                        }
                    }
                }
            }
        }
        let max_c = match &self.nodes[loc.node].kind {
            NodeKind::Table { headers, .. } => headers.len().saturating_sub(1),
            _ => 0,
        };
        Some(self.caret_after(loc.node, None, Some((r, lo.min(max_c))), 0))
    }

    pub fn delete_table(&mut self, caret: usize) -> Option<usize> {
        let loc = self.loc(caret);
        if !matches!(
            self.nodes.get(loc.node).map(|n| &n.kind),
            Some(NodeKind::Table { .. })
        ) {
            return None;
        }
        self.nodes[loc.node].kind = NodeKind::Paragraph { inlines: vec![] };
        Some(self.caret_after(loc.node, None, None, 0))
    }

    /// Tab between cells. At the last cell, append a body row (tree mutation).
    pub fn table_tab(&mut self, caret: usize, shift: bool) -> Option<usize> {
        let loc = self.loc(caret);
        let (r, c) = loc.cell?;
        let (headers_len, body_len) = match &self.nodes.get(loc.node)?.kind {
            NodeKind::Table { headers, rows } => (headers.len().max(1), rows.len()),
            _ => return None,
        };
        let total_rows = body_len + 1;
        if shift {
            if c > 0 {
                return Some(self.caret_after(loc.node, None, Some((r, c - 1)), 0));
            }
            if r > 0 {
                return Some(self.caret_after(
                    loc.node,
                    None,
                    Some((r - 1, headers_len - 1)),
                    0,
                ));
            }
            return Some(self.caret_after(loc.node, None, Some((0, 0)), 0));
        }
        if c + 1 < headers_len {
            return Some(self.caret_after(loc.node, None, Some((r, c + 1)), 0));
        }
        if r + 1 < total_rows {
            return Some(self.caret_after(loc.node, None, Some((r + 1, 0)), 0));
        }
        {
            if let NodeKind::Table { headers, rows } = &mut self.nodes[loc.node].kind {
                rows.push(vec![vec![]; headers.len().max(1)]);
            }
        }
        Some(self.caret_after(loc.node, None, Some((r + 1, 0)), 0))
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
        us.get(at).map(|u| unit_display(&p, *u).start).unwrap_or(0)
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
        let at = if above { loc.node } else { loc.node + 1 };        self.nodes.insert(
            at,
            Node {
                id: next_id(),
                kind: NodeKind::Paragraph { inlines: vec![] },
            },
        );
        self.caret_after(at, None, None, 0)
    }

    fn tab_item_at(&mut self, node: usize, ix: usize, outdent: bool) -> bool {
        let NodeKind::List { items, .. } = &mut self.nodes[node].kind else {
            return false;
        };
        if ix >= items.len() {
            return false;
        }
        let end = list_subtree_end(items, ix);
        if outdent {
            if items[ix].indent == 0 {
                return false;
            }
            for item in &mut items[ix..end] {
                if item.indent > 0 {
                    item.indent -= 1;
                }
            }
            true
        } else {
            let max = if ix == 0 {
                items[ix].indent + 1
            } else {
                items[ix - 1].indent + 1
            };
            if items[ix].indent >= max {
                return false;
            }
            for item in &mut items[ix..end] {
                item.indent += 1;
            }
            true
        }
    }

    pub fn tab(&mut self, caret: usize, outdent: bool) -> Option<usize> {
        let loc = self.loc(caret);
        // `<details>` bodies behave like list nesting: shift-tab ejects the
        // block after the matching `</details>`; tab on the block directly
        // after a `</details>` adopts it as the last body block. This covers
        // the Tab key and Helix/Vim `>`/`<` (both route through `doc.tab`).
        if outdent {
            // Nested list items outdent within the list first; only
            // top-level blocks eject out of the disclosure.
            let mut list_nested = false;
            if let NodeKind::List { items, .. } = &self.nodes[loc.node].kind {
                if let Some(ix) = loc.item {
                    list_nested = items.get(ix).is_some_and(|it| it.indent > 0);
                }
            }
            if !list_nested {
                if let Some(nix) = self.eject_from_details(loc.node) {
                    return Some(self.caret_after(nix, self.last_item(nix), None, loc.offset));
                }
            }
        } else if self.adopt_into_details(loc.node) {
            return Some(self.caret_after(loc.node - 1, self.last_item(loc.node - 1), None, loc.offset));
        }
        let ix = loc.item?;
        if self.tab_item_at(loc.node, ix, outdent) {
            Some(self.caret_after(loc.node, Some(ix), None, loc.offset))
        } else {
            None
        }
    }

    pub fn tab_selection(&mut self, sel: Range<usize>, outdent: bool) -> Option<Range<usize>> {
        let a = sel.start.min(sel.end);
        let b = sel.end.max(sel.start);
        if a >= b {
            return None;
        }
        let p = self.project();
        let us = units(&p);
        let mut modified = false;
        // Skip descendants already shifted with a selected ancestor.
        let mut skip: Option<(usize, usize)> = None;
        for u in us {
            let u_disp = unit_display(&p, u);
            let overlap_start = a.max(u_disp.start);
            let overlap_end = b.min(u_disp.end);
            if overlap_start < overlap_end {
                if let Some(item_ix) = u.item {
                    if skip.is_some_and(|(block, until)| block == u.block && item_ix < until) {
                        continue;
                    }
                    if self.tab_item_at(u.block, item_ix, outdent) {
                        modified = true;
                        let until = match &self.nodes[u.block].kind {
                            NodeKind::List { items, .. } => list_subtree_end(items, item_ix),
                            _ => item_ix + 1,
                        };
                        skip = Some((u.block, until));
                    }
                }
            }
        }
        if modified {
            Some(a..b)
        } else {
            None
        }
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
                | NodeKind::HtmlHeading { inlines, .. }
                | NodeKind::Details { inlines, .. }
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

    /// After typing a closing `` ` ``, wrap the span between the previous
    /// unmatched backtick on this block into an inline code mark and drop both
    /// delimiters. Empty `` `` is left alone (fence / escape).
    fn maybe_close_backtick_code(
        &mut self,
        node: usize,
        item: Option<usize>,
        caret: usize,
    ) -> usize {
        let inlines = match (&mut self.nodes[node].kind, item) {
            (NodeKind::Paragraph { inlines }, None)
            | (NodeKind::Heading { inlines, .. }, None)
            | (NodeKind::HtmlHeading { inlines, .. }, None)
            | (NodeKind::Details { inlines, .. }, None)
            | (NodeKind::Quote { inlines }, None)
            | (NodeKind::Alert { inlines, .. }, None) => inlines,
            (NodeKind::List { items, .. }, Some(i)) => match items.get_mut(i) {
                Some(it) => &mut it.inlines,
                None => return caret,
            },
            _ => return caret,
        };
        let text = inlines_text(inlines);
        if caret == 0 || caret > text.len() {
            return caret;
        }
        if !text[..caret].ends_with('`') {
            return caret;
        }
        let close = caret - '`'.len_utf8();
        let Some(open) = text[..close].rfind('`') else {
            return caret;
        };
        let inner = open + '`'.len_utf8()..close;
        if inner.start >= inner.end {
            return caret;
        }
        if text[inner.clone()].contains('\n') {
            return caret;
        }
        // Opening backtick must not already sit inside a code run.
        let mut at = 0usize;
        for run in inlines.iter() {
            let end = at + run.text.len();
            if open >= at && open < end {
                if run.marks.code {
                    return caret;
                }
                break;
            }
            at = end;
        }
        let content = text[inner.clone()].to_string();
        delete_inlines(inlines, open, caret);
        let mut code_marks = Marks::default();
        code_marks.code = true;
        insert_inlines(inlines, open, &content, code_marks);
        *inlines = merge_inlines(std::mem::take(inlines));
        open + content.len()
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
            let cell = cells
                .iter()
                .find(|c| d >= c.display.start && d <= c.display.end)
                .or_else(|| cells.iter().rev().find(|c| d >= c.display.start))
                .or_else(|| cells.first());
            if let Some(c) = cell {
                let len = c.display.end.saturating_sub(c.display.start);
                return Loc {
                    node,
                    item: None,
                    cell: Some((c.row, c.col)),
                    offset: d.saturating_sub(c.display.start).min(len),
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
            | Some(NodeKind::HtmlHeading { inlines, .. })
            | Some(NodeKind::Details { inlines, .. })
            | Some(NodeKind::Quote { inlines })
            | Some(NodeKind::Alert { inlines, .. }) => inlines,
            Some(NodeKind::List { items, .. }) => items
                .get(loc.item.unwrap_or(0))
                .map(|i| i.inlines.as_slice())
                .unwrap_or(&[]),
            Some(NodeKind::Table { headers, rows }) => {
                let (r, c) = loc.cell.unwrap_or((0, 0));
                if r == 0 {
                    headers.get(c).map(|h| h.as_slice()).unwrap_or(&[])
                } else {
                    rows.get(r - 1)
                        .and_then(|row| row.get(c))
                        .map(|cell| cell.as_slice())
                        .unwrap_or(&[])
                }
            }
            _ => &[],
        }
    }

    fn inlines_at_mut(&mut self, loc: Loc) -> &mut Vec<Inline> {
        let convertible = !matches!(
            self.nodes[loc.node].kind,
            NodeKind::Paragraph { .. }
                | NodeKind::Heading { .. }
                | NodeKind::HtmlHeading { .. }
                | NodeKind::Details { .. }
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
            | NodeKind::HtmlHeading { inlines, .. }
            | NodeKind::Details { inlines, .. }
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
        cell: Option<(usize, usize)>,
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
        if let (Some((r, c)), BlockExtra::Table { cells, .. }) = (cell, &b.extra) {
            if let Some(tc) = cells.iter().find(|x| x.row == r && x.col == c) {
                let len = tc.display.end.saturating_sub(tc.display.start);
                return tc.display.start + offset.min(len);
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
            NodeKind::List { items, .. } => {
                items.last().map(|i| inlines_len(&i.inlines)).unwrap_or(0)
            }
            NodeKind::Paragraph { inlines }
            | NodeKind::Heading { inlines, .. }
            | NodeKind::HtmlHeading { inlines, .. }
            | NodeKind::Details { inlines, .. }
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
        // Merging into `</details>` chrome would land invisible (and drop the
        // text on the floor): merge into the last body node / summary instead.
        let keep = self.visible_keep_above(keep);
        if keep >= drop || drop >= self.nodes.len() {
            return self.caret_after(keep, None, None, self.node_text_len(keep));
        }
        let keep_len = self.node_text_len(keep);
        let right = match &self.nodes[drop].kind {
            NodeKind::Paragraph { inlines }
            | NodeKind::Heading { inlines, .. }
            | NodeKind::HtmlHeading { inlines, .. }
            | NodeKind::Details { inlines, .. }
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
            | NodeKind::HtmlHeading { inlines, .. }
            | NodeKind::Details { inlines, .. }
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

    /// Identity unless `keep` is `</details>` chrome (zero-height): then the
    /// last body node before the close, or the summary when the body is
    /// empty. Keeps backspace merges/landings on a visible block.
    fn visible_keep_above(&self, keep: usize) -> usize {
        if !matches!(self.nodes.get(keep).map(|n| &n.kind), Some(NodeKind::DetailsClose)) {
            return keep;
        }
        self.above_details_close(keep)
    }

    /// Visible node above the `</details>` at `close_ix`: last body node, or
    /// the summary when empty. Nested closes resolve recursively; malformed
    /// input (no opener) falls back to the node above.
    fn above_details_close(&self, close_ix: usize) -> usize {
        let mut depth = 0usize;
        let mut opener = None;
        for (j, n) in self.nodes.iter().enumerate().take(close_ix + 1).rev() {
            match &n.kind {
                NodeKind::DetailsClose => depth += 1,
                NodeKind::Details { .. } => {
                    if depth <= 1 {
                        opener = Some(j);
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        let Some(o) = opener else {
            return close_ix.saturating_sub(1);
        };
        let mut t = close_ix.saturating_sub(1);
        while t > o {
            if !matches!(self.nodes.get(t).map(|n| &n.kind), Some(NodeKind::DetailsClose)) {
                break;
            }
            t = self.above_details_close(t);
        }
        if t > o { t } else { o }
    }

    /// Innermost `<details>` opener enclosing `node` (strictly inside
    /// `open..=close`). None when `node` is itself a Details/DetailsClose
    /// or sits outside any disclosure. Nesting-aware via depth counting.
    fn enclosing_details_opener(&self, node: usize) -> Option<usize> {
        if node >= self.nodes.len() {
            return None;
        }
        if matches!(
            self.nodes[node].kind,
            NodeKind::Details { .. } | NodeKind::DetailsClose
        ) {
            return None;
        }
        let mut depth = 0usize;
        for (j, n) in self.nodes.iter().enumerate().take(node).rev() {
            match &n.kind {
                NodeKind::DetailsClose => depth += 1,
                NodeKind::Details { .. } => {
                    if depth == 0 {
                        return Some(j);
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        None
    }

    /// Matching `</details>` close for the opener at `opener_ix`
    /// (depth-counted for nesting). None when never closed.
    fn details_close_after(&self, opener_ix: usize) -> Option<usize> {
        if !matches!(self.nodes.get(opener_ix)?.kind, NodeKind::Details { .. }) {
            return None;
        }
        let mut depth = 0usize;
        for (j, n) in self.nodes.iter().enumerate().skip(opener_ix) {
            match &n.kind {
                NodeKind::Details { .. } => depth += 1,
                NodeKind::DetailsClose => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(j);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Shift-tab / backspace inside a disclosure: move `node` after the
    /// matching `</details>` (eject one level). Returns the new index.
    fn eject_from_details(&mut self, node: usize) -> Option<usize> {
        let o = self.enclosing_details_opener(node)?;
        let c = self.details_close_after(o)?;
        if node <= o || node >= c {
            return None;
        }
        let kind = self.nodes.remove(node);
        // Removal shifts the close down by one (`node < c` always holds).
        let at = c.min(self.nodes.len());
        self.nodes.insert(at, kind);
        Some(at)
    }

    /// Tab on the block directly after a `</details>`: adopt it as the last
    /// body block (move before the close). True when a move happened; the
    /// node lands at `node - 1`.
    fn adopt_into_details(&mut self, node: usize) -> bool {
        if node == 0 || node >= self.nodes.len() {
            return false;
        }
        if !matches!(self.nodes[node - 1].kind, NodeKind::DetailsClose) {
            return false;
        }
        if matches!(self.nodes[node].kind, NodeKind::DetailsClose) {
            return false;
        }
        // The close must belong to a real opener; otherwise leave it alone.
        let close_ix = node - 1;
        let mut depth = 0usize;
        let mut opener = None;
        for (j, n) in self.nodes.iter().enumerate().take(close_ix + 1).rev() {
            match &n.kind {
                NodeKind::DetailsClose => depth += 1,
                NodeKind::Details { .. } => {
                    if depth <= 1 {
                        opener = Some(j);
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        if opener.is_none() {
            return false;
        }
        let kind = self.nodes.remove(node);
        let at = close_ix.min(self.nodes.len());
        self.nodes.insert(at, kind);
        true
    }
}

fn slice_inlines(inlines: &[Inline], start: usize, end: usize) -> Vec<Inline> {
    let end = end.max(start);
    let (_, rest) = split_inlines(inlines, start);
    let (mid, _) = split_inlines(&rest, end.saturating_sub(start));
    mid
}

fn slice_kind_keep(kind: &NodeKind, inlines: Vec<Inline>, full: bool) -> NodeKind {
    if !full {
        return NodeKind::Paragraph { inlines };
    }
    match kind {
        NodeKind::Heading { level, .. } => NodeKind::Heading {
            level: *level,
            inlines,
        },
        NodeKind::HtmlHeading { level, .. } => NodeKind::HtmlHeading {
            level: *level,
            inlines,
        },
        NodeKind::Details { open, .. } => NodeKind::Details { inlines, open: *open },
        NodeKind::Quote { .. } => NodeKind::Quote { inlines },
        NodeKind::Alert { kind, .. } => NodeKind::Alert {
            kind: *kind,
            inlines,
        },
        _ => NodeKind::Paragraph { inlines },
    }
}

fn remap_inlines_links(inlines: &mut [Inline], dest: &mut Vec<String>, src: &[String]) {
    for run in inlines {
        if let Some(id) = run.marks.link {
            let url = src.get(id as usize).cloned().unwrap_or_default();
            let new_id = dest.len() as u32;
            dest.push(url);
            run.marks.link = Some(new_id);
        }
    }
}

fn remap_node_links(node: &mut Node, dest: &mut Vec<String>, src: &[String]) {
    match &mut node.kind {
        NodeKind::Paragraph { inlines }
        | NodeKind::Heading { inlines, .. }
        | NodeKind::HtmlHeading { inlines, .. }
        | NodeKind::Details { inlines, .. }
        | NodeKind::Quote { inlines }
        | NodeKind::Alert { inlines, .. } => remap_inlines_links(inlines, dest, src),
        NodeKind::List { items, .. } => {
            for it in items {
                remap_inlines_links(&mut it.inlines, dest, src);
            }
        }
        NodeKind::Table { headers, rows } => {
            for cell in headers {
                remap_inlines_links(cell, dest, src);
            }
            for row in rows {
                for cell in row {
                    remap_inlines_links(cell, dest, src);
                }
            }
        }
        _ => {}
    }
}

fn node_is_empty(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Paragraph { inlines }
        | NodeKind::Heading { inlines, .. }
        | NodeKind::HtmlHeading { inlines, .. }
        | NodeKind::Details { inlines, .. }
        | NodeKind::HtmlHeading { inlines, .. }
        | NodeKind::Details { inlines, .. }
        | NodeKind::Quote { inlines }
        | NodeKind::Alert { inlines, .. } => inlines_len(inlines) == 0,
        NodeKind::List { items, .. } => {
            items.is_empty() || items.iter().all(|i| inlines_len(&i.inlines) == 0)
        }
        NodeKind::Code { text, .. } => text.is_empty(),
        NodeKind::Html { raw } => raw.is_empty(),
        NodeKind::DetailsClose => false,
        NodeKind::Table { headers, rows } => {
            headers.iter().all(|c| inlines_len(c) == 0) && rows.is_empty()
        }
        NodeKind::Rule | NodeKind::Image { .. } => false,
    }
}

fn nodes_plain_text(nodes: &[Node]) -> String {
    nodes
        .iter()
        .map(|n| match &n.kind {
            NodeKind::Code { text, .. } => text.clone(),
            NodeKind::Html { raw } => raw.clone(),
            NodeKind::DetailsClose => "</details>".to_string(),
            NodeKind::Paragraph { inlines }
            | NodeKind::Heading { inlines, .. }
            | NodeKind::HtmlHeading { inlines, .. }
            | NodeKind::Details { inlines, .. }
            | NodeKind::HtmlHeading { inlines, .. }
            | NodeKind::Details { inlines, .. }
            | NodeKind::Quote { inlines }
            | NodeKind::Alert { inlines, .. } => inlines_text(inlines),
            NodeKind::List { items, .. } => items
                .iter()
                .map(|i| inlines_text(&i.inlines))
                .collect::<Vec<_>>()
                .join("\n"),
            NodeKind::Image { alt, .. } => alt.clone(),
            _ => String::new(),
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Exclusive end of `ix` plus nested items (indent strictly deeper than `ix`).
fn list_subtree_end(items: &[ListItem], ix: usize) -> usize {
    let Some(base) = items.get(ix).map(|it| it.indent) else {
        return ix;
    };
    let mut j = ix + 1;
    while j < items.len() && items[j].indent > base {
        j += 1;
    }
    j
}

fn flatten_list_items(nodes: Vec<Node>) -> Vec<ListItem> {
    nodes
        .into_iter()
        .flat_map(|n| match n.kind {
            NodeKind::List { items, .. } => items,
            _ => Vec::new(),
        })
        .collect()
}

fn can_merge_para(left: &Node, right: &Node) -> bool {
    matches!(left.kind, NodeKind::Paragraph { .. })
        && matches!(right.kind, NodeKind::Paragraph { .. })
}

fn node_content_len(n: &Node) -> usize {
    match &n.kind {
        NodeKind::List { items, .. } => items.last().map(|i| inlines_len(&i.inlines)).unwrap_or(0),
        NodeKind::Code { text, .. } => text.len(),
        NodeKind::Html { raw } => raw.len(),
        NodeKind::Paragraph { inlines }
        | NodeKind::Heading { inlines, .. }
        | NodeKind::HtmlHeading { inlines, .. }
        | NodeKind::Details { inlines, .. }
        | NodeKind::HtmlHeading { inlines, .. }
        | NodeKind::Details { inlines, .. }
        | NodeKind::Quote { inlines }
        | NodeKind::Alert { inlines, .. } => inlines_len(inlines),
        _ => 0,
    }
}

fn last_item_of(n: &Node) -> Option<usize> {
    match &n.kind {
        NodeKind::List { items, .. } if !items.is_empty() => Some(items.len() - 1),
        _ => None,
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
        BlockExtra::Code { lang, .. } => NodeKind::Code {
            lang: lang.clone(),
            text: p.display.get(b.display.clone()).unwrap_or("").to_string(),
            indent: match &b.extra {
                BlockExtra::Code { indent, .. } => *indent,
                _ => 0,
            },
        },
        BlockExtra::Table { cells, rows, cols } => {
            let mut headers = vec![vec![]; (*cols).max(1)];
            let mut body = vec![vec![vec![]; (*cols).max(1)]; (*rows).saturating_sub(1)];
            for c in cells {
                let ins = table_inlines_in(p, c.display.clone());
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
        BlockExtra::HtmlHeading(level) => NodeKind::HtmlHeading {
            level: *level,
            inlines: inlines_in(p, b.display.clone()),
        },
        BlockExtra::Details { open, .. } => NodeKind::Details {
            inlines: inlines_in(p, b.display.clone()),
            open: *open,
        },
        BlockExtra::DetailsClose => NodeKind::DetailsClose,
        BlockExtra::Text => NodeKind::Paragraph {
            inlines: inlines_in(p, b.display.clone()),
        },
    };
    Node {
        id: next_id(),
        kind,
    }
}

fn table_inlines_in(p: &Projection, range: Range<usize>) -> Vec<Inline> {
    let mut ins = inlines_in(p, range);
    for run in &mut ins {
        if run.text.contains(crate::display::TABLE_CELL_BR) {
            run.text = run.text.replace(crate::display::TABLE_CELL_BR, "\n");
        }
    }
    ins
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
        ("[] ", false, Some(false)),
        ("[ ] ", false, Some(false)),
        ("[x] ", false, Some(true)),
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

/// ` ``` ` or ` ```lang ` (info string) → code-block language.
/// Anything after the fence is accepted as lang (first token); unknown
/// languages fall back to plain highlighting at render time.
fn parse_fence_lang(text: &str) -> Option<String> {
    let rest = text.trim().strip_prefix("```")?;
    if rest.contains('`') || rest.contains('\n') {
        return None;
    }
    Some(rest.split_whitespace().next().unwrap_or("").to_string())
}

fn node_to_gfm(n: &Node, links: &[String]) -> String {
    match &n.kind {
        NodeKind::Paragraph { inlines } => inlines_to_gfm(inlines, links),
        NodeKind::Heading { level, inlines } => {
            format!(
                "{} {}",
                "#".repeat((*level).clamp(1, 6) as usize),
                inlines_to_gfm(inlines, links)
            )
        }
        NodeKind::Quote { inlines } => wrap_lines("> ", &inlines_to_gfm(inlines, links)),
        NodeKind::Alert { kind, inlines } => {
            let body = inlines_to_gfm(inlines, links);
            if body.is_empty() {
                format!("> [!{}]\n> ", kind.as_str())
            } else {
                format!("> [!{}]\n{}", kind.as_str(), wrap_lines("> ", &body))
            }
        }
        NodeKind::List { ordered, items } => {
            // Relative indent so a whole-list Tab cannot emit 4 leading spaces
            // (CommonMark indented code). Outermost items always start at column 0.
            let base = items.iter().map(|it| it.indent).min().unwrap_or(0);
            items
                .iter()
                .enumerate()
                .map(|(i, it)| {
                    let pad = "  ".repeat(it.indent.saturating_sub(base));
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
                    format!("{pad}{marker}{}", inlines_to_gfm(&it.inlines, links))
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        NodeKind::Code { lang, text, indent, .. } => {
            let pad = " ".repeat((*indent).min(12));
            if pad.is_empty() {
                format!("```{lang}\n{text}\n```")
            } else {
                let body = text.lines().map(|l| if l.trim().is_empty() { l.to_string() } else { format!("{pad}{l}") }).collect::<Vec<_>>().join("\n");
                format!("{pad}```{lang}\n{body}\n{pad}```")
            }
        }
        NodeKind::Table { headers, rows } => {
            let hs: Vec<String> = headers.iter().map(|c| inlines_to_gfm(c, links)).collect();
            let rs: Vec<Vec<String>> = rows
                .iter()
                .map(|r| r.iter().map(|c| inlines_to_gfm(c, links)).collect())
                .collect();
            crate::display::serialize_table(&hs, &rs)
        }
        NodeKind::Rule => "---".into(),
        NodeKind::Image { alt, src } => format!("![{alt}]({src})"),
        NodeKind::Html { raw } => raw.clone(),
        NodeKind::HtmlHeading { level, inlines } => {
            format!("<h{level}>{}</h{level}>", inlines_to_gfm(inlines, links))
        }
        NodeKind::Details { inlines, open, .. } => {
            let tag = if *open { "<details open>" } else { "<details>" };
            format!("{tag}\n<summary>{}</summary>", inlines_to_gfm(inlines, links))
        }
        NodeKind::DetailsClose => "</details>".into(),
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

fn inlines_to_gfm(inlines: &[Inline], links: &[String]) -> String {
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
        if let Some(id) = run.marks.link {
            let url = links.get(id as usize).map(|u| u.as_str()).unwrap_or("");
            s = format!("[{s}]({url})");
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
        let empty_para =
            matches!(n.kind, NodeKind::Paragraph { ref inlines } if inlines.is_empty());
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
    let mut links = doc.links.clone();
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
        NodeKind::HtmlHeading { level, inlines } => {
            emit_inlines(inlines, display, segments, links);
            (BlockExtra::HtmlHeading(*level), BlockKind::Heading(*level))
        }
        NodeKind::Details { inlines, open, .. } => {
            emit_inlines(inlines, display, segments, links);
            let summary = inlines_text(inlines);
            (BlockExtra::Details { summary, open: *open }, BlockKind::Html)
        }
        NodeKind::DetailsClose => {
            let d0 = display.len();
            if display.len() == d0 {
                segments.push(Segment {
                    display: d0..d0,
                    source: d0..d0,
                    marks: Marks::default(),
                });
            }
            (BlockExtra::DetailsClose, BlockKind::Html)
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
                BlockKind::List { ordered: *ordered },
            )
        }
        NodeKind::Code { lang, text, indent, .. } => {
            let d0 = display.len();
            display.push_str(text);
            segments.push(Segment {
                display: d0..display.len(),
                source: d0..display.len(),
                marks: Marks::default(),
            });
            (BlockExtra::Code { lang: lang.clone(), indent: *indent }, BlockKind::Code)
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
                emit_table_inlines(h, display, segments, links);
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
                    emit_table_inlines(cell, display, segments, links);
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
    emit_inlines_inner(inlines, display, segments, links, false);
}

fn emit_table_inlines(
    inlines: &[Inline],
    display: &mut String,
    segments: &mut Vec<Segment>,
    links: &mut Vec<String>,
) {
    emit_inlines_inner(inlines, display, segments, links, true);
}

fn emit_inlines_inner(
    inlines: &[Inline],
    display: &mut String,
    segments: &mut Vec<Segment>,
    links: &mut Vec<String>,
    flatten_newlines: bool,
) {
    for run in inlines {
        let marks = run.marks;
        if let Some(id) = marks.link {
            while links.len() as u32 <= id {
                links.push(String::new());
            }
        }
        let d0 = display.len();
        if flatten_newlines {
            display.push_str(&crate::display::flatten_table_cell_text(&run.text));
        } else {
            display.push_str(&run.text);
        }
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
    fn hash_space_converts() {        let mut d = Doc::empty();
        let mut c = d.insert_text(0, None, "#", Marks::default());
        assert_eq!(d.project().display, "#");
        c = d.insert_text(c, None, " ", Marks::default());
        assert!(matches!(
            d.nodes[0].kind,
            NodeKind::Heading { level: 1, .. }
        ));
        assert_eq!(d.project().display, "");
        let _ = c;
    }

    #[test]
    fn fence_enter_becomes_code_block() {
        for (typed, lang) in [("```", ""), ("```rust", "rust"), ("```rs", "rs"), ("```mermaid", "mermaid"), ("```whatever", "whatever")] {
            let mut d = Doc::empty();
            let mut c = 0;
            for ch in typed.chars() {
                c = d.insert_text(c, None, &ch.to_string(), Marks::default());
            }
            c = d.enter(c, false);
            match &d.nodes[0].kind {
                NodeKind::Code { lang: l, text, .. } => {
                    assert_eq!(l, lang, "{typed:?}");
                    assert_eq!(text, "", "{typed:?}");
                }
                other => panic!("{typed:?} -> {other:?}"),
            }
            assert!(d.to_gfm().starts_with(&format!("```{lang}")), "{}", d.to_gfm());
            let _ = c;
        }
    }

    #[test]
    fn table_cell_newlines_flatten_in_display() {
        let mut d = Doc::from_gfm("| a | b |\n| --- | --- |\n| 1 | 2 |");
        match &mut d.nodes[0].kind {
            NodeKind::Table { rows, .. } => {
                rows[0][0] = vec![Inline {
                    text: "foo\nbar".into(),
                    marks: Marks::default(),
                }];
            }
            other => panic!("{other:?}"),
        }
        let p = d.project();
        let BlockExtra::Table { cells, rows, cols } = &p.blocks[0].extra else {
            panic!("{:?}", p.blocks[0].extra);
        };
        assert_eq!(*rows, 2);
        assert_eq!(*cols, 2);
        assert_eq!(cells.len(), 4);
        let table = &p.display[p.blocks[0].display.clone()];
        assert_eq!(
            table.matches('\n').count(),
            1,
            "row separators stay unique: {table:?}"
        );
        assert!(table.contains("foo\u{001e}bar"), "{table:?}");
        assert!(!table.contains("foo\nbar"), "{table:?}");
        // Tree still stores the original newline.
        match &d.nodes[0].kind {
            NodeKind::Table { rows, .. } => assert_eq!(rows[0][0][0].text, "foo\nbar"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn type_between_code_and_table() {
        let mut d =
            Doc::from_gfm("```\nfn main() {}\n```\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n");
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
        assert_eq!(
            p2.blocks.len(),
            n,
            "{:?}",
            p2.blocks.iter().map(|b| b.kind).collect::<Vec<_>>()
        );
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

/// Splice a new `src` attribute value into an HTML `<video>` tag.
/// Returns None when no `src=` attribute is found.
fn replace_video_src(raw: &str, src: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    let tag = lower.find("<video")?;
    let rest_start = tag;
    let rest_lower = &lower[rest_start..];
    let src_ix = rest_lower.find("src")?;
    let abs_ix = rest_start + src_ix + 3;
    let after = raw[abs_ix..].trim_start();
    let after = after.strip_prefix('=')?.trim_start();
    let quote = after.as_bytes().first()?;
    let base = abs_ix + (raw[abs_ix..].len() - after.len());
    if *quote == b'"' || *quote == b'\'' {
        let q = *quote as char;
        let end_rel = after[1..].find(q)?;
        let start = base + 1;
        let end = base + 1 + end_rel;
        let mut out = raw.to_string();
        out.replace_range(start..end, src);
        Some(out)
    } else {
        let end_rel = after
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(after.len());
        let mut out = raw.to_string();
        out.replace_range(base..base + end_rel, src);
        Some(out)
    }
}

/// Replace the first `src=` (or `href=`) attribute anywhere in `raw` so
/// generic HTML blocks (`<iframe>`, `<embed>`, `<source>`…) can have their
/// URL edited from the handle toolbar.
fn replace_first_attr(raw: &str, names: &[&str], src: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    let mut best_span: Option<(usize, usize, usize)> = None;
    for name in names {
        let mut search = lower.as_str();
        let mut offset = 0usize;
        while let Some(ix) = search.find(*name) {
            let abs = offset + ix;
            let before = raw[..abs].chars().next_back();
            if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                search = &search[ix + name.len()..];
                offset = abs + name.len();
                continue;
            }
            let after = raw[abs + name.len()..].trim_start();
            let Some(eq) = after.strip_prefix('=') else {
                search = &search[ix + name.len()..];
                offset = abs + name.len();
                continue;
            };
            let after = eq.trim_start();
            let base = abs + (raw[abs..].len() - after.len());
            let quote = after.as_bytes().first()?;
            let (start, end) = if *quote == b'"' || *quote == b'\'' {
                let q = *quote as char;
                let end_rel = after[1..].find(q)?;
                (base + 1, base + 1 + end_rel)
            } else {
                let end_rel = after
                    .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
                    .unwrap_or(after.len());
                (base, base + end_rel)
            };
            if best_span.is_none_or(|(pos, _, _)| abs < pos) {
                best_span = Some((abs, start, end));
            }
            break;
        }
    }
    let (_, start, end) = best_span?;
    let mut out = raw.to_string();
    out.replace_range(start..end, src);
    Some(out)
}

#[cfg(test)]
mod feature_tests {
    use super::*;

    #[test]
    fn tab_indents_list_item() {
        let mut d = Doc::from_gfm("- a\n- b");
        let p = d.project();
        // caret on second item
        let caret = p.blocks[0].extra.clone();
        let BlockExtra::List { items, .. } = caret else {
            panic!()
        };
        let c = items[1].display.start;
        let c = d.tab(c, false).expect("indent");
        let NodeKind::List { items, .. } = &d.nodes[0].kind else {
            panic!()
        };
        assert_eq!(items[1].indent, 1);
        let c = d.tab(c, true).expect("outdent");
        let NodeKind::List { items, .. } = &d.nodes[0].kind else {
            panic!()
        };
        assert_eq!(items[1].indent, 0);
        assert!(d.tab(c, false).is_some());

        // caret on first item should also be able to indent
        let p_curr = d.project();
        let BlockExtra::List { items: p_items, .. } = &p_curr.blocks[0].extra else {
            panic!()
        };
        let c0 = p_items[0].display.start;
        let c0 = d.tab(c0, false).expect("indent first item");
        let NodeKind::List { items: items2, .. } = &d.nodes[0].kind else {
            panic!()
        };
        assert_eq!(items2[0].indent, 1);
        let _ = d.tab(c0, true).expect("outdent first item");
        let _ = d.tab(c, true).expect("outdent second item");

        // Multi-block tab selection
        let p3 = d.project();
        let len = p3.display.len();
        d.tab_selection(0..len, false).expect("tab selection");
        let NodeKind::List { items: items3, .. } = &d.nodes[0].kind else {
            panic!()
        };
        assert_eq!(items3[0].indent, 1);
        assert_eq!(items3[1].indent, 1);
    }

    #[test]
    fn tab_whole_list_does_not_emit_indented_code() {
        let mut d = Doc::from_gfm("- bullet\n- two\n- three");
        let len = d.project().display.len();
        for _ in 0..3 {
            assert!(d.tab_selection(0..len, false).is_some());
        }
        let NodeKind::List { items, .. } = &d.nodes[0].kind else {
            panic!("still a list in the tree");
        };
        assert!(items.iter().all(|it| it.indent == 3), "{items:?}");
        let gfm = d.to_gfm();
        assert!(
            !gfm.lines().any(|l| l.starts_with("    ")),
            "4+ leading spaces become indented code: {gfm:?}"
        );
        let d2 = Doc::from_gfm(&gfm);
        assert!(
            matches!(d2.nodes[0].kind, NodeKind::List { .. }),
            "re-parse must stay a list, got {:?} from {gfm:?}",
            d2.nodes[0].kind
        );
    }

    #[test]
    fn nested_list_gfm_roundtrips_indent() {
        let mut d = Doc::from_gfm("- a\n- b\n- c");
        let p = d.project();
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!()
        };
        let b = items[1].display.start;
        let c = items[2].display.start;
        d.tab(b, false).expect("indent b");
        d.tab(c, false).expect("indent c");
        d.tab(c, false).expect("indent c again");
        let gfm = d.to_gfm();
        let d2 = Doc::from_gfm(&gfm);
        assert!(
            matches!(d2.nodes[0].kind, NodeKind::List { .. }),
            "nested items must not become a code block: {gfm:?} {:?}",
            d2.nodes[0].kind
        );
        assert!(gfm.contains("- a"), "{gfm:?}");
        assert!(gfm.contains("- b") && gfm.contains("- c"), "{gfm:?}");
        let NodeKind::List { items, .. } = &d2.nodes[0].kind else {
            panic!("{:?}", d2.nodes[0].kind);
        };
        assert_eq!(items.len(), 3, "{gfm:?}");
        assert_eq!(inlines_text(&items[0].inlines), "a");
        assert_eq!(items[0].indent, 0);
        assert_eq!(items[1].indent, 1);
        assert_eq!(items[2].indent, 2);
    }

    #[test]
    fn nested_list_from_gfm_keeps_parent() {
        let src = "- parent\n  - child\n    - grand\n- sibling";
        let d = Doc::from_gfm(src);
        let NodeKind::List { items, .. } = &d.nodes[0].kind else {
            panic!("{:?}", d.nodes[0].kind);
        };
        assert_eq!(
            items
                .iter()
                .map(|it| (it.indent, inlines_text(&it.inlines)))
                .collect::<Vec<_>>(),
            vec![
                (0, "parent".into()),
                (1, "child".into()),
                (2, "grand".into()),
                (0, "sibling".into()),
            ]
        );
        let gfm = d.to_gfm();
        assert!(gfm.contains("- parent"), "{gfm:?}");
        let d2 = Doc::from_gfm(&gfm);
        let NodeKind::List { items, .. } = &d2.nodes[0].kind else {
            panic!("{} {:?}", gfm, d2.nodes[0].kind);
        };
        assert_eq!(items.len(), 4, "{gfm:?}");
        assert_eq!(inlines_text(&items[0].inlines), "parent");
    }

    #[test]
    fn tab_parent_indents_nested_children() {
        let mut d = Doc::from_gfm("- parent\n  - child\n    - grand\n- sibling");
        let p = d.project();
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!()
        };
        let parent = items[0].display.start;
        d.tab(parent, false).expect("indent parent subtree");
        let NodeKind::List { items, .. } = &d.nodes[0].kind else {
            panic!()
        };
        assert_eq!(
            items.iter().map(|it| it.indent).collect::<Vec<_>>(),
            vec![1, 2, 3, 0],
            "children follow the parent; sibling stays"
        );
        d.tab(parent, true).expect("outdent parent subtree");
        let NodeKind::List { items, .. } = &d.nodes[0].kind else {
            panic!()
        };
        assert_eq!(
            items.iter().map(|it| it.indent).collect::<Vec<_>>(),
            vec![0, 1, 2, 0]
        );
    }

    #[test]
    fn write_nested_tasks_keeps_parents() {
        use crate::display::Affinity;
        let mut d = Doc::empty();
        let mut c = 0;
        for ch in "- [ ] wow2".chars() {
            c = d.insert_text(c, None, &ch.to_string(), Marks::default());
        }
        c = d.enter(c, false);
        c = d.tab(c, false).expect("tab");
        for ch in "wow3".chars() {
            c = d.insert_text(c, None, &ch.to_string(), Marks::default());
        }
        c = d.enter(c, false);
        c = d.tab(c, false).expect("tab2");
        for ch in "wow4".chars() {
            c = d.insert_text(c, None, &ch.to_string(), Marks::default());
        }
        let gfm = d.to_gfm();
        assert!(
            gfm.contains("wow2") && gfm.contains("wow3") && gfm.contains("wow4"),
            "parents lost while writing: {gfm:?}"
        );
        let d2 = Doc::from_gfm(&gfm);
        let texts: Vec<String> = d2
            .nodes
            .iter()
            .flat_map(|n| match &n.kind {
                NodeKind::List { items, .. } => items
                    .iter()
                    .map(|it| inlines_text(&it.inlines))
                    .collect::<Vec<_>>(),
                _ => vec![],
            })
            .collect();
        assert_eq!(texts, vec!["wow2", "wow3", "wow4"], "gfm was {gfm:?}");
        let _ = Affinity::Inside;
        let _ = c;
    }

    #[test]
    fn nested_tasks_roundtrip() {
        let src = "- [ ] wow2\n  - [ ] wow3\n    - [ ] wow4";
        let d = Doc::from_gfm(src);
        let NodeKind::List { items, .. } = &d.nodes[0].kind else {
            panic!("not a list: {:?}", d.nodes[0].kind);
        };
        assert_eq!(items.len(), 3, "{items:?}");
        let gfm = d.to_gfm();
        let d2 = Doc::from_gfm(&gfm);
        let NodeKind::List { items, .. } = &d2.nodes[0].kind else {
            panic!("reparse lost list: {gfm:?} -> {:?}", d2.nodes[0].kind);
        };
        assert_eq!(
            items
                .iter()
                .map(|it| inlines_text(&it.inlines))
                .collect::<Vec<_>>(),
            vec!["wow2".to_string(), "wow3".to_string(), "wow4".to_string()],
            "gfm was {gfm:?}"
        );
    }

    #[test]
    fn toggle_task_flips_checked() {
        let mut d = Doc::from_gfm("- [ ] todo");
        d.toggle_task(0, 0);
        let NodeKind::List { items, .. } = &d.nodes[0].kind else {
            panic!()
        };
        assert_eq!(items[0].checked, Some(true));
        assert!(d.to_gfm().contains("- [x]"));
    }

    #[test]
    fn toggle_mark_multiline() {
        let mut d = Doc::from_gfm("Line one\n\nLine two");
        let p = d.project();
        let len = p.display.len();
        let res = d.toggle_mark(0..len, Mark::Bold);
        assert!(res.is_some());
        let p2 = d.project();
        assert!(p2.marks_at(0, Affinity::Inside).bold);
        let second_line_start = p2.display.find("Line two").unwrap();
        assert!(p2.marks_at(second_line_start, Affinity::Inside).bold);

        // Test unbolding multi-line
        let res2 = d.toggle_mark(0..len, Mark::Bold);
        assert!(res2.is_some());
        let p3 = d.project();
        assert!(!p3.marks_at(0, Affinity::Inside).bold);
        assert!(!p3.marks_at(second_line_start, Affinity::Inside).bold);
    }

    #[test]
    fn delete_all_does_not_panic() {
        let mut d = Doc::from_gfm("# Hello\n\nWorld line 2\n\n- item 1\n- item 2");
        let p = d.project();
        let len = p.display.len();
        let c = d.delete_display(0..len);
        assert_eq!(c, 0);
        assert!(!d.nodes.is_empty());
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

    #[test]
    fn link_url_roundtrips() {
        let d = Doc::from_gfm("[hello](https://example.com)");
        let p = d.project();
        let (_, url) = p.link_at(0).expect("link");
        assert_eq!(url, "https://example.com");
        let gfm = d.to_gfm();
        assert!(gfm.contains("https://example.com"), "{gfm}");
        let p2 = Doc::from_gfm(&gfm).project();
        assert_eq!(p2.link_at(0).map(|(_, u)| u), Some("https://example.com"));
    }

    #[test]
    fn apply_link_stores_url() {
        let mut d = Doc::from_gfm("hello");
        d.apply_link(0..5, "https://zed.dev");
        let p = d.project();
        assert_eq!(p.link_at(1).map(|(_, u)| u), Some("https://zed.dev"));
        assert!(d.to_gfm().contains("https://zed.dev"));
    }

    #[test]
    fn copy_heading_pastes_as_heading() {
        let src = Doc::from_gfm("# Hello\n\npara");
        let p = src.project();
        let gfm = src.gfm_range(p.blocks[0].display.clone());
        assert!(gfm.starts_with("# "), "{gfm}");
        let mut d = Doc::empty();
        d.paste_gfm(0, &gfm);
        assert!(matches!(
            d.nodes[0].kind,
            NodeKind::Heading { level: 1, .. }
        ));
        assert_eq!(d.project().display, "Hello");
    }

    #[test]
    fn copy_bold_pastes_marks() {
        let src = Doc::from_gfm("**hi**");
        let gfm = src.gfm_range(0..src.project().display.len());
        let mut d = Doc::from_gfm("x");
        d.paste_gfm(1, &gfm);
        let p = d.project();
        assert_eq!(p.display, "xhi");
        assert!(p.marks_at(1, Affinity::Inside).bold);
    }

    #[test]
    fn copy_list_item_stays_list() {
        let src = Doc::from_gfm("- a\n- b");
        let p = src.project();
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!();
        };
        let gfm = src.gfm_range(items[0].display.clone());
        assert!(gfm.starts_with("- "), "{gfm}");
        assert!(!gfm.contains("- b"), "{gfm}");
        let mut d = Doc::empty();
        d.paste_gfm(0, &gfm);
        assert!(matches!(d.nodes[0].kind, NodeKind::List { .. }));
    }

    #[test]
    fn paste_list_into_list_appends_items() {
        let mut d = Doc::from_gfm("- a\n- b");
        let p = d.project();
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!();
        };
        let caret = items[0].display.end;
        d.paste_gfm(caret, "- c");
        let NodeKind::List { items, .. } = &d.nodes[0].kind else {
            panic!();
        };
        assert_eq!(items.len(), 3);
        assert_eq!(inlines_text(&items[1].inlines), "c");
    }

    #[test]
    fn paste_heading_after_paragraph() {
        let mut d = Doc::from_gfm("Hello");
        d.paste_gfm(5, "# Title");
        assert!(matches!(d.nodes[0].kind, NodeKind::Paragraph { .. }));
        assert!(matches!(
            d.nodes[1].kind,
            NodeKind::Heading { level: 1, .. }
        ));
    }

    #[test]
    fn bracket_space_makes_tasklist() {
        let mut d = Doc::empty();
        let mut c = d.insert_text(0, None, "[", Marks::default());
        c = d.insert_text(c, None, " ", Marks::default());
        c = d.insert_text(c, None, "]", Marks::default());
        c = d.insert_text(c, None, " ", Marks::default());
        assert!(
            matches!(
                &d.nodes[0].kind,
                NodeKind::List {
                    items,
                    ..
                } if items[0].checked == Some(false)
            ),
            "{:?}",
            d.nodes[0].kind
        );
        assert_eq!(d.to_gfm().trim(), "- [ ]");
        let _ = c;
    }

    #[test]
    fn empty_brackets_space_makes_tasklist() {
        let mut d = Doc::empty();
        let mut c = d.insert_text(0, None, "[", Marks::default());
        c = d.insert_text(c, None, "]", Marks::default());
        c = d.insert_text(c, None, " ", Marks::default());
        assert!(
            matches!(
                &d.nodes[0].kind,
                NodeKind::List {
                    items,
                    ..
                } if items[0].checked == Some(false)
            ),
            "{:?}",
            d.nodes[0].kind
        );
        assert_eq!(d.to_gfm().trim(), "- [ ]");
        let _ = c;
    }

    #[test]
    fn closing_backtick_makes_inline_code() {
        let mut d = Doc::empty();
        let mut c = d.insert_text(0, None, "`", Marks::default());
        c = d.insert_text(c, None, "code", Marks::default());
        c = d.insert_text(c, None, "`", Marks::default());
        let p = d.project();
        assert_eq!(p.display, "code");
        assert!(p.marks_at(0, crate::display::Affinity::Inside).code);
        assert!(d.to_gfm().contains("`code`"), "{}", d.to_gfm());
        let _ = c;
    }

    #[test]
    fn table_typing_stays_in_cell() {
        let mut d = Doc::from_gfm("| a | b |\n| --- | --- |\n| 1 | 2 |\n");
        let p = d.project();
        let BlockExtra::Table { cells, .. } = &p.blocks[0].extra else {
            panic!("{:?}", p.blocks[0].extra);
        };
        let cell = cells
            .iter()
            .find(|c| !c.header && c.row == 1 && c.col == 1)
            .unwrap();
        let caret = cell.display.end;
        let next = d.insert_text(caret, None, "x", Marks::default());
        let p2 = d.project();
        let BlockExtra::Table { cells, .. } = &p2.blocks[0].extra else {
            panic!();
        };
        let cell2 = cells
            .iter()
            .find(|c| !c.header && c.row == 1 && c.col == 1)
            .unwrap();
        assert!(
            p2.display[cell2.display.clone()].contains('x'),
            "typed into {:?}",
            &p2.display[cell2.display.clone()]
        );
        assert!(
            next >= cell2.display.start && next <= cell2.display.end,
            "caret {next} not in {}..{}",
            cell2.display.start,
            cell2.display.end
        );
    }

    #[test]
    fn table_insert_col_mutates_tree() {
        let mut d = Doc::from_gfm("| a | b |\n| --- | --- |\n| 1 | 2 |\n");
        let p = d.project();
        let BlockExtra::Table { cells, .. } = &p.blocks[0].extra else {
            panic!();
        };
        let cell = cells.iter().find(|c| c.row == 0 && c.col == 0).unwrap();
        d.insert_table_col(cell.display.start, false).unwrap();
        let NodeKind::Table { headers, rows } = &d.nodes[0].kind else {
            panic!();
        };
        assert_eq!(headers.len(), 3);
        assert_eq!(rows[0].len(), 3);
        assert_eq!(inlines_text(&headers[0]), "a");
        assert_eq!(inlines_text(&headers[2]), "b");
    }

    #[test]
    fn table_toggle_mark_stays_in_cell() {
        let mut d = Doc::from_gfm("| a | b |\n| --- | --- |\n| hello | 2 |\n");
        let p = d.project();
        let BlockExtra::Table { cells, .. } = &p.blocks[0].extra else {
            panic!();
        };
        let cell = cells.iter().find(|c| c.row == 1 && c.col == 0).unwrap();
        let sel = cell.display.clone();
        d.toggle_mark(sel.clone(), Mark::Bold).unwrap();
        let p2 = d.project();
        assert!(p2.marks_at(sel.start, Affinity::Inside).bold);
        let BlockExtra::Table { cells, .. } = &p2.blocks[0].extra else {
            panic!();
        };
        let other = cells.iter().find(|c| c.row == 1 && c.col == 1).unwrap();
        assert!(!p2.marks_at(other.display.start, Affinity::Inside).bold);
    }

    #[test]
    fn table_shift_enter_stays_in_cell() {
        let mut d = Doc::from_gfm("| a | b |\n| --- | --- |\n| 1 | 2 |\n");
        let p = d.project();
        let BlockExtra::Table { cells, .. } = &p.blocks[0].extra else {
            panic!();
        };
        let cell = cells.iter().find(|c| c.row == 1 && c.col == 0).unwrap();
        let caret = cell.display.end;
        let next = d.enter(caret, true);
        match &d.nodes[0].kind {
            NodeKind::Table { rows, .. } => {
                assert!(inlines_text(&rows[0][0]).contains('\n'), "{rows:?}");
            }
            other => panic!("{other:?}"),
        }
        let p2 = d.project();
        let BlockExtra::Table { cells, rows, .. } = &p2.blocks[0].extra else {
            panic!();
        };
        assert_eq!(*rows, 2);
        let cell2 = cells.iter().find(|c| c.row == 1 && c.col == 0).unwrap();
        assert!(
            next >= cell2.display.start && next <= cell2.display.end,
            "caret {next} not in {}..{}",
            cell2.display.start,
            cell2.display.end
        );
        let display = &p2.display;
        assert!(
            display.is_char_boundary(next),
            "caret {next} not a char boundary in {display:?}"
        );
        assert!(
            !display[cell2.display.clone()].contains('\n'),
            "cell display must not use raw newlines: {:?}",
            &display[cell2.display.clone()]
        );
    }

    #[test]
    fn table_delete_row_range() {
        let mut d = Doc::from_gfm("| a | b |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |\n");
        let p = d.project();
        let BlockExtra::Table { cells, .. } = &p.blocks[0].extra else {
            panic!();
        };
        let cell = cells.iter().find(|c| c.row == 1 && c.col == 0).unwrap();
        d.delete_table_rows(cell.display.start, 1, 2).unwrap();
        let NodeKind::Table { rows, .. } = &d.nodes[0].kind else {
            panic!();
        };
        assert!(rows.is_empty(), "{rows:?}");
    }

    #[test]
    fn table_toggle_mark_rect_scopes_to_rect() {
        let mut d = Doc::from_gfm("| a | b |\n| --- | --- |\n| 1 | 2 |\n");
        assert!(d.toggle_mark_table(0, 0, 1, 1, 1, Mark::Bold));
        let p = d.project();
        let BlockExtra::Table { cells, .. } = &p.blocks[0].extra else {
            panic!();
        };
        for c in cells {
            let bold = p.marks_at(c.display.start, Affinity::Inside).bold;
            if c.col == 1 {
                assert!(bold, "right column cell must be bold: {c:?}");
            } else {
                assert!(!bold, "left column cell must stay plain: {c:?}");
            }
        }
    }

    #[test]
    fn replace_chars_stays_put() {
        let mut d = Doc::from_gfm("Tablek");
        let p = d.project();
        let at = p.display.len() - 1;
        let c = d.replace_chars(at, 1, 'X');
        assert_eq!(d.project().display, "TableX");
        assert_eq!(c, at);
    }

    #[test]
    fn join_lines_merges_list_items() {
        let mut d = Doc::from_gfm("- a\n- b\n- c");
        let p = d.project();
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!();
        };
        let c = d.join_lines(items[0].display.start, 1);
        let NodeKind::List { items, .. } = &d.nodes[0].kind else {
            panic!();
        };
        assert_eq!(items.len(), 2);
        assert_eq!(inlines_text(&items[0].inlines), "a b");
        let _ = c;
    }

    #[test]
    fn enter_closed_details_lands_below_close() {
        let mut d = Doc::from_gfm("<details>\n<summary>Hi</summary>\n\nbody\n\n</details>\n");
        let p = d.project();
        assert!(matches!(p.blocks[0].extra, BlockExtra::Details { .. }));
        let caret = p.blocks[0].display.end;
        let c = d.enter_closed_details(caret).expect("on details");
        let gfm = d.to_gfm();
        // New paragraph after </details>, summary intact.
        assert!(gfm.contains("</details>"), "{gfm:?}");
        let close = gfm.find("</details>").unwrap();
        let after = gfm[close..].to_string();
        assert!(after.contains("\n\n"), "{gfm:?}");
        let p2 = d.project();
        // Caret sits after the close (visible block below).
        assert!(c > p2.blocks[0].display.end, "c={c} {:?}", p2.display);
    }

    #[test]
    fn enter_closed_details_splits_summary_below() {
        let mut d = Doc::from_gfm("<details>\n<summary>Hello</summary>\n\nbody\n\n</details>\n");
        let p = d.project();
        let caret = p.blocks[0].display.start + 2; // mid-summary "He|llo"
        let _ = d.enter_closed_details(caret).expect("on details");
        let NodeKind::Details { inlines, .. } = &d.nodes[0].kind else {
            panic!();
        };
        assert_eq!(inlines_text(inlines), "He");
        // Right half goes to the paragraph after </details>.
        let close = d.nodes.iter().position(|n| matches!(n.kind, NodeKind::DetailsClose)).unwrap();
        let NodeKind::Paragraph { inlines } = &d.nodes[close + 1].kind else {
            panic!("{:?}", d.nodes[close + 1].kind);
        };
        assert_eq!(inlines_text(inlines), "llo");
    }

    #[test]
    fn backspace_details_is_two_step() {
        let mut d = Doc::from_gfm("head\n\n<details>\n<summary>Hi</summary>\n\nbody\n\n</details>\n");
        let p = d.project();
        let di = p.blocks.iter().position(|b| matches!(b.extra, BlockExtra::Details { .. })).unwrap();
        // First backspace strips the wrapper, keeps the summary text.
        let c = d.backspace(p.blocks[di].display.start).expect("convert");
        let NodeKind::Paragraph { inlines } = &d.nodes[di].kind else {
            panic!("{:?}", d.nodes[di].kind);
        };
        assert_eq!(inlines_text(inlines), "Hi");
        assert!(d.to_gfm().contains("body"), "{:?}", d.to_gfm());
        // Second backspace merges into the previous block.
        let c2 = d.backspace(c).expect("merge");
        let gfm = d.to_gfm();
        assert!(gfm.contains("headHi"), "{gfm:?}");
        assert!(!gfm.contains("<summary>"), "{gfm:?}");
        assert_eq!(c2, "head".len());
    }

    #[test]
    fn backspace_quote_is_two_step() {
        let mut d = Doc::from_gfm("head\n\n> hi\n");
        let p = d.project();
        let qi = p.blocks.iter().position(|b| matches!(b.extra, BlockExtra::Quote)).unwrap();
        let c = d.backspace(p.blocks[qi].display.start).expect("convert");
        let NodeKind::Paragraph { inlines } = &d.nodes[qi].kind else {
            panic!("{:?}", d.nodes[qi].kind);
        };
        assert_eq!(inlines_text(inlines), "hi");
        let c2 = d.backspace(c).expect("merge");
        assert!(d.to_gfm().contains("headhi"), "{:?}", d.to_gfm());
        assert_eq!(c2, "head".len());
    }

    #[test]
    fn backspace_empty_after_details_lands_last_inside() {
        let mut d = Doc::from_gfm("<details>\n<summary>Hi</summary>\n\nbody\n\n</details>\n\ntail\n");
        let p = d.project();
        // Empty paragraph after the close (like `o` on a closed disclosure).
        let ins = d.open_after_details(p.blocks[0].display.start).expect("insert");
        // Backspace erases it and lands on the last block inside, text kept.
        let c = d.backspace(ins).expect("remove");
        let p2 = d.project();
        let ci = p2.blocks.iter().position(|b| matches!(b.extra, BlockExtra::DetailsClose)).unwrap();
        assert_eq!(c, p2.blocks[ci - 1].display.end, "c={c} {:?}", p2.display);
        assert!(d.to_gfm().contains("body"), "{:?}", d.to_gfm());
    }

    #[test]
    fn backspace_text_after_details_merges_last_inside() {
        let mut d = Doc::from_gfm("<details>\n<summary>Hi</summary>\n\nbody\n\n</details>\n\ntail\n");
        let p = d.project();
        let ti = p.blocks.iter().position(|b| p.display.get(b.display.clone()) == Some("tail")).unwrap();
        let ci = p.blocks.iter().position(|b| matches!(b.extra, BlockExtra::DetailsClose)).unwrap();
        let body_end = p.blocks[ci - 1].display.end;
        // Text merges into the last body node (never into the chrome, which
        // would silently drop it) and the caret lands at the old body end.
        let c = d.backspace(p.blocks[ti].display.start).expect("merge");
        assert_eq!(c, body_end, "c={c} {:?}", d.project().display);
        assert!(d.to_gfm().contains("bodytail"), "{:?}", d.to_gfm());
    }

    #[test]
    fn tab_outdent_ejects_body_block_after_close() {
        let mut d = Doc::from_gfm("<details>\n<summary>Hi</summary>\n\nbody\n\nmore\n\n</details>\n");
        let p = d.project();
        let bi = p.blocks.iter().position(|b| p.display.get(b.display.clone()) == Some("body")).unwrap();
        let c = d.tab(p.blocks[bi].display.start, true).expect("eject");
        let gfm = d.to_gfm();
        // Body block moved after </details>, summary + sibling kept inside.
        let close = gfm.find("</details>").unwrap();
        assert!(gfm[close..].contains("body"), "{gfm:?}");
        assert!(gfm[..close].contains("more"), "{gfm:?}");
        let p2 = d.project();
        assert!(c <= p2.display.len());
    }

    #[test]
    fn tab_indent_adopts_trailing_block_into_details() {
        let mut d = Doc::from_gfm("<details>\n<summary>Hi</summary>\n\nbody\n\n</details>\n\ntail\n");
        let p = d.project();
        let ti = p.blocks.iter().position(|b| p.display.get(b.display.clone()) == Some("tail")).unwrap();
        let c = d.tab(p.blocks[ti].display.start, false).expect("adopt");
        let gfm = d.to_gfm();
        let close = gfm.find("</details>").unwrap();
        assert!(gfm[..close].contains("tail"), "{gfm:?}");
        let p2 = d.project();
        assert!(c <= p2.display.len());
    }

    #[test]
    fn tab_indent_nonadjacent_block_stays_outside() {
        let mut d = Doc::from_gfm("<details>\n<summary>Hi</summary>\n\nbody\n\n</details>\n\nmid\n\ntail\n");
        let p = d.project();
        let ti = p.blocks.iter().position(|b| p.display.get(b.display.clone()) == Some("tail")).unwrap();
        // Not directly after the close → no adopt (plain tab has no non-list target).
        assert!(d.tab(p.blocks[ti].display.start, false).is_none());
    }

    #[test]
    fn backspace_body_start_ejects_after_close() {
        let mut d = Doc::from_gfm("<details>\n<summary>Hi</summary>\n\nbody\n\n</details>\n");
        let p = d.project();
        let bi = p.blocks.iter().position(|b| p.display.get(b.display.clone()) == Some("body")).unwrap();
        let c = d.backspace(p.blocks[bi].display.start).expect("eject");
        let gfm = d.to_gfm();
        let close = gfm.find("</details>").unwrap();
        assert!(gfm[close..].contains("body"), "{gfm:?}");
        let p2 = d.project();
        assert_eq!(c, p2.blocks.iter().find(|b| p2.display.get(b.display.clone()) == Some("body")).unwrap().display.start, "c={c} {:?}", p2.display);
    }
}
