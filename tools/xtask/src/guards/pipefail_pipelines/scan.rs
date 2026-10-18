//! Finds the reader of every pipe in a shell script.
//!
//! The scanner knows only enough shell to tell a pipe from text that
//! looks like one: quotes, escapes, comments, `||`, `$(...)` nested in
//! double quotes, and here-document bodies. It reads each reader's
//! words with their quotes removed.

/// The stage that reads from one `|`.
pub(crate) struct Reader {
    /// The 1-based line of the reader's first word.
    pub(crate) line: usize,
    pub(crate) words: Vec<String>,
}

#[derive(Clone, Copy)]
enum Context {
    /// Shell code; `depth` counts open plain parentheses so that only
    /// the `)` that closes a `$(` leaves the substitution.
    Code {
        depth: usize,
    },
    DoubleQuoted,
}

/// Returns the reader of every pipe in `text`, in source order.
pub(crate) fn readers(text: &str) -> Vec<Reader> {
    let chars: Vec<char> = text.chars().collect();
    let mut readers = Vec::new();
    let mut stack = vec![Context::Code { depth: 0 }];
    let mut pending_heredocs: Vec<(String, bool)> = Vec::new();
    let mut index = 0;
    while let Some(&c) = chars.get(index) {
        let next = chars.get(index + 1).copied();
        let top = stack.last().copied().unwrap_or(Context::Code { depth: 0 });
        index = match (top, c) {
            (_, '\\') => index + 2,
            (Context::DoubleQuoted, '"') => {
                stack.pop();
                index + 1
            }
            (_, '$') if next == Some('(') => {
                stack.push(Context::Code { depth: 0 });
                index + 2
            }
            (Context::DoubleQuoted, _) => index + 1,
            (Context::Code { .. }, '"') => {
                stack.push(Context::DoubleQuoted);
                index + 1
            }
            (Context::Code { .. }, '\'') => closing_single_quote(&chars, index),
            (Context::Code { .. }, '#') if starts_word(&chars, index) => line_end(&chars, index),
            (Context::Code { depth }, '(') => {
                set_depth(&mut stack, depth + 1);
                index + 1
            }
            (Context::Code { depth }, ')') => {
                if depth > 0 {
                    set_depth(&mut stack, depth - 1);
                } else if stack.len() > 1 {
                    stack.pop();
                }
                index + 1
            }
            (Context::Code { .. }, '<') if next == Some('<') => {
                heredoc_start(&chars, index, &mut pending_heredocs)
            }
            (Context::Code { .. }, '\n') => pending_heredocs
                .drain(..)
                .fold(index + 1, |body, (delimiter, strip_tabs)| {
                    heredoc_end(&chars, body, &delimiter, strip_tabs)
                }),
            (Context::Code { .. }, '|') if next == Some('|') => index + 2,
            (Context::Code { .. }, '|') => {
                let preceded_by_clobber = index
                    .checked_sub(1)
                    .and_then(|previous| chars.get(previous))
                    == Some(&'>');
                let stage = if next == Some('&') {
                    index + 2
                } else {
                    index + 1
                };
                if !preceded_by_clobber {
                    readers.extend(reader_at(&chars, stage));
                }
                stage
            }
            (Context::Code { .. }, _) => index + 1,
        };
    }
    readers
}

fn set_depth(stack: &mut [Context], depth: usize) {
    if let Some(top) = stack.last_mut() {
        *top = Context::Code { depth };
    }
}

fn starts_word(chars: &[char], index: usize) -> bool {
    index
        .checked_sub(1)
        .and_then(|previous| chars.get(previous))
        .is_none_or(|before| before.is_whitespace() || matches!(before, ';' | '(' | '|' | '&'))
}

fn line_end(chars: &[char], index: usize) -> usize {
    chars
        .iter()
        .skip(index)
        .position(|c| *c == '\n')
        .map_or(chars.len(), |offset| index + offset)
}

fn closing_single_quote(chars: &[char], index: usize) -> usize {
    chars
        .iter()
        .skip(index + 1)
        .position(|c| *c == '\'')
        .map_or(chars.len(), |offset| index + offset + 2)
}

/// Index of the first character at or after `index` that is not a
/// space or a tab.
fn skip_blanks(chars: &[char], index: usize) -> usize {
    chars
        .iter()
        .skip(index)
        .position(|c| *c != ' ' && *c != '\t')
        .map_or(chars.len(), |offset| index + offset)
}

/// Records the delimiter of a `<<` or `<<-` here-document and returns
/// the index after it. A `<<<` here-string has no body.
fn heredoc_start(chars: &[char], index: usize, pending: &mut Vec<(String, bool)>) -> usize {
    let after = index + 2;
    if chars.get(after) == Some(&'<') {
        return after + 1;
    }
    let strip_tabs = chars.get(after) == Some(&'-');
    let cursor = skip_blanks(chars, if strip_tabs { after + 1 } else { after });
    let (delimiter, end) = word_at(chars, cursor);
    if !delimiter.is_empty() {
        pending.push((delimiter, strip_tabs));
    }
    end
}

/// Returns the index after the line that ends a here-document body.
fn heredoc_end(chars: &[char], mut index: usize, delimiter: &str, strip_tabs: bool) -> usize {
    while index < chars.len() {
        let end = line_end(chars, index);
        let line: String = chars.get(index..end).unwrap_or_default().iter().collect();
        index = end + 1;
        let line = if strip_tabs {
            line.trim_start_matches('\t')
        } else {
            line.as_str()
        };
        if line == delimiter {
            break;
        }
    }
    index
}

/// Reads the words of the stage that starts at `index`. A pipe at the
/// end of a line continues on the next line.
fn reader_at(chars: &[char], mut index: usize) -> Option<Reader> {
    let mut words = Vec::new();
    let mut line = None;
    loop {
        index = skip_blanks(chars, index);
        let continued = chars.get(index) == Some(&'\\') && chars.get(index + 1) == Some(&'\n');
        if continued || (words.is_empty() && chars.get(index) == Some(&'\n')) {
            index = if continued { index + 2 } else { index + 1 };
            continue;
        }
        match chars.get(index) {
            None | Some('\n' | ';' | '&' | '|' | ')' | '<' | '>' | '#') => break,
            Some(_) => {}
        }
        line.get_or_insert_with(|| 1 + chars.iter().take(index).filter(|c| **c == '\n').count());
        let (word, end) = word_at(chars, index);
        words.push(word);
        index = end;
    }
    line.map(|line| Reader { line, words })
}

/// Reads one shell word at `index`, removing its quotes, and returns
/// it with the index after it.
fn word_at(chars: &[char], mut index: usize) -> (String, usize) {
    let mut word = String::new();
    let mut quote = None;
    while let Some(&c) = chars.get(index) {
        index = match (quote, c) {
            (None, ' ' | '\t' | '\n' | ';' | '&' | '|' | ')' | '<' | '>') => break,
            (None | Some('"'), '\\') => {
                word.extend(chars.get(index + 1));
                index + 2
            }
            (None, '\'' | '"') => {
                quote = Some(c);
                index + 1
            }
            (Some(open), _) if open == c => {
                quote = None;
                index + 1
            }
            _ => {
                word.push(c);
                index + 1
            }
        };
    }
    (word, index)
}
