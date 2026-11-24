//! Steam's text KeyValues format (`loginusers.vdf`, `config.vdf`, `libraryfolders.vdf`).

use anyhow::{Result, bail};

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Str(String),
    Map(Vec<(String, Node)>),
}

impl Node {
    pub fn get(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Map(entries) => entries
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            Node::Str(_) => None,
        }
    }

    pub fn get_path(&self, path: &[&str]) -> Option<&Node> {
        path.iter().try_fold(self, |node, key| node.get(key))
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Node::Str(s) => Some(s),
            Node::Map(_) => None,
        }
    }

    pub fn entries(&self) -> &[(String, Node)] {
        match self {
            Node::Map(entries) => entries,
            Node::Str(_) => &[],
        }
    }

    /// The child map under `key`, created (as an empty map) if missing.
    pub fn child_map(&mut self, key: &str) -> &mut Node {
        let Node::Map(entries) = self else {
            panic!("child_map called on a string node");
        };
        let idx = match entries
            .iter()
            .position(|(k, _)| k.eq_ignore_ascii_case(key))
        {
            Some(i) => {
                if !matches!(entries[i].1, Node::Map(_)) {
                    entries[i].1 = Node::Map(Vec::new());
                }
                i
            }
            None => {
                entries.push((key.to_string(), Node::Map(Vec::new())));
                entries.len() - 1
            }
        };
        &mut entries[idx].1
    }

    pub fn set(&mut self, key: &str, value: Node) {
        if let Node::Map(entries) = self {
            match entries
                .iter_mut()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
            {
                Some((_, v)) => *v = value,
                None => entries.push((key.to_string(), value)),
            }
        }
    }

    pub fn remove(&mut self, key: &str) {
        if let Node::Map(entries) = self {
            entries.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
        }
    }
}

/// Parses a whole file into a map node holding its top-level entries.
pub fn parse(text: &str) -> Result<Node> {
    let mut tokens = Tokenizer {
        chars: text.char_indices().peekable(),
        text,
    };
    let entries = parse_entries(&mut tokens, true)?;
    Ok(Node::Map(entries))
}

enum Token {
    Str(String),
    Open,
    Close,
}

struct Tokenizer<'a> {
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    text: &'a str,
}

impl Tokenizer<'_> {
    fn next_token(&mut self) -> Result<Option<Token>> {
        loop {
            let Some(&(i, c)) = self.chars.peek() else {
                return Ok(None);
            };
            match c {
                c if c.is_whitespace() => {
                    self.chars.next();
                }
                '/' if self.text[i..].starts_with("//") => {
                    for (_, c) in self.chars.by_ref() {
                        if c == '\n' {
                            break;
                        }
                    }
                }
                '{' => {
                    self.chars.next();
                    return Ok(Some(Token::Open));
                }
                '}' => {
                    self.chars.next();
                    return Ok(Some(Token::Close));
                }
                '"' => {
                    self.chars.next();
                    let mut s = String::new();
                    loop {
                        match self.chars.next() {
                            None => bail!("unterminated string"),
                            Some((_, '"')) => break,
                            Some((_, '\\')) => match self.chars.next() {
                                Some((_, 'n')) => s.push('\n'),
                                Some((_, 't')) => s.push('\t'),
                                Some((_, other)) => s.push(other),
                                None => bail!("unterminated string"),
                            },
                            Some((_, other)) => s.push(other),
                        }
                    }
                    return Ok(Some(Token::Str(s)));
                }
                _ => {
                    // Unquoted token (rare in Steam's files, but valid).
                    let mut s = String::new();
                    while let Some(&(_, c)) = self.chars.peek() {
                        if c.is_whitespace() || c == '{' || c == '}' || c == '"' {
                            break;
                        }
                        s.push(c);
                        self.chars.next();
                    }
                    return Ok(Some(Token::Str(s)));
                }
            }
        }
    }
}

fn parse_entries(tokens: &mut Tokenizer, top: bool) -> Result<Vec<(String, Node)>> {
    let mut entries = Vec::new();
    loop {
        let key = match tokens.next_token()? {
            None if top => return Ok(entries),
            None => bail!("missing closing brace"),
            Some(Token::Close) if !top => return Ok(entries),
            Some(Token::Close) => bail!("unexpected closing brace"),
            Some(Token::Open) => bail!("unexpected opening brace"),
            Some(Token::Str(s)) => s,
        };
        let value = match tokens.next_token()? {
            Some(Token::Str(s)) => Node::Str(s),
            Some(Token::Open) => Node::Map(parse_entries(tokens, false)?),
            _ => bail!("key \"{key}\" has no value"),
        };
        entries.push((key, value));
    }
}

/// Writes a map node in Steam's own layout (tabs, braces on their own lines).
pub fn serialize(node: &Node) -> String {
    let mut out = String::new();
    write_entries(node.entries(), 0, &mut out);
    out
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn write_entries(entries: &[(String, Node)], depth: usize, out: &mut String) {
    let indent = "\t".repeat(depth);
    for (key, value) in entries {
        match value {
            Node::Str(s) => {
                out.push_str(&format!(
                    "{indent}\"{}\"\t\t\"{}\"\n",
                    escape(key),
                    escape(s)
                ));
            }
            Node::Map(children) => {
                out.push_str(&format!("{indent}\"{}\"\n{indent}{{\n", escape(key)));
                write_entries(children, depth + 1, out);
                out.push_str(&format!("{indent}}}\n"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_write() {
        let text = "\"users\"\n{\n\t\"765\"\n\t{\n\t\t\"PersonaName\"\t\t\"A \\\"q\\\" b\"\n\t\t\"Path\"\t\t\"C:\\\\x\"\n\t}\n}\n";
        let node = parse(text).unwrap();
        assert_eq!(
            node.get_path(&["users", "765", "PersonaName"])
                .and_then(Node::as_str),
            Some("A \"q\" b")
        );
        assert_eq!(
            node.get_path(&["users", "765", "path"])
                .and_then(Node::as_str),
            Some("C:\\x")
        );
        assert_eq!(serialize(&node), text);
    }
}
