use std::{
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use gpui::{Context, Task};
use orbitshell_ssh::remote_editor::MAX_BYTES;

use super::EditorInput;
use crate::{
    editor_format::{self, FormatMode},
    editor_search::SearchQuery,
    editor_syntax::{self, Language},
};

#[derive(Clone, Copy)]
pub(crate) enum SearchAction {
    Next,
    Previous,
    SelectAll,
    Replace,
    ReplaceAll,
}

pub(super) struct EditorTask {
    pub(super) cancelled: Arc<AtomicBool>,
    pub(super) _task: Task<()>,
}

impl Drop for EditorTask {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl EditorInput {
    pub(crate) fn language(&self) -> Language {
        self.language
    }

    pub(crate) fn cursor_position(&self) -> (usize, usize) {
        let line = self.line_of(self.caret);
        (line + 1, self.column_of(line, self.caret) + 1)
    }

    pub(crate) fn format_notice(&self) -> Option<(&str, bool)> {
        self.format_notice
            .as_ref()
            .map(|(notice, error)| (notice.as_str(), *error))
    }

    pub(crate) fn search_notice(&self) -> Option<&str> {
        self.search_notice.as_deref()
    }

    pub(crate) fn format_pending(&self) -> bool {
        self.format_task.is_some()
    }

    pub(crate) fn match_count(&self) -> Option<(Option<usize>, usize)> {
        self.results.as_ref().map(|results| {
            let index = self.current_match.as_ref().and_then(|found| {
                results
                    .ranges
                    .binary_search_by_key(&found.start, |range| range.start)
                    .ok()
            });
            (index.map(|index| index + 1), results.ranges.len())
        })
    }

    pub(crate) fn selected_text(&self) -> Option<String> {
        if self.highlights_selection.is_empty() {
            let range = self.selection();
            return (!range.is_empty()).then(|| self.text[range].to_owned());
        }
        Some(
            self.highlights_selection
                .iter()
                .map(|range| &self.text[range.clone()])
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    pub(super) fn replace_selected_matches(&mut self, inserted: &str, cx: &mut Context<Self>) {
        let matches = &self.highlights_selection;
        let start = matches[0].start;
        let end = matches.last().unwrap().end;
        let removed = matches.iter().map(|range| range.len()).sum::<usize>();
        if inserted.len().saturating_mul(matches.len()) + self.text.len() - removed > MAX_BYTES {
            self.limit_reached = true;
            cx.notify();
            return;
        }
        let mut replacement = String::new();
        let mut written = start;
        for range in matches {
            replacement.push_str(&self.text[written..range.start]);
            replacement.push_str(inserted);
            written = range.end;
        }
        self.apply_change(start..end, replacement, None, cx);
    }

    pub(super) fn document_changed(&mut self, cx: &mut Context<Self>) {
        self.version = self.version.wrapping_add(1);
        self.format_notice = None;
        self.format_task = None;
        self.schedule_highlight(cx);
        self.schedule_search(cx);
    }

    pub(super) fn schedule_highlight(&mut self, cx: &mut Context<Self>) {
        if self.language == Language::Plain {
            return;
        }
        let version = self.version;
        let language = self.language;
        let draft = self.text.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let token = cancelled.clone();
        let task = cx.spawn(async move |view, cx| {
            let executor = cx.background_executor().clone();
            executor.timer(Duration::from_millis(90)).await;
            let spans = executor
                .spawn(async move {
                    editor_syntax::highlight(language, &draft, || token.load(Ordering::Relaxed))
                })
                .await;
            let _ = view.update(cx, |view, cx| {
                if view.version == version {
                    view.highlights = spans;
                    view.highlight_task = None;
                    cx.notify();
                }
            });
        });
        self.highlight_task = Some(EditorTask {
            cancelled,
            _task: task,
        });
    }

    pub(crate) fn set_search(&mut self, query: Option<SearchQuery>, cx: &mut Context<Self>) {
        let query = query.filter(|query| !query.text.is_empty());
        if self.query == query {
            return;
        }
        self.query = query;
        self.current_match = None;
        self.pending_search_action = None;
        self.schedule_search(cx);
        cx.notify();
    }

    fn schedule_search(&mut self, cx: &mut Context<Self>) {
        self.results = None;
        self.search_task = None;
        self.search_notice = None;
        let Some(query) = self.query.clone() else {
            return;
        };
        let version = self.version;
        let source = self.text.clone();
        let expected = query.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let token = cancelled.clone();
        let task = cx.spawn(async move |view, cx| {
            let executor = cx.background_executor().clone();
            executor.timer(Duration::from_millis(70)).await;
            let result = executor
                .spawn(async move { query.collect(&source, || token.load(Ordering::Relaxed)) })
                .await;
            let _ = view.update(cx, |view, cx| {
                if view.version != version || view.query.as_ref() != Some(&expected) {
                    return;
                }
                view.search_task = None;
                match result {
                    Ok(results) => {
                        view.results = Some(Arc::new(results));
                        if let Some((action, replacement)) = view.pending_search_action.take() {
                            view.search(action, &replacement, cx);
                        }
                    }
                    Err(error) => {
                        view.search_notice = Some(error);
                        view.pending_search_action = None;
                    }
                }
                cx.notify();
            });
        });
        self.search_task = Some(EditorTask {
            cancelled,
            _task: task,
        });
    }

    pub(crate) fn search(
        &mut self,
        action: SearchAction,
        replacement: &str,
        cx: &mut Context<Self>,
    ) {
        if self.is_composing() {
            return;
        }
        let mutates = matches!(action, SearchAction::Replace | SearchAction::ReplaceAll);
        if mutates && self.read_only {
            return;
        }
        let Some(results) = self.results.clone() else {
            if self.search_task.is_some() {
                self.pending_search_action = Some((action, replacement.to_owned()));
            }
            return;
        };
        if results.ranges.is_empty() {
            return;
        }
        let ranges = &results.ranges;
        self.search_notice = None;
        let selection = self.selection();
        let is_current = |range: &Range<usize>| {
            range == &selection && (!range.is_empty() || self.current_match.as_ref() == Some(range))
        };
        match action {
            SearchAction::Next => {
                let start = ranges.partition_point(|range| range.start < selection.end);
                let index =
                    (start + usize::from(ranges.get(start).is_some_and(is_current))) % ranges.len();
                self.select_match(ranges[index].clone());
            }
            SearchAction::Previous => {
                let start = ranges
                    .partition_point(|range| range.end <= selection.start && !is_current(range));
                let index = start.checked_sub(1).unwrap_or(ranges.len() - 1);
                self.select_match(ranges[index].clone());
            }
            SearchAction::SelectAll => {
                if ranges.len() > 1000 {
                    self.search_notice = Some("匹配超过 1000 处，请缩小范围。".into());
                } else {
                    self.select_match(ranges[0].clone());
                    self.highlights_selection = ranges.clone();
                }
            }
            SearchAction::Replace => {
                let start = ranges.partition_point(|range| range.start < selection.start);
                let range = &ranges[start % ranges.len()];
                if is_current(range) {
                    match results.replacement(&self.text, range, replacement) {
                        Ok(inserted) => {
                            let old_version = self.version;
                            self.apply_change(range.clone(), inserted, None, cx);
                            if self.limit_reached {
                                self.search_notice =
                                    Some("替换结果超过 2 MiB，草稿未修改。".into());
                            } else if old_version == self.version {
                                let next = ranges[(start + 1) % ranges.len()].clone();
                                self.select_match(next);
                            } else {
                                self.pending_search_action =
                                    Some((SearchAction::Next, String::new()));
                            }
                        }
                        Err(error) => self.search_notice = Some(error),
                    }
                } else {
                    self.select_match(range.clone());
                }
            }
            SearchAction::ReplaceAll => self.start_replace_all(replacement, cx),
        }
        cx.notify();
    }

    fn select_match(&mut self, range: Range<usize>) {
        self.move_caret(range.end, false);
        self.anchor = range.start;
        self.current_match = Some(range);
    }

    fn start_replace_all(&mut self, replacement: &str, cx: &mut Context<Self>) {
        let Some(results) = self.results.clone() else {
            return;
        };
        let count = results.ranges.len();
        let source = self.text.clone();
        let expected = self.query.clone();
        let version = self.version;
        let replacement = replacement.to_owned();
        self.search_notice = Some("正在替换…".into());
        let cancelled = Arc::new(AtomicBool::new(false));
        let token = cancelled.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    results.replace_all(&source, &replacement, || token.load(Ordering::Relaxed))
                })
                .await;
            let _ = view.update(cx, |view, cx| {
                if view.version != version || view.query != expected {
                    return;
                }
                view.search_task = None;
                match result {
                    Ok(output) => {
                        let old_position = view.caret;
                        let mut position = old_position.min(output.len());
                        while !output.is_char_boundary(position) {
                            position -= 1;
                        }
                        view.apply_change(
                            0..view.text.len(),
                            output,
                            Some([position, position]),
                            cx,
                        );
                        view.search_notice = Some(format!("已替换 {count} 处，Ctrl+Z 可撤销。"));
                    }
                    Err(error) => view.search_notice = Some(error),
                }
                cx.notify();
            });
        });
        self.search_task = Some(EditorTask {
            cancelled,
            _task: task,
        });
    }

    pub(crate) fn format_file(&mut self, mode: FormatMode, cx: &mut Context<Self>) {
        if self.read_only || self.name.is_empty() || self.is_composing() {
            return;
        }
        let draft = self.text.clone();
        let name = self.name.clone();
        let language = self.language;
        let version = self.version;
        let cancelled = Arc::new(AtomicBool::new(false));
        let token = cancelled.clone();
        self.format_notice = None;
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if token.load(Ordering::Relaxed) {
                        return Err("格式化已取消。".into());
                    }
                    editor_format::format(&draft, &name, language, mode)
                })
                .await;
            let _ = view.update(cx, |view, cx| {
                if view.version != version {
                    return;
                }
                view.format_task = None;
                let result = result.and_then(|(formatted, mode)| {
                    let selection = editor_format::map_selection(
                        &view.text,
                        &formatted,
                        [view.anchor, view.caret],
                    )?;
                    if formatted == view.text {
                        return Ok(if mode == FormatMode::Indent {
                            "当前语言没有可调整的缩进。"
                        } else {
                            "格式已规整，草稿没有变化。"
                        });
                    }
                    view.apply_change(0..view.text.len(), formatted, Some(selection), cx);
                    Ok(match mode {
                        FormatMode::Nginx => "已按 Nginx 格式化草稿，Ctrl+Z 可撤销。",
                        FormatMode::Json => "已按 JSON 格式化草稿，Ctrl+Z 可撤销。",
                        _ => "已整理当前语言缩进，Ctrl+Z 可撤销。",
                    })
                });
                view.format_notice = Some(match result {
                    Ok(notice) => (notice.into(), false),
                    Err(error) => (error, true),
                });
                cx.notify();
            });
        });
        self.format_task = Some(EditorTask {
            cancelled,
            _task: task,
        });
        cx.notify();
    }
}
