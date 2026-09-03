use serde::{Deserialize, Serialize};

use super::ActionItem;

/// Wrapper that marks clipboard text as a list of our actions.
#[derive(Serialize, Deserialize)]
struct Clip {
    parrot_actions: Vec<ActionItem>,
}

/// Encodes actions as clipboard text so they survive a trip through the system clipboard.
pub fn encode(items: &[ActionItem]) -> String {
    let clip = Clip { parrot_actions: items.to_vec() };
    serde_json::to_string(&clip).unwrap_or_default()
}

/// Reads back actions from clipboard text, or `None` if the text is not ours.
pub fn decode(text: &str) -> Option<Vec<ActionItem>> {
    let clip: Clip = serde_json::from_str(text.trim()).ok()?;
    (!clip.parrot_actions.is_empty()).then_some(clip.parrot_actions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Action, TimeUnit};

    fn item(id: u64, name: &str) -> ActionItem {
        let mut item = ActionItem::new(id, Action::Label { name: name.into() });
        item.comment = "note".into();
        item
    }

    #[test]
    fn round_trips_actions_with_their_comments() {
        let items = vec![item(1, "a"), ActionItem::new(2, Action::Wait { duration: 5.0, unit: TimeUnit::S })];
        let back = decode(&encode(&items)).unwrap();
        assert_eq!(back, items);
    }

    #[test]
    fn foreign_text_is_rejected() {
        assert!(decode("hello").is_none());
        assert!(decode("{\"items\":[]}").is_none());
        assert!(decode(&encode(&[])).is_none());
    }
}
