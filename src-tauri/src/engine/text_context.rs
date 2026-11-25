use crate::model::{now, Message};
use std::collections::BTreeMap;

const SOURCE_CHARS: usize = 24_000;
const DRAFT_CHARS: usize = 12_000;

// Only confirmed messages are persisted. These bounded source buffers track
// whether the effective input changed, independently of ASR sentence boundaries.
#[derive(Default)]
pub(super) struct TextContext {
    confirmed: BTreeMap<String, String>,
    drafts: BTreeMap<String, String>,
    handled: BTreeMap<String, String>,
}

fn tail(text: &str, limit: usize) -> String {
    text.chars()
        .rev()
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn normalized(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn suffix_after<'a>(draft: &'a str, confirmed: &str) -> Option<&'a str> {
    let mut chars = draft.char_indices().filter(|(_, c)| !c.is_whitespace());
    let mut end = 0;
    for expected in confirmed.chars() {
        let (at, actual) = chars.next()?;
        if actual != expected {
            return None;
        }
        end = at + actual.len_utf8();
    }
    Some(&draft[end..])
}

impl TextContext {
    pub fn partial(&mut self, source: &str, text: &str) {
        // ASR sends empty interim packets as keepalives and after finals. They
        // do not retract previously heard words; a final or new draft replaces them.
        if !text.trim().is_empty() {
            self.drafts
                .insert(source.into(), tail(text.trim(), SOURCE_CHARS));
        }
    }

    pub fn confirm(&mut self, source: &str, text: &str) {
        let text = normalized(text);
        let confirmed = self.confirmed.entry(source.into()).or_default();
        confirmed.push_str(&text);
        *confirmed = tail(confirmed, SOURCE_CHARS);
        if let Some(draft) = self.drafts.remove(source) {
            // One interim packet can cover several sentences. Keep its suffix
            // while the first sentence is being confirmed, until ASR updates it.
            if let Some(rest) = suffix_after(&draft, &text) {
                self.partial(source, rest);
            }
        }
    }

    fn snapshot(&self) -> BTreeMap<String, String> {
        let mut snapshot = self.confirmed.clone();
        for (source, draft) in &self.drafts {
            let text = snapshot.entry(source.clone()).or_default();
            text.push_str(&normalized(draft));
            *text = tail(text, SOURCE_CHARS);
        }
        snapshot.retain(|_, text| !text.is_empty());
        snapshot
    }

    pub fn changed(&self) -> bool {
        self.snapshot() != self.handled
    }

    pub fn mark_handled(&mut self) {
        self.handled = self.snapshot();
    }

    pub fn append_drafts(&self, messages: &mut Vec<Message>, session_id: &str) {
        // Reserve half of the provider's 24k character budget for history, and
        // give every active source a share so one long draft cannot hide the rest.
        let per_source = DRAFT_CHARS / self.drafts.len().max(1);
        for (source, text) in &self.drafts {
            messages.push(Message {
                id: String::new(),
                session_id: session_id.into(),
                role: "user".into(),
                source: format!("{source}（临时识别，尚未确认，可能修正）"),
                text: tail(text, per_source),
                created_at: now(),
                interrupted: false,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmation_keeps_the_same_effective_input_and_unconfirmed_suffix() {
        let mut context = TextContext::default();
        context.partial("mic", "第一句 第二句");
        assert!(context.changed());
        context.mark_handled();
        context.confirm("mic", "第 一句");
        assert!(!context.changed());
        assert_eq!(context.drafts["mic"], "第二句");
        context.partial("mic", "第二句");
        context.confirm("mic", "第二句");
        assert!(!context.changed());
        assert!(context.drafts.is_empty());
        context.partial("mic", "第三句");
        assert!(context.changed());
    }

    #[test]
    fn empty_packets_and_spacing_do_not_retrigger_but_corrections_do() {
        let mut context = TextContext::default();
        context.partial("mic", "");
        assert!(!context.changed());
        context.partial("mic", "明天 去上海");
        context.mark_handled();
        context.partial("mic", "明天去上海");
        context.partial("mic", "");
        assert!(!context.changed());
        context.partial("mic", "明天去北京");
        assert!(context.changed());
        context.mark_handled();
        context.confirm("mic", "明天去北京");
        assert!(!context.changed());
    }

    #[test]
    fn sources_remain_separate_and_drafts_share_a_bounded_unicode_budget() {
        let mut context = TextContext::default();
        context.partial("mic", &"麦".repeat(SOURCE_CHARS + 20));
        context.partial("system", &"声".repeat(SOURCE_CHARS + 20));
        let mut messages = Vec::new();
        context.append_drafts(&mut messages, "session");
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages
                .iter()
                .map(|m| m.text.chars().count())
                .sum::<usize>(),
            DRAFT_CHARS
        );
        assert!(messages.iter().all(|m| m.source.contains("尚未确认")));
        context.mark_handled();
        context.confirm("mic", &"麦".repeat(SOURCE_CHARS + 20));
        assert!(!context.changed());
        context.partial("system", "电脑音频的修正");
        assert!(context.changed());
        assert!(context
            .snapshot()
            .values()
            .all(|s| s.chars().count() <= SOURCE_CHARS));
    }
}
