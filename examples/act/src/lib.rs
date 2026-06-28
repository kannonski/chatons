//! act-chaton — make your terminal output *interactive*.
//!
//! Reads the pane you opened it from (`host::source_text`), finds the actionable things on
//! screen — URLs, file:line references, git hashes — and lets you act on the one you pick:
//! open a URL, jump to a file in nvim, `git show` a commit. The thing no terminal does on its
//! own: understand and act on its own output. `j`/`k` select, `enter` acts, `q` quits.

// wit-bindgen 0.36's generated export glue isn't edition-2024 lint-clean yet (unsafe-in-unsafe).
#![allow(unsafe_op_in_unsafe_fn)]

wit_bindgen::generate!({ world: "chaton", path: "../../wit" });

use chatons::plugin::host;
use regex::Regex;
use std::cell::RefCell;
use std::collections::HashSet;

struct Item {
    kind: &'static str, // "url" | "file" | "hash"
    glyph: &'static str,
    text: String,
}

fn scan(text: &str) -> Vec<Item> {
    let url_re = Regex::new(r#"https?://[^\s)>\]}'"]+"#).unwrap();
    let file_re = Regex::new(r"[\w.+~/-]+\.[A-Za-z][\w]{0,9}(?::\d+(?::\d+)?)?").unwrap();
    let hash_re = Regex::new(r"\b[0-9a-f]{7,40}\b").unwrap();

    let trim = |s: &str| {
        s.trim_end_matches(['.', ',', ')', ']', '}', ':', '\'', '"']).to_string()
    };
    let urls: Vec<String> = url_re
        .find_iter(text)
        .map(|m| trim(m.as_str()))
        .filter(|s| !s.is_empty())
        .collect();
    let in_url = |t: &str| urls.iter().any(|u| u.contains(t));

    let mut items = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for u in &urls {
        if seen.insert(u.clone()) {
            items.push(Item { kind: "url", glyph: "🔗", text: u.clone() });
        }
    }
    for m in hash_re.find_iter(text) {
        let t = m.as_str().to_string();
        if !in_url(&t) && seen.insert(t.clone()) {
            items.push(Item { kind: "hash", glyph: "◆", text: t });
        }
    }
    for m in file_re.find_iter(text) {
        let s = m.as_str();
        // require a path-like signal so bare domains/words don't match
        if (s.contains('/') || s.contains(':')) && !in_url(s) && seen.insert(s.to_string()) {
            items.push(Item { kind: "file", glyph: "📄", text: s.to_string() });
        }
    }
    items
}

struct Act {
    items: Vec<Item>,
    cur: usize,
}

impl Act {
    fn new() -> Self {
        Act { items: scan(&host::source_text()), cur: 0 }
    }

    fn draw(&self) {
        let mut s = String::from("\n  🐈 chatons — act on screen\n\n");
        if self.items.is_empty() {
            s.push_str("  nothing actionable on the previous screen\n");
        }
        for (i, it) in self.items.iter().enumerate() {
            let cursor = if i == self.cur { "▌" } else { " " };
            s.push_str(&format!("  {cursor} {} {}\n", it.glyph, it.text));
        }
        let hint = self.items.get(self.cur).map_or("", |it| match it.kind {
            "url" => "open in browser",
            "file" => "open in nvim",
            "hash" => "git show",
            _ => "",
        });
        s.push_str(&format!("\n  j/k select · ↵ {hint} · q quit\n"));
        host::render(&s);
    }

    fn act(&self) {
        let Some(it) = self.items.get(self.cur) else { return };
        let cmd = match it.kind {
            "url" => format!("launch --type=background xdg-open {}", it.text),
            "hash" => format!("launch --type=tab --cwd=current git show {}", it.text),
            "file" => {
                let mut parts = it.text.splitn(3, ':');
                let file = parts.next().unwrap_or("");
                let line = parts.next().filter(|l| l.chars().all(|c| c.is_ascii_digit()));
                match line {
                    Some(l) => format!("launch --type=tab --cwd=current nvim +{l} {file}"),
                    None => format!("launch --type=tab --cwd=current nvim {file}"),
                }
            }
            _ => return,
        };
        host::kitty(&cmd);
    }
}

thread_local! {
    static STATE: RefCell<Act> = RefCell::new(Act::new());
}

struct App;

impl Guest for App {
    fn init() {
        STATE.with_borrow(|s| s.draw());
    }

    fn on_key(k: Key) -> bool {
        STATE.with_borrow_mut(|s| {
            match k {
                Key::Text('q') | Key::Escape => return false,
                Key::Text('j') => {
                    if s.cur + 1 < s.items.len() {
                        s.cur += 1;
                    }
                }
                Key::Text('k') => s.cur = s.cur.saturating_sub(1),
                Key::Enter => {
                    s.act();
                    return false;
                }
                _ => {}
            }
            s.draw();
            true
        })
    }
}

export!(App);
