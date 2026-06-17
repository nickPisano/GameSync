//! Minimal parser for Valve's KeyValues (VDF) text format, used by Steam's
//! `libraryfolders.vdf` and `appmanifest_*.acf` files.
//!
//! Supports quoted and bare tokens, nested `{ }` objects, `\` escapes inside
//! quotes, and `//` line comments. This is intentionally small — enough to read
//! the handful of fields we need, not a general VDF library.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Vdf {
    Str(String),
    Obj(BTreeMap<String, Vdf>),
}

impl Vdf {
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Obj(m) => m.get(key),
            Vdf::Str(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Vdf::Str(s) => Some(s),
            Vdf::Obj(_) => None,
        }
    }

    pub fn as_obj(&self) -> Option<&BTreeMap<String, Vdf>> {
        match self {
            Vdf::Obj(m) => Some(m),
            Vdf::Str(_) => None,
        }
    }
}

#[derive(Debug)]
enum Tok {
    Open,
    Close,
    Str(String),
}

fn tokenize(input: &str) -> Vec<Tok> {
    let mut toks = Vec::new();
    let mut it = input.chars().peekable();
    while let Some(&c) = it.peek() {
        if c.is_whitespace() {
            it.next();
        } else if c == '{' {
            it.next();
            toks.push(Tok::Open);
        } else if c == '}' {
            it.next();
            toks.push(Tok::Close);
        } else if c == '/' {
            it.next();
            if it.peek() == Some(&'/') {
                // line comment
                for n in it.by_ref() {
                    if n == '\n' {
                        break;
                    }
                }
            }
        } else if c == '"' {
            it.next();
            let mut buf = String::new();
            while let Some(&n) = it.peek() {
                if n == '"' {
                    it.next();
                    break;
                } else if n == '\\' {
                    it.next();
                    if let Some(&e) = it.peek() {
                        it.next();
                        buf.push(match e {
                            'n' => '\n',
                            't' => '\t',
                            '\\' => '\\',
                            '"' => '"',
                            other => other,
                        });
                    }
                } else {
                    buf.push(n);
                    it.next();
                }
            }
            toks.push(Tok::Str(buf));
        } else {
            // bare token
            let mut buf = String::new();
            while let Some(&n) = it.peek() {
                if n.is_whitespace() || n == '{' || n == '}' {
                    break;
                }
                buf.push(n);
                it.next();
            }
            if !buf.is_empty() {
                toks.push(Tok::Str(buf));
            }
        }
    }
    toks
}

fn parse_obj(toks: &[Tok], pos: &mut usize) -> Vdf {
    let mut map = BTreeMap::new();
    while *pos < toks.len() {
        match &toks[*pos] {
            Tok::Close => {
                *pos += 1;
                break;
            }
            Tok::Open => {
                // anonymous block; consume and discard
                *pos += 1;
                let _ = parse_obj(toks, pos);
            }
            Tok::Str(key) => {
                let key = key.clone();
                *pos += 1;
                match toks.get(*pos) {
                    Some(Tok::Open) => {
                        *pos += 1;
                        map.insert(key, parse_obj(toks, pos));
                    }
                    Some(Tok::Str(val)) => {
                        let val = val.clone();
                        *pos += 1;
                        map.insert(key, Vdf::Str(val));
                    }
                    Some(Tok::Close) => {
                        *pos += 1;
                        break;
                    }
                    None => break,
                }
            }
        }
    }
    Vdf::Obj(map)
}

/// Parse a VDF document into a top-level object.
pub fn parse(input: &str) -> Vdf {
    let toks = tokenize(input);
    let mut pos = 0;
    parse_obj(&toks, &mut pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_appmanifest() {
        let s = r#"
            "AppState"
            {
                "appid"      "374320"
                "name"       "DARK SOULS III"
                "installdir" "DARK SOULS III"
                "UserConfig"
                {
                    "language" "english"
                }
            }
        "#;
        let v = parse(s);
        let app = v.get("AppState").unwrap();
        assert_eq!(app.get("appid").unwrap().as_str(), Some("374320"));
        assert_eq!(app.get("name").unwrap().as_str(), Some("DARK SOULS III"));
        assert_eq!(
            app.get("installdir").unwrap().as_str(),
            Some("DARK SOULS III")
        );
    }

    #[test]
    fn parses_libraryfolders() {
        let s = r#"
            "libraryfolders"
            {
                "0"
                {
                    "path"  "C:\\Program Files (x86)\\Steam"
                }
                "1"
                {
                    "path"  "D:\\SteamLibrary"
                }
            }
        "#;
        let v = parse(s);
        let lf = v.get("libraryfolders").unwrap().as_obj().unwrap();
        let paths: Vec<&str> = lf
            .values()
            .filter_map(|e| e.get("path").and_then(|p| p.as_str()))
            .collect();
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&"D:\\SteamLibrary"));
    }
}
