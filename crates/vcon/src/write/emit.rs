//! Compact JSON with members in a fixed order, and the depth bound that
//! `read::inspect` applies.

use serde_json::Value;

use super::WriteError;
use crate::read::MAX_JSON_DEPTH;

/// A JSON value whose objects keep their members in the order given.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Node {
    /// A string.
    Str(String),
    /// A number or literal, already written.
    Raw(String),
    /// An array.
    Arr(Vec<Node>),
    /// An object, its members in order.
    Obj(Vec<(String, Node)>),
    /// A value given by the caller, written as serde_json writes it.
    Val(Value),
}

impl Node {
    pub(super) fn str(text: &str) -> Node {
        Node::Str(text.to_owned())
    }
}

/// An object under construction: members are added in the order written.
#[derive(Debug, Default)]
pub(super) struct Obj(Vec<(String, Node)>);

impl Obj {
    pub(super) fn put(&mut self, name: &str, value: Node) {
        self.0.push((name.to_owned(), value));
    }

    pub(super) fn put_str(&mut self, name: &str, value: &Option<String>) {
        if let Some(text) = value {
            self.put(name, Node::str(text));
        }
    }

    pub(super) fn done(self) -> Node {
        Node::Obj(self.0)
    }
}

/// Writes `node` as compact JSON, refusing output with more than
/// [`MAX_JSON_DEPTH`] arrays and objects open at once.
pub(super) fn render(node: &Node) -> Result<Vec<u8>, WriteError> {
    let mut out = Vec::new();
    write(node, 0, &mut out)?;
    Ok(out)
}

fn quoted(text: &str, out: &mut Vec<u8>) {
    // Serializing a str cannot fail; an empty string is never written,
    // since every string has at least its quotes.
    out.extend_from_slice(serde_json::to_string(text).unwrap_or_default().as_bytes());
}

fn open(depth: usize) -> Result<usize, WriteError> {
    if depth >= MAX_JSON_DEPTH {
        return Err(WriteError::TooDeep);
    }
    Ok(depth + 1)
}

fn write(node: &Node, depth: usize, out: &mut Vec<u8>) -> Result<(), WriteError> {
    match node {
        Node::Str(text) => quoted(text, out),
        Node::Raw(text) => out.extend_from_slice(text.as_bytes()),
        Node::Arr(items) => {
            let inner = open(depth)?;
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write(item, inner, out)?;
            }
            out.push(b']');
        }
        Node::Obj(members) => {
            let inner = open(depth)?;
            out.push(b'{');
            for (i, (name, value)) in members.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                quoted(name, out);
                out.push(b':');
                write(value, inner, out)?;
            }
            out.push(b'}');
        }
        Node::Val(value) => write_value(value, depth, out)?,
    }
    Ok(())
}

fn write_value(value: &Value, depth: usize, out: &mut Vec<u8>) -> Result<(), WriteError> {
    match value {
        Value::Array(items) => {
            let inner = open(depth)?;
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_value(item, inner, out)?;
            }
            out.push(b']');
        }
        Value::Object(members) => {
            let inner = open(depth)?;
            out.push(b'{');
            for (i, (name, item)) in members.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                quoted(name, out);
                out.push(b':');
                write_value(item, inner, out)?;
            }
            out.push(b'}');
        }
        Value::String(text) => quoted(text, out),
        // null, booleans and numbers, as serde_json writes them.
        other => out.extend_from_slice(other.to_string().as_bytes()),
    }
    Ok(())
}
