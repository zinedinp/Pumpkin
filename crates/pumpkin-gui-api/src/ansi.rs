//! Turns ANSI-formatted console text into what a window needs to show it: a list of styled runs
//! carrying the real colours/attributes/hyperlinks, and a plain string for search, copy and save.

use serde::{Deserialize, Serialize};

/// One stretch of a log line that shares a single appearance.
///
/// Deliberately toolkit-neutral: colours are plain RGB, and a run with no colour of its own is
/// drawn in whatever the frontend uses for that log level.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyledRun {
    pub text: String,
    /// `None` when the run carries no ANSI colour of its own.
    pub color: Option<(u8, u8, u8)>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// Hyperlink target, from an OSC 8 sequence or a bare URL. Empty when the run is not a link.
    pub link: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderedLine {
    pub plain: String,
    pub runs: Vec<StyledRun>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct Style {
    fg: Option<(u8, u8, u8)>,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
}

impl Style {
    /// A run carrying this style, with `text` and `link` filled in by the caller.
    fn run(self, text: &str, link: &str) -> StyledRun {
        StyledRun {
            text: text.to_owned(),
            color: self.fg,
            bold: self.bold,
            italic: self.italic,
            // A link is underlined on top of whatever the surrounding text does.
            underline: self.underline || !link.is_empty(),
            strike: self.strike,
            link: link.to_owned(),
        }
    }
}

/// The standard xterm default palette. A real terminal may render its 16 base colours
/// differently depending on its own theme; this is the closest fixed approximation available
/// without knowing the user's terminal.
const fn ansi16(index: u8, bright: bool) -> (u8, u8, u8) {
    match (index, bright) {
        (0, false) => (0x00, 0x00, 0x00),
        (1, false) => (0xcd, 0x00, 0x00),
        (2, false) => (0x00, 0xcd, 0x00),
        (3, false) => (0xcd, 0xcd, 0x00),
        (4, false) => (0x00, 0x00, 0xee),
        (5, false) => (0xcd, 0x00, 0xcd),
        (6, false) => (0x00, 0xcd, 0xcd),
        (7, false) => (0xe5, 0xe5, 0xe5),
        (0, true) => (0x7f, 0x7f, 0x7f),
        (1, true) => (0xff, 0x00, 0x00),
        (2, true) => (0x00, 0xff, 0x00),
        (3, true) => (0xff, 0xff, 0x00),
        (4, true) => (0x5c, 0x5c, 0xff),
        (5, true) => (0xff, 0x00, 0xff),
        (6, true) => (0x00, 0xff, 0xff),
        _ => (0xff, 0xff, 0xff),
    }
}

/// Applies one SGR parameter list (the digits between `\x1b[` and the terminating letter).
fn apply_sgr(style: &mut Style, raw: &str) {
    if raw.is_empty() {
        // Bare `CSI m` means reset, same as `CSI 0 m`.
        *style = Style::default();
        return;
    }

    let params: Vec<i64> = raw.split(';').map(|p| p.parse().unwrap_or(0)).collect();
    let mut i = 0;
    while i < params.len() {
        match params[i] {
            0 => *style = Style::default(),
            1 => style.bold = true,
            3 => style.italic = true,
            4 => style.underline = true,
            9 => style.strike = true,
            22 => style.bold = false,
            23 => style.italic = false,
            24 => style.underline = false,
            29 => style.strike = false,
            39 => style.fg = None,
            // Truecolour (`38;2;r;g;b`); 256-colour (`38;5;n`) is not produced by `colored` or
            // `to_pretty_console`, so it is deliberately left unhandled rather than miss-rendered.
            38 if params.get(i + 1) == Some(&2) && i + 4 < params.len() => {
                let clamp = |v: i64| -> u8 { u8::try_from(v.clamp(0, 255)).unwrap_or(0) };
                style.fg = Some((
                    clamp(params[i + 2]),
                    clamp(params[i + 3]),
                    clamp(params[i + 4]),
                ));
                i += 4;
            }
            n @ 30..=37 => style.fg = Some(ansi16(u8::try_from(n - 30).unwrap_or(0), false)),
            n @ 90..=97 => style.fg = Some(ansi16(u8::try_from(n - 90).unwrap_or(0), true)),
            _ => {}
        }
        i += 1;
    }
}

/// Finds the next bare `http(s)://` URL in `text`, trimming trailing punctuation that reads more
/// like the end of a sentence than part of the link
fn next_url(text: &str) -> Option<(usize, usize)> {
    let start = ["https://", "http://"]
        .into_iter()
        .filter_map(|prefix| text.find(prefix))
        .min()?;

    let rest = &text[start..];
    let mut end = rest
        .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\''))
        .unwrap_or(rest.len());

    while end > 0 {
        let last = rest[..end].chars().next_back().unwrap_or_default();
        if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}') {
            end -= last.len_utf8();
        } else {
            break;
        }
    }

    (end > 0).then_some((start, start + end))
}

/// Splits `text` on the bare URLs it contains, so each one becomes its own link run.
fn push_linkified(out: &mut Vec<StyledRun>, text: &str, style: Style) {
    let mut rest = text;
    while let Some((start, end)) = next_url(rest) {
        if start > 0 {
            out.push(style.run(&rest[..start], ""));
        }
        let url = &rest[start..end];
        out.push(style.run(url, url));
        rest = &rest[end..];
    }
    if !rest.is_empty() {
        out.push(style.run(rest, ""));
    }
}

/// Emits one run of text: an explicit OSC 8 link stays whole, anything else is split on bare URLs.
fn flush_run(out: &mut Vec<StyledRun>, run: &str, style: Style, link: Option<&str>) {
    if run.is_empty() {
        return;
    }

    match link {
        Some(url) => out.push(style.run(run, url)),
        None => push_linkified(out, run, style),
    }
}

/// Parses `text`'s SGR colour/attribute codes and OSC 8 hyperlinks into styled runs, and
/// separately strips all of it down to what a human would read.
#[must_use]
pub fn render(text: &str) -> RenderedLine {
    let mut plain = String::with_capacity(text.len());
    let mut runs = Vec::new();

    let mut style = Style::default();
    let mut link: Option<String> = None;
    let mut run = String::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            // A stray carriage return (Windows-style line endings smuggled into a message) would
            // otherwise show up as gibberish in the log view.
            if c != '\r' {
                run.push(c);
                plain.push(c);
            }
            continue;
        }

        match chars.peek() {
            Some('[') => {
                chars.next();
                let mut params = String::new();
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                    params.push(next);
                }
                flush_run(&mut runs, &run, style, link.as_deref());
                run.clear();
                apply_sgr(&mut style, &params);
            }
            Some(']') => {
                chars.next();
                let mut payload = String::new();
                loop {
                    match chars.next() {
                        None | Some('\u{7}') => break,
                        Some('\u{1b}') if chars.peek() == Some(&'\\') => {
                            chars.next();
                            break;
                        }
                        Some(other) => payload.push(other),
                    }
                }

                // Pumpkin only ever emits `8;;<url>` (open) and `8;;` (close).
                if let Some(rest) = payload.strip_prefix("8;") {
                    let url = rest.split_once(';').map_or(rest, |(_, uri)| uri);
                    flush_run(&mut runs, &run, style, link.as_deref());
                    run.clear();
                    link = (!url.is_empty()).then(|| url.to_owned());
                }
            }
            // Any other escape (or a lone ESC at the end of the text): drop just the introducer
            // so a sequence this parser does not know about cannot swallow the rest of the line.
            Some(_) | None => {}
        }
    }

    flush_run(&mut runs, &run, style, link.as_deref());
    RenderedLine { plain, runs }
}
