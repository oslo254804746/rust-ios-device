//! Two passes over the same retained body: validate/count without allocations,
//! then exact bank admission before creating the explicit borrowed arena.
//! Strings/data borrow the original charged body; no IndexMap/XpcValue exists.
//! Container wire lengths and trailing bytes follow decode_message semantics.
use super::*;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Clone, Copy)]
enum Kind<'a> {
    Other,
    String(&'a str),
    Uint64(u64),
    Array,
    Dictionary,
    FileTransfer,
}
#[derive(Clone, Copy)]
struct Node<'a> {
    kind: Kind<'a>,
    first: usize,
    len: usize,
}
#[derive(Clone, Copy)]
struct Edge<'a> {
    key: &'a str,
    node: usize,
    order: usize,
}
const EMPTY_EDGE: Edge<'static> = Edge {
    key: "",
    node: 0,
    order: 0,
};
pub(crate) struct Arena<'a> {
    nodes: Vec<Node<'a>>,
    edges: Vec<Edge<'a>>,
    _node_fee: OwnedSemaphorePermit,
    _edge_fee: OwnedSemaphorePermit,
}
#[derive(Clone, Copy)]
pub(crate) struct Value<'t, 'a> {
    tree: &'t Arena<'a>,
    node: usize,
}
#[derive(Clone, Copy)]
pub(crate) struct Dictionary<'t, 'a> {
    value: Value<'t, 'a>,
}
#[derive(Clone, Copy)]
pub(crate) struct Array<'t, 'a> {
    value: Value<'t, 'a>,
}

impl<'a> Arena<'a> {
    pub(crate) fn root(&self) -> Value<'_, 'a> {
        Value {
            tree: self,
            node: 0,
        }
    }
}
impl<'t, 'a> Value<'t, 'a> {
    pub(crate) fn as_str(self) -> Option<&'a str> {
        if let Kind::String(s) = self.tree.nodes[self.node].kind {
            Some(s)
        } else {
            None
        }
    }
    pub(crate) fn as_uint64(self) -> Option<u64> {
        if let Kind::Uint64(n) = self.tree.nodes[self.node].kind {
            Some(n)
        } else {
            None
        }
    }
    pub(crate) fn as_dict(self) -> Option<Dictionary<'t, 'a>> {
        if matches!(self.tree.nodes[self.node].kind, Kind::Dictionary) {
            Some(Dictionary { value: self })
        } else {
            None
        }
    }
    pub(crate) fn as_array(self) -> Option<Array<'t, 'a>> {
        if matches!(self.tree.nodes[self.node].kind, Kind::Array) {
            Some(Array { value: self })
        } else {
            None
        }
    }
    fn children(self) -> &'t [Edge<'a>] {
        let node = self.tree.nodes[self.node];
        &self.tree.edges[node.first..node.first + node.len]
    }
}
impl<'t, 'a> Dictionary<'t, 'a> {
    pub(crate) fn len(self) -> usize {
        self.value.children().len()
    }
    pub(crate) fn iter(self) -> impl Iterator<Item = (&'a str, Value<'t, 'a>)> + 't
    where
        'a: 't,
    {
        self.value.children().iter().map(move |edge| {
            (
                edge.key,
                Value {
                    tree: self.value.tree,
                    node: edge.node,
                },
            )
        })
    }
    pub(crate) fn get(self, key: &str) -> Option<Value<'t, 'a>> {
        self.value
            .children()
            .iter()
            .find(|edge| edge.key == key)
            .map(|edge| Value {
                tree: self.value.tree,
                node: edge.node,
            })
    }
}
impl<'t, 'a> Array<'t, 'a> {
    pub(crate) fn len(self) -> usize {
        self.value.children().len()
    }
    pub(crate) fn iter(self) -> impl Iterator<Item = Value<'t, 'a>> + 't
    where
        'a: 't,
    {
        self.value.children().iter().map(move |edge| Value {
            tree: self.value.tree,
            node: edge.node,
        })
    }
}

#[derive(Default, Clone, Copy)]
struct Counts {
    nodes: usize,
    edges: usize,
}
struct Parser<'a> {
    bytes: &'a [u8],
    counts: Counts,
}
impl<'a> Parser<'a> {
    fn take(&mut self, n: usize, reason: &'static str) -> Result<&'a [u8], XpcError> {
        if self.bytes.len() < n {
            return Err(XpcError::Tls(reason.into()));
        }
        let (head, tail) = self.bytes.split_at(n);
        self.bytes = tail;
        Ok(head)
    }
    fn u32(&mut self, reason: &'static str) -> Result<u32, XpcError> {
        Ok(u32::from_le_bytes(
            self.take(4, reason)?.try_into().expect("four bytes"),
        ))
    }
    fn key(&mut self) -> Result<&'a str, XpcError> {
        let nul = self
            .bytes
            .iter()
            .position(|&byte| byte == 0)
            .ok_or_else(|| XpcError::Tls("XPC: unterminated dictionary key".into()))?;
        let raw = self.take(nul, "XPC: dict key terminator truncated")?;
        self.take(1, "XPC: dict key terminator truncated")?;
        let total = nul
            .checked_add(1)
            .ok_or_else(|| XpcError::Tls("XPC dict key length overflow".into()))?;
        let padded = checked_align4("dict key", total)?;
        self.take(padded - total, "XPC: dict key padding truncated")?;
        std::str::from_utf8(raw).map_err(|error| {
            native_error(ErrorKind::InvalidData, "decode XPC dictionary key", error)
        })
    }
    fn parse(
        &mut self,
        depth: usize,
        sink: &mut Option<&mut Arena<'a>>,
    ) -> Result<usize, XpcError> {
        if depth > MAX_XPC_NESTING_DEPTH {
            return Err(XpcError::Tls(format!(
                "XPC: nesting exceeds {MAX_XPC_NESTING_DEPTH} levels"
            )));
        }
        let tag = self.u32("XPC: value too short")?;
        let index = self.counts.nodes;
        self.counts.nodes = index
            .checked_add(1)
            .ok_or_else(crate::device::existing_rsd::ExistingRsdProbeError::metadata_limit)?;
        if let Some(arena) = sink.as_deref_mut() {
            arena.nodes.push(Node {
                kind: Kind::Other,
                first: 0,
                len: 0,
            });
        }
        let mut first = 0;
        let mut count = 0;
        let kind = match tag {
            TYPE_NULL => Kind::Other,
            TYPE_BOOL => {
                self.take(4, "XPC: bool truncated")?;
                Kind::Other
            }
            TYPE_INT64 => {
                self.take(8, "XPC: i64 truncated")?;
                Kind::Other
            }
            TYPE_UINT64 => Kind::Uint64(u64::from_le_bytes(
                self.take(8, "XPC: u64 truncated")?
                    .try_into()
                    .expect("eight bytes"),
            )),
            TYPE_DOUBLE => {
                self.take(8, "XPC: f64 truncated")?;
                Kind::Other
            }
            TYPE_DATE => {
                self.take(8, "XPC: date truncated")?;
                Kind::Other
            }
            TYPE_UUID => {
                self.take(16, "XPC: uuid truncated")?;
                Kind::Other
            }
            TYPE_DATA | TYPE_STRING => {
                let (what, truncated, length_truncated, conversion) = if tag == TYPE_DATA {
                    (
                        "data",
                        "XPC: data truncated",
                        "XPC: data length truncated",
                        "convert XPC data length",
                    )
                } else {
                    (
                        "string",
                        "XPC: string truncated",
                        "XPC: string length truncated",
                        "convert XPC string length",
                    )
                };
                let declared = self.u32(length_truncated)?;
                let len = usize::try_from(declared)
                    .map_err(|error| native_error(ErrorKind::InvalidData, conversion, error))?;
                let padded = checked_align4(what, len)?;
                let raw = &self.take(padded, truncated)?[..len];
                if tag == TYPE_STRING {
                    let end = raw.iter().position(|&byte| byte == 0).unwrap_or(raw.len());
                    Kind::String(std::str::from_utf8(&raw[..end]).map_err(|error| {
                        native_error(ErrorKind::InvalidData, "decode XPC string", error)
                    })?)
                } else {
                    Kind::Other
                }
            }
            TYPE_ARRAY | TYPE_DICTIONARY | TYPE_FILE_TRANSFER => {
                let dictionary = tag == TYPE_DICTIONARY;
                count = if tag == TYPE_FILE_TRANSFER {
                    self.take(8, "XPC: file transfer truncated")?;
                    1
                } else {
                    let reason = if dictionary {
                        "XPC: dict header truncated"
                    } else {
                        "XPC: array header truncated"
                    };
                    // The original decoder requires both length and count before
                    // reading either, and intentionally ignores container lengths.
                    let raw = self.take(8, reason)?;
                    let declared = u32::from_le_bytes(raw[4..].try_into().expect("four bytes"));
                    let count = usize::try_from(declared).map_err(|error| {
                        native_error(
                            ErrorKind::InvalidData,
                            if dictionary {
                                "convert XPC dictionary count"
                            } else {
                                "convert XPC array count"
                            },
                            error,
                        )
                    })?;
                    if count > MAX_XPC_COLLECTION_SIZE {
                        return Err(XpcError::Tls(format!("XPC collection too large: {count}")));
                    }
                    count
                };
                first = self.counts.edges;
                self.counts.edges = first.checked_add(count).ok_or_else(
                    crate::device::existing_rsd::ExistingRsdProbeError::metadata_limit,
                )?;
                if let Some(arena) = sink.as_deref_mut() {
                    arena.edges.resize(self.counts.edges, EMPTY_EDGE);
                }
                for order in 0..count {
                    let key = if dictionary { self.key()? } else { "" };
                    let child = self.parse(depth + 1, sink)?;
                    if let Some(arena) = sink.as_deref_mut() {
                        arena.edges[first + order] = Edge {
                            key,
                            node: child,
                            order,
                        };
                    }
                }
                if dictionary {
                    if let Some(arena) = sink.as_deref_mut() {
                        let edges = &mut arena.edges[first..first + count];
                        // Match IndexMap duplicate semantics: first insertion
                        // order and last value win. Sorting needs no heap scratch.
                        edges.sort_unstable_by(|a, b| a.key.cmp(b.key).then(a.order.cmp(&b.order)));
                        let mut read = 0;
                        let mut write = 0;
                        while read < count {
                            let begin = read;
                            while read + 1 < count && edges[read + 1].key == edges[begin].key {
                                read += 1;
                            }
                            let mut selected = edges[read];
                            selected.order = edges[begin].order;
                            edges[write] = selected;
                            write += 1;
                            read += 1;
                        }
                        count = write;
                        edges[..count].sort_unstable_by_key(|edge| edge.order);
                    }
                    Kind::Dictionary
                } else if tag == TYPE_ARRAY {
                    Kind::Array
                } else {
                    Kind::FileTransfer
                }
            }
            other => return Err(XpcError::Tls(format!("XPC: unknown type 0x{other:08X}"))),
        };
        if let Some(arena) = sink.as_deref_mut() {
            arena.nodes[index] = Node {
                kind,
                first,
                len: count,
            };
        }
        Ok(index)
    }
}

pub(crate) fn decode<'a>(
    header: &[u8],
    body: &'a [u8],
    bank: &Arc<Semaphore>,
) -> Result<Option<Arena<'a>>, XpcError> {
    if header.len() < 4 {
        return Err(XpcError::Tls("XPC: buffer too short for magic".into()));
    }
    let magic = u32::from_le_bytes(header[..4].try_into().expect("four bytes"));
    if magic != WRAPPER_MAGIC {
        return Err(XpcError::Tls(format!("XPC: bad magic 0x{magic:08X}")));
    }
    if header.len() < 24 {
        return Err(XpcError::Tls("XPC: buffer too short for header".into()));
    }
    let flags = u32::from_le_bytes(header[4..8].try_into().expect("four bytes"));
    let declared = u64::from_le_bytes(header[8..16].try_into().expect("eight bytes"));
    let len =
        checked_xpc_body_len(declared, xpc_body_limit_for_flags(flags)).map_err(XpcError::Io)?;
    if len == 0 {
        return Ok(None);
    }
    if body.len() < len {
        return Err(XpcError::Tls(format!(
            "XPC: body truncated: header declares {declared} bytes, {} available",
            body.len()
        )));
    }
    if len < 8 {
        return Err(XpcError::Tls(format!(
            "XPC: body header truncated: {len} bytes, need at least 8"
        )));
    }
    let magic = u32::from_le_bytes(body[..4].try_into().expect("four bytes"));
    if magic != OBJECT_MAGIC {
        return Err(XpcError::Tls(format!(
            "XPC: bad object magic 0x{magic:08X}"
        )));
    }
    let version = u32::from_le_bytes(body[4..8].try_into().expect("four bytes"));
    if version != BODY_VERSION {
        return Err(XpcError::Tls(format!(
            "XPC: bad body version 0x{version:08X}"
        )));
    }
    let mut counter = Parser {
        bytes: &body[8..len],
        counts: Counts::default(),
    };
    counter.parse(0, &mut None)?;
    let (node_fee, nodes) =
        crate::xpc::raw_storage::exact_vec(bank, counter.counts.nodes).map_err(XpcError::Io)?;
    let (edge_fee, edges) =
        crate::xpc::raw_storage::exact_vec(bank, counter.counts.edges).map_err(XpcError::Io)?;
    let mut arena = Arena {
        nodes,
        edges,
        _node_fee: node_fee,
        _edge_fee: edge_fee,
    };
    let mut builder = Parser {
        bytes: &body[8..len],
        counts: Counts::default(),
    };
    builder.parse(0, &mut Some(&mut arena))?;
    Ok(Some(arena))
}

#[cfg(test)]
#[path = "borrowed/tests.rs"]
mod tests;
