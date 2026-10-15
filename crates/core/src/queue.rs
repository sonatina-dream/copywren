//! Paste Queue: collect several copies, then paste them one by one (SPEC §6).

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::model::{ClipContents, ItemKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueDirection {
    /// Paste in the order things were copied.
    #[default]
    Fifo,
    /// Paste the most recent copy first.
    Lifo,
}

impl QueueDirection {
    pub fn toggled(self) -> Self {
        match self {
            QueueDirection::Fifo => QueueDirection::Lifo,
            QueueDirection::Lifo => QueueDirection::Fifo,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            QueueDirection::Fifo => "First copied, first pasted",
            QueueDirection::Lifo => "Last copied, first pasted",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snippet {
    pub kind: ItemKind,
    pub preview_text: String,
    pub contents: ClipContents,
}

/// Snippets are kept in copy order; `direction` decides which end pastes next.
#[derive(Debug, Clone, Default)]
pub struct PasteQueue {
    snippets: VecDeque<Snippet>,
    pub direction: QueueDirection,
}

impl PasteQueue {
    pub fn push(&mut self, snippet: Snippet) {
        self.snippets.push_back(snippet);
    }

    /// Removes and returns the snippet that should be pasted now.
    pub fn take_next(&mut self) -> Option<Snippet> {
        match self.direction {
            QueueDirection::Fifo => self.snippets.pop_front(),
            QueueDirection::Lifo => self.snippets.pop_back(),
        }
    }

    /// Snippets in the order they will be pasted.
    pub fn in_paste_order(&self) -> Vec<&Snippet> {
        match self.direction {
            QueueDirection::Fifo => self.snippets.iter().collect(),
            QueueDirection::Lifo => self.snippets.iter().rev().collect(),
        }
    }

    /// Removes the snippet at `index` in paste order.
    pub fn remove_in_paste_order(&mut self, index: usize) -> Option<Snippet> {
        let len = self.snippets.len();
        if index >= len {
            return None;
        }
        let storage_index = match self.direction {
            QueueDirection::Fifo => index,
            QueueDirection::Lifo => len - 1 - index,
        };
        self.snippets.remove(storage_index)
    }

    pub fn toggle_direction(&mut self) {
        self.direction = self.direction.toggled();
    }

    pub fn clear(&mut self) {
        self.snippets.clear();
    }

    pub fn len(&self) -> usize {
        self.snippets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.snippets.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snip(text: &str) -> Snippet {
        Snippet {
            kind: ItemKind::Text,
            preview_text: text.into(),
            contents: ClipContents::plain_text(text),
        }
    }

    fn texts(queue: &PasteQueue) -> Vec<String> {
        queue
            .in_paste_order()
            .iter()
            .map(|s| s.preview_text.clone())
            .collect()
    }

    #[test]
    fn fifo_and_lifo() {
        let mut queue = PasteQueue::default();
        for t in ["a", "b", "c"] {
            queue.push(snip(t));
        }
        assert_eq!(texts(&queue), ["a", "b", "c"]);
        assert_eq!(queue.take_next().unwrap().preview_text, "a");
        queue.toggle_direction();
        assert_eq!(texts(&queue), ["c", "b"]);
        assert_eq!(queue.take_next().unwrap().preview_text, "c");
        assert_eq!(queue.take_next().unwrap().preview_text, "b");
        assert!(queue.take_next().is_none());
        assert!(queue.is_empty());
    }

    #[test]
    fn remove_respects_display_order() {
        let mut queue = PasteQueue::default();
        for t in ["a", "b", "c"] {
            queue.push(snip(t));
        }
        queue.direction = QueueDirection::Lifo;
        assert_eq!(queue.remove_in_paste_order(0).unwrap().preview_text, "c");
        assert_eq!(texts(&queue), ["b", "a"]);
        assert!(queue.remove_in_paste_order(5).is_none());
        queue.direction = QueueDirection::Fifo;
        assert_eq!(queue.remove_in_paste_order(1).unwrap().preview_text, "b");
        assert_eq!(queue.len(), 1);
        queue.clear();
        assert!(queue.is_empty());
    }
}
