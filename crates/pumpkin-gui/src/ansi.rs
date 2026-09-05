//! Turns ANSI-formatted console text into what the window needs to show it: an HTML fragment
//! carrying the real colours/attributes/hyperlinks, and a plain string for search, copy and save.

use std::fmt::Write as _;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderedLine {
    pub plain: String,
    pub html: String,
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
    fn push_css(self, out: &mut String) {
        if let Some((r, g, b)) = self.fg {
            let _ = write!(out, "color:#{r:02x}{g:02x}{b:02x};");
        }
        if self.bold {
            out.push_str("font-weight:bold;");
        }
        if self.italic {
            out.push_str("font-style:italic;");
        }
        match (self.underline, self.strike) {
            (true, true) => out.push_str("text-decoration:underline line-through;"),
            (true, false) => out.push_str("text-decoration:underline;"),
            (false, true) => out.push_str("text-decoration:line-through;"),
            (false, false) => {}
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
            // `to_pretty_console`, so it is deliberately left unhandled rather than mis-rendered.
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

fn push_escaped(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
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

/// Placeholder colour for a link with no ANSI colour of its own. Qt Quick's `TextEdit` has no
/// `linkColor` property to redirect, and an anchor without an explicit `color` in its style
/// renders in Qt's own built-in link blue regardless of any ancestor's colour.
pub const DEFAULT_LINK_COLOR: &str = "@default-link-color@";

/// Emits a link, always with an explicit colour: `color` if this run had an ANSI one, otherwise
/// [`DEFAULT_LINK_COLOR`]. Underline is the only attribute a link adds over the surrounding text.
fn push_link(url: &str, label: &str, color: Option<(u8, u8, u8)>, out: &mut String) {
    out.push_str("<a href=\"");
    push_escaped(url, out);
    out.push_str("\" style=\"text-decoration:underline;color:");
    match color {
        Some((r, g, b)) => {
            let _ = write!(out, "#{r:02x}{g:02x}{b:02x}");
        }
        None => out.push_str(DEFAULT_LINK_COLOR),
    }
    out.push_str(";\">");
    push_escaped(label, out);
    out.push_str("</a>");
}

/// Escapes `text` and wraps any bare URLs it contains in `<a href>`, coloured like the rest of
/// `color`'s run.
fn linkify(text: &str, color: Option<(u8, u8, u8)>, out: &mut String) {
    let mut rest = text;
    while let Some((start, end)) = next_url(rest) {
        push_escaped(&rest[..start], out);
        push_link(&rest[start..end], &rest[start..end], color, out);
        rest = &rest[end..];
    }
    push_escaped(rest, out);
}

/// Emits one run of text as HTML: a `<span>` only if some style is actually active, and either an
/// explicit OSC 8 link or auto-linkified bare URLs.
fn flush_run(html: &mut String, run: &str, style: Style, link: Option<&str>) {
    if run.is_empty() {
        return;
    }

    let mut css = String::new();
    style.push_css(&mut css);
    let wrap = !css.is_empty();

    if wrap {
        html.push_str("<span style=\"");
        html.push_str(&css);
        html.push_str("\">");
    }

    match link {
        Some(url) => push_link(url, run, style.fg, html),
        None => linkify(run, style.fg, html),
    }

    if wrap {
        html.push_str("</span>");
    }
}

/// Parses `text`'s SGR colour/attribute codes and OSC 8 hyperlinks into an HTML fragment, and
/// separately strips all of it down to what a human would read.
#[must_use]
pub fn render(text: &str) -> RenderedLine {
    let mut plain = String::with_capacity(text.len());
    let mut html = String::with_capacity(text.len());

    let mut style = Style::default();
    let mut link: Option<String> = None;
    let mut run = String::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            // A stray carriage return (Windows-style line endings smuggled into a message) would
            // otherwise show up as a gibberish in the rich-text view.
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
                flush_run(&mut html, &run, style, link.as_deref());
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
                    flush_run(&mut html, &run, style, link.as_deref());
                    run.clear();
                    link = (!url.is_empty()).then(|| url.to_owned());
                }
            }
            // Any other escape (or a lone ESC at the end of the text): drop just the introducer
            // so a sequence this parser does not know about cannot swallow the rest of the line.
            Some(_) | None => {}
        }
    }

    flush_run(&mut html, &run, style, link.as_deref());
    RenderedLine { plain, html }
}
