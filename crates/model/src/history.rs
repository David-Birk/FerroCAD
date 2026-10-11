//! Undo/redo by whole-document snapshots.
//!
//! Documents are small (sketch parameters and feature descriptions, no
//! B-rep), so snapshotting is simpler and more robust than command
//! inversion. Solids are rebuilt from the restored document.

use crate::document::Document;

#[derive(Clone, Debug)]
pub struct History {
    undo: Vec<(String, Document)>,
    redo: Vec<(String, Document)>,
    limit: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::new(200)
    }
}

impl History {
    pub fn new(limit: usize) -> Self {
        Self { undo: Vec::new(), redo: Vec::new(), limit: limit.max(1) }
    }

    /// Record the state *before* a change. Any redo branch is discarded.
    pub fn record(&mut self, label: impl Into<String>, before: Document) {
        self.undo.push((label.into(), before));
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Convenience: snapshot `doc` as it is now, before mutating it.
    pub fn checkpoint(&mut self, label: impl Into<String>, doc: &Document) {
        self.record(label, doc.clone());
    }

    /// Restore the previous state. Returns the label of the undone change.
    pub fn undo(&mut self, doc: &mut Document) -> Option<String> {
        let (label, before) = self.undo.pop()?;
        let current = std::mem::replace(doc, before);
        self.redo.push((label.clone(), current));
        Some(label)
    }

    pub fn redo(&mut self, doc: &mut Document) -> Option<String> {
        let (label, after) = self.redo.pop()?;
        let current = std::mem::replace(doc, after);
        self.undo.push((label.clone(), current));
        Some(label)
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|(l, _)| l.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|(l, _)| l.as_str())
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_roundtrip() {
        let mut doc = Document::new();
        let mut h = History::default();

        h.checkpoint("set a", &doc);
        doc.params.set("a", "1").unwrap();
        h.checkpoint("set b", &doc);
        doc.params.set("b", "2").unwrap();

        assert_eq!(h.undo(&mut doc).as_deref(), Some("set b"));
        assert!(doc.params.get("b").is_none());
        assert_eq!(h.undo(&mut doc).as_deref(), Some("set a"));
        assert!(doc.params.get("a").is_none());
        assert!(h.undo(&mut doc).is_none());

        assert_eq!(h.redo(&mut doc).as_deref(), Some("set a"));
        assert_eq!(h.redo(&mut doc).as_deref(), Some("set b"));
        assert_eq!(doc.params.value("b").unwrap(), 2.0);
        assert!(h.redo(&mut doc).is_none());
    }

    #[test]
    fn new_change_drops_redo_and_limit_applies() {
        let mut doc = Document::new();
        let mut h = History::new(2);
        for i in 0..3 {
            h.checkpoint(format!("step {i}"), &doc);
            doc.params.set(&format!("p{i}"), "1").unwrap();
        }
        h.undo(&mut doc);
        h.checkpoint("other", &doc);
        assert!(h.redo_label().is_none());
        // only two undo steps are kept
        assert!(h.undo(&mut doc).is_some());
        assert!(h.undo(&mut doc).is_some());
        assert!(h.undo(&mut doc).is_none());
    }
}
